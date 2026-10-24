use super::*;
use crate::scheduled::{Entry, Status, Store};
use chrono::{Local, TimeZone, Timelike};

pub(super) struct State {
    store: Option<Store>,
    previous: i64,
    ready_account: Option<String>,
    reported: bool,
    closed: bool,
    button: Option<gtk::MenuButton>,
    ui: Option<ListUi>,
    editors: Vec<(String, glib::WeakRef<gtk::Window>)>,
}

#[derive(Clone)]
struct ListUi {
    window: gtk::Window,
    list: gtk::ListBox,
    empty: gtk::Label,
    account: String,
}

impl State {
    pub(super) fn load(data_dir: &std::path::Path) -> Self {
        let now = now_ms();
        let store = match Store::load(data_dir, now) {
            Ok(store) => Some(store),
            Err(_) => {
                tracing::error!("予約ファイルを読み込めないため、予約送信を停止しました");
                None
            }
        };
        Self {
            store,
            previous: now,
            ready_account: None,
            reported: false,
            closed: false,
            button: None,
            ui: None,
            editors: Vec::new(),
        }
    }
}

fn healthy(state: &AppState) -> bool {
    state
        .scheduled
        .borrow()
        .store
        .as_ref()
        .is_some_and(Store::healthy)
}

fn ready_account(state: &AppState) -> Option<String> {
    (*state.session_ready.borrow()
        && !*state.restarting.borrow()
        && !resume_sync::in_progress(state)
        && gio::NetworkMonitor::default().is_network_available())
    .then(|| state.self_mid.borrow().clone())
    .flatten()
}

pub(super) fn wire(state: &AppState) {
    let schedule = gio::SimpleAction::new("schedule-message", None);
    let s = state.clone();
    schedule.connect_activate(move |_, _| open_new(&s));
    state.window.add_action(&schedule);
    let list = gio::SimpleAction::new("scheduled-messages", None);
    let s = state.clone();
    list.connect_activate(move |_, _| open_list(&s));
    state.window.add_action(&list);
    let button = gtk::MenuButton::builder()
        .icon_name("appointment-new-symbolic")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    state
        .composer_row
        .insert_child_after(&button, Some(&state.send_btn));
    state.scheduled.borrow_mut().button = Some(button);
    apply_language(state);

    let s = state.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        if s.scheduled.borrow().closed {
            return glib::ControlFlow::Break;
        }
        tick(&s);
        glib::ControlFlow::Continue
    });
    let s = state.clone();
    state.app.connect_shutdown(move |_| {
        s.scheduled.borrow_mut().closed = true;
        close_list(&s);
    });
}

pub(super) fn apply_language(state: &AppState) {
    if let Some(button) = &state.scheduled.borrow().button {
        let popover = gtk::Popover::builder()
            .position(gtk::PositionType::Top)
            .build();
        let items = gtk::Box::new(gtk::Orientation::Vertical, 4);
        for (key, action) in [
            ("schedule_message", "win.schedule-message"),
            ("scheduled_messages", "win.scheduled-messages"),
        ] {
            let item = gtk::Button::builder()
                .label(crate::i18n::t(key))
                .action_name(action)
                .css_classes(["flat"])
                .build();
            let weak = popover.downgrade();
            item.connect_clicked(move |_| {
                if let Some(popover) = weak.upgrade() {
                    popover.popdown();
                }
            });
            items.append(&item);
        }
        popover.set_child(Some(&items));
        button.set_popover(Some(&popover));
        button.set_tooltip_text(Some(&crate::i18n::t("schedule_actions")));
        button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
            "schedule_actions",
        ))]);
    }
}

fn tick(state: &AppState) {
    let now = now_ms();
    let ready = ready_account(state);
    let mut scheduler = state.scheduled.borrow_mut();
    let previous = scheduler.previous;
    // 前回の確認時点で接続できていなかった場合も、後から拾って送らない。
    let account = if scheduler.ready_account == ready {
        ready.as_deref()
    } else {
        None
    };
    scheduler.previous = now;
    scheduler.ready_account = ready.clone();
    let Some(store) = scheduler.store.as_mut() else {
        drop(scheduler);
        report_storage_error(state);
        return;
    };
    if !store.healthy() {
        drop(scheduler);
        report_storage_error(state);
        return;
    }
    let before: Vec<_> = store.entries().iter().map(|entry| entry.status).collect();
    let claimed = store.tick(previous, now, account);
    let changed = before
        != store
            .entries()
            .iter()
            .map(|entry| entry.status)
            .collect::<Vec<_>>();
    drop(scheduler);
    match claimed {
        Ok(entries) => {
            for entry in entries {
                dispatch(state, entry);
            }
        }
        Err(_) => report_storage_error(state),
    }
    let wrong_account = state.scheduled.borrow().ui.as_ref().is_some_and(|ui| {
        !*state.session_ready.borrow()
            || state.self_mid.borrow().as_deref() != Some(ui.account.as_str())
    });
    if wrong_account {
        close_list(state);
    } else if changed {
        render(state);
    }
    let account = state.self_mid.borrow().clone();
    let logged_in = *state.session_ready.borrow();
    let mut close = Vec::new();
    state
        .scheduled
        .borrow_mut()
        .editors
        .retain(|(owner, weak)| {
            let Some(window) = weak.upgrade() else {
                return false;
            };
            if !logged_in || account.as_deref() != Some(owner.as_str()) {
                close.push(window);
                false
            } else {
                true
            }
        });
    for window in close {
        window.close();
    }
}

// 通信プロセスやログイン先を切り替えると、送信済みか確認できない要求が残る。
// 要求を破棄する前に結果不明へ移し、再送や「送信中」の残留を防ぐ。
pub(super) fn interrupt(state: &AppState) {
    let requests: Vec<_> = state
        .pending
        .borrow()
        .iter()
        .filter_map(|(id, pending)| {
            if let Pending::ScheduledSend {
                reservation_id,
                account,
                ..
            } = pending
            {
                Some((*id, *reservation_id, account.clone()))
            } else {
                None
            }
        })
        .collect();
    for (request, id, account) in requests {
        state.pending.borrow_mut().remove(&request);
        finish(state, &account, id, Status::Unknown);
    }
    state.scheduled.borrow_mut().ready_account = None;
    close_list(state);
    let editors = std::mem::take(&mut state.scheduled.borrow_mut().editors);
    for (_, weak) in editors {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    }
}

fn dispatch(state: &AppState, entry: Entry) {
    let status = if now_ms() > entry.send_at + crate::scheduled::DISPATCH_WINDOW_MS
        || ready_account(state).as_deref() != Some(entry.account.as_str())
    {
        Some(Status::Expired)
    } else if contact_blocks::blocked(state, &entry.chat_mid) != Some(false)
        || state.pending.borrow().values().any(|pending| {
            matches!(pending,
            Pending::ContactBlocks { change: Some((mid, _)), .. } if mid == &entry.chat_mid)
        })
        || entry.quote.as_ref().is_some_and(|quote| {
            state
                .unsent_ids
                .borrow()
                .contains(&(entry.account.clone(), quote.message_id.clone()))
        })
    {
        Some(Status::Failed)
    } else {
        None
    };
    if let Some(status) = status {
        finish(state, &entry.account, entry.id, status);
        return;
    }
    match state.sidecar.send_scheduled(&entry) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::ScheduledSend {
                    reservation_id: entry.id,
                    account: entry.account,
                    chat_mid: entry.chat_mid,
                },
            );
        }
        Err(_) => finish(state, &entry.account, entry.id, Status::Unknown),
    }
}

fn report_storage_error(state: &AppState) {
    if !state.scheduled.borrow().reported {
        state.scheduled.borrow_mut().reported = true;
        toast(state, &crate::i18n::t("schedule_storage_error"));
        render(state);
    }
}

fn finish(state: &AppState, account: &str, id: u64, status: Status) {
    let result = state
        .scheduled
        .borrow_mut()
        .store
        .as_mut()
        .map(|store| store.finish(account, id, status));
    if !matches!(result, Some(Ok(()))) {
        report_storage_error(state);
    }
    render(state);
}

pub(super) fn response(
    state: &AppState,
    id: u64,
    account: &str,
    chat_mid: &str,
    ok: bool,
    result: &serde_json::Value,
    error: Option<&str>,
) {
    let message = if ok {
        result
            .get("message")
            .cloned()
            .and_then(|value| serde_json::from_value::<MessageInfo>(value).ok())
            .filter(|message| !message.id.is_empty())
    } else {
        None
    };
    let status = if message.is_some() {
        Status::Sent
    } else {
        match error {
            Some("scheduled_expired" | "scheduled_unavailable") => Status::Expired,
            Some("contact_blocked_send" | "scheduled_invalid" | "scheduled_limit") => {
                Status::Failed
            }
            _ => Status::Unknown,
        }
    };
    finish(state, account, id, status);
    if state.self_mid.borrow().as_deref() != Some(account) || !*state.session_ready.borrow() {
        return;
    }
    if let Some(message) = message {
        if state.current_chat.borrow().as_deref() == Some(chat_mid) {
            set_msg_state(state, "list", None);
            append_message(state, &message, true);
            if *state.stick_bottom.borrow() {
                pin_messages_to_latest(state);
            }
        }
        toast(state, &crate::i18n::t("schedule_sent_notice"));
    } else {
        toast(state, &crate::i18n::t("schedule_failed_notice"));
    }
}

fn open_new(state: &AppState) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if contact_blocks::blocked(state, &chat_mid) == Some(true) {
        toast(state, &crate::i18n::t("contact_blocked_send"));
        return;
    }
    if !healthy(state) {
        report_storage_error(state);
        return;
    }
    if !writing_ui::flush(state) {
        return;
    }
    let chat_name = state
        .chats
        .borrow()
        .iter()
        .find(|chat| chat.mid == chat_mid)
        .map(|chat| chat.name.clone())
        .unwrap_or_else(|| chat_mid.clone());
    let quote = state
        .reply_target
        .borrow()
        .as_ref()
        .filter(|target| target.chat_mid == chat_mid)
        .map(|target| target.quote.clone());
    open_editor(
        state,
        Entry {
            id: 0,
            account,
            chat_mid,
            chat_name,
            text: state.composer.text().to_string(),
            quote,
            send_at: (Local::now() + chrono::Duration::minutes(5))
                .with_second(0)
                .unwrap()
                .timestamp_millis(),
            status: Status::Queued,
        },
    );
}

fn open_editor(state: &AppState, entry: Entry) {
    let window = gtk::Window::builder()
        .title(crate::i18n::t("schedule_message"))
        .transient_for(&state.window)
        .modal(true)
        .default_width(480)
        .build();
    state
        .scheduled
        .borrow_mut()
        .editors
        .push((entry.account.clone(), window.downgrade()));
    let header = gtk::HeaderBar::new();
    let save = gtk::Button::with_label(&crate::i18n::t("schedule_save"));
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    window.set_titlebar(Some(&header));
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    let recipient = gtk::Label::builder()
        .label(crate::i18n::tf(
            "schedule_recipient",
            &[("name", &entry.chat_name)],
        ))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["heading"])
        .build();
    body.append(&recipient);
    let help = gtk::Label::builder()
        .label(crate::i18n::t("schedule_help"))
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(50)
        .css_classes(["dim-label"])
        .build();
    body.append(&help);
    let time = Local
        .timestamp_millis_opt(entry.send_at)
        .single()
        .unwrap_or_else(Local::now);
    let date = gtk::Entry::builder()
        .text(time.format("%Y-%m-%d").to_string())
        .placeholder_text("YYYY-MM-DD")
        .hexpand(true)
        .build();
    date.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "schedule_date",
    ))]);
    date.set_tooltip_text(Some(&crate::i18n::t("schedule_date")));
    let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    hour.set_value(f64::from(time.hour()));
    hour.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "schedule_hour",
    ))]);
    let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    minute.set_value(f64::from(time.minute()));
    minute.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "schedule_minute",
    ))]);
    let when = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    when.append(&date);
    when.append(&hour);
    when.append(&gtk::Label::new(Some(":")));
    when.append(&minute);
    body.append(&when);
    let timezone = gtk::Label::builder()
        .label(crate::i18n::tf(
            "schedule_timezone",
            &[("zone", &Local::now().format("UTC%:z").to_string())],
        ))
        .xalign(0.0)
        .css_classes(["caption", "dim-label"])
        .build();
    body.append(&timezone);
    let quote = gtk::CheckButton::with_label(&crate::i18n::t("schedule_include_quote"));
    quote.set_active(entry.quote.is_some());
    quote.set_visible(entry.quote.is_some());
    body.append(&quote);
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    text.buffer().set_text(&entry.text);
    text.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "schedule_text",
    ))]);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(150)
        .max_content_height(300)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&text)
        .css_classes(["card"])
        .build();
    body.append(&scroll);
    let error = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["error"])
        .build();
    body.append(&error);
    window.set_child(Some(&body));
    let s = state.clone();
    let weak = window.downgrade();
    save.connect_clicked(move |_| {
        let buffer = text.buffer();
        let content = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        let now = now_ms();
        let due = crate::scheduled::parse_local_date(
            date.text().as_str(),
            hour.value_as_int() as u32,
            minute.value_as_int() as u32,
        );
        let Ok(send_at) = due else {
            error.set_text(&crate::i18n::t("schedule_invalid"));
            return;
        };
        if send_at <= now || content.trim().is_empty() || content.chars().count() > 10_000 {
            error.set_text(&crate::i18n::t("schedule_invalid"));
            return;
        }
        if !*s.session_ready.borrow()
            || s.self_mid.borrow().as_deref() != Some(entry.account.as_str())
        {
            error.set_text(&crate::i18n::t("schedule_account_changed"));
            return;
        }
        let mut next = entry.clone();
        next.text = content;
        next.send_at = send_at;
        if !quote.is_active() {
            next.quote = None;
        }
        let result = s
            .scheduled
            .borrow_mut()
            .store
            .as_mut()
            .map(|store| store.save_entry(next, now));
        if !matches!(result, Some(Ok(_))) {
            error.set_text(&crate::i18n::t("schedule_save_failed"));
            return;
        }
        // ダイアログ表示中に書き換えた入力欄や、他のトークの文章を消さない。
        if entry.id == 0
            && s.current_chat.borrow().as_deref() == Some(entry.chat_mid.as_str())
            && s.composer.text().as_str() == entry.text
        {
            s.composer.set_text("");
            replies::cancel(&s);
        }
        render(&s);
        toast(&s, &crate::i18n::t("schedule_saved"));
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    window.present();
}

fn close_list(state: &AppState) {
    let ui = state.scheduled.borrow_mut().ui.take();
    if let Some(ui) = ui {
        ui.window.close();
    }
}

fn open_list(state: &AppState) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    if let Some(ui) = &state.scheduled.borrow().ui {
        ui.window.present();
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let window = gtk::Window::builder()
        .title(crate::i18n::t("scheduled_messages"))
        .transient_for(&state.window)
        .default_width(570)
        .default_height(500)
        .build();
    window.set_titlebar(Some(&gtk::HeaderBar::new()));
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    let empty = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    body.append(&empty);
    body.append(&scroll);
    window.set_child(Some(&body));
    let weak = state.scheduled.clone();
    window.connect_close_request(move |_| {
        weak.borrow_mut().ui = None;
        glib::Propagation::Proceed
    });
    state.scheduled.borrow_mut().ui = Some(ListUi {
        window: window.clone(),
        list,
        empty,
        account,
    });
    render(state);
    window.present();
}

fn status_key(status: Status) -> &'static str {
    match status {
        Status::Queued => "schedule_queued",
        Status::Dispatching => "schedule_dispatching",
        Status::Sent => "schedule_sent",
        Status::Failed => "schedule_failed",
        Status::Unknown => "schedule_unknown",
        Status::Expired => "schedule_expired",
        Status::Cancelled => "schedule_cancelled",
    }
}

fn render(state: &AppState) {
    let scheduler = state.scheduled.borrow();
    let Some(ui) = scheduler.ui.clone() else {
        return;
    };
    let healthy = scheduler.store.as_ref().is_some_and(Store::healthy);
    let mut entries: Vec<_> = scheduler
        .store
        .as_ref()
        .map(|store| {
            store
                .entries()
                .iter()
                .filter(|entry| entry.account == ui.account)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    drop(scheduler);
    entries.sort_by_key(|entry| (entry.status != Status::Queued, entry.send_at, entry.id));
    while let Some(child) = ui.list.first_child() {
        ui.list.remove(&child);
    }
    ui.empty.set_text(&crate::i18n::t(if !healthy {
        "schedule_storage_error"
    } else if entries.is_empty() {
        "schedule_empty"
    } else {
        "schedule_list_help"
    }));
    for entry in entries {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let date = Local
            .timestamp_millis_opt(entry.send_at)
            .single()
            .map(|time| time.format("%Y/%m/%d %H:%M").to_string())
            .unwrap_or_default();
        let title = gtk::Label::builder()
            .label(format!("{} · {date}", entry.chat_name))
            .xalign(0.0)
            .wrap(true)
            .css_classes(["heading"])
            .build();
        let status = gtk::Label::builder()
            .label(crate::i18n::t(status_key(entry.status)))
            .xalign(0.0)
            .wrap(true)
            .css_classes(["caption"])
            .build();
        let text = gtk::Label::builder()
            .label(&entry.text)
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .lines(4)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        row.append(&title);
        row.append(&status);
        row.append(&text);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if entry.status == Status::Queued {
            let edit = gtk::Button::with_label(&crate::i18n::t("schedule_edit"));
            edit.set_sensitive(healthy);
            let s = state.clone();
            let edited = entry.clone();
            edit.connect_clicked(move |_| {
                if s.self_mid.borrow().as_deref() == Some(edited.account.as_str())
                    && *s.session_ready.borrow()
                {
                    open_editor(&s, edited.clone());
                }
            });
            actions.append(&edit);
        }
        if entry.status != Status::Dispatching {
            let remove =
                gtk::Button::with_label(&crate::i18n::t(if entry.status == Status::Queued {
                    "schedule_cancel"
                } else {
                    "schedule_remove"
                }));
            remove.set_sensitive(healthy);
            let s = state.clone();
            remove.connect_clicked(move |_| {
                if s.self_mid.borrow().as_deref() != Some(entry.account.as_str())
                    || !*s.session_ready.borrow()
                {
                    return;
                }
                let result = s
                    .scheduled
                    .borrow_mut()
                    .store
                    .as_mut()
                    .map(|store| store.cancel_or_remove(&entry.account, entry.id));
                if !matches!(result, Some(Ok(()))) {
                    toast(&s, &crate::i18n::t("schedule_save_failed"));
                }
                render(&s);
            });
            actions.append(&remove);
        }
        row.append(&actions);
        ui.list.append(&row);
    }
}
