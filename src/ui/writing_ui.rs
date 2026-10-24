use super::*;
use crate::writing::{Delivery, Draft, Store};

pub(super) struct State {
    store: Option<Store>,
    owner: Option<(String, String)>,
    holding: bool,
    generation: u64,
    reported: bool,
    button: Option<gtk::Button>,
    ui: Option<RecoveryUi>,
}

#[derive(Clone)]
struct RecoveryUi {
    window: gtk::Window,
    list: gtk::ListBox,
    account: String,
}

impl State {
    pub(super) fn load(directory: &std::path::Path) -> Self {
        Self {
            store: Store::load(directory).ok(),
            owner: None,
            holding: false,
            generation: 0,
            reported: false,
            button: None,
            ui: None,
        }
    }
}

pub(super) fn wire(state: &AppState) {
    let button = gtk::Button::builder()
        .visible(false)
        .css_classes(["flat"])
        .build();
    state
        .composer_row
        .insert_child_after(&button, Some(&state.send_btn));
    let s = state.clone();
    button.connect_clicked(move |_| open_recovery(&s));
    state.writing.borrow_mut().button = Some(button);
    let s = state.clone();
    state
        .composer
        .buffer()
        .connect_changed(move |_| changed(&s));
    let s = state.clone();
    state.app.connect_shutdown(move |_| {
        flush(&s);
        close_recovery(&s);
    });
    refresh(state);
}

pub(super) fn changed(state: &AppState) {
    let generation = {
        let mut writing = state.writing.borrow_mut();
        if writing.holding || writing.owner.is_none() {
            return;
        }
        writing.generation = writing.generation.wrapping_add(1);
        writing.generation
    };
    let s = state.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
        if s.writing.borrow().generation == generation {
            flush(&s);
        }
    });
}

fn snapshot(state: &AppState) -> Option<Draft> {
    let (account, chat_mid) = state.writing.borrow().owner.clone()?;
    let quote = state
        .reply_target
        .borrow()
        .as_ref()
        .filter(|target| target.chat_mid == chat_mid)
        .map(|target| target.quote.clone());
    Some(Draft {
        account,
        chat_mid,
        text: state.composer.text().to_string(),
        quote,
    })
}

pub(super) fn flush(state: &AppState) -> bool {
    let Some(draft) = snapshot(state) else {
        return true;
    };
    let result = state
        .writing
        .borrow_mut()
        .store
        .as_mut()
        .map(|store| store.save_draft(draft));
    if matches!(result, Some(Ok(()))) {
        refresh(state);
        true
    } else {
        storage_error(state);
        false
    }
}

fn storage_error(state: &AppState) {
    if !state.writing.borrow().reported {
        state.writing.borrow_mut().reported = true;
        toast(state, &crate::i18n::t("writing_save_error"));
    }
}

pub(super) fn leave_chat(state: &AppState) -> bool {
    if !flush(state) {
        return false;
    }
    {
        let mut writing = state.writing.borrow_mut();
        writing.holding = true;
        writing.owner = None;
        writing.generation = writing.generation.wrapping_add(1);
    }
    state.composer.set_text("");
    replies::cancel(state);
    state.writing.borrow_mut().holding = false;
    true
}

pub(super) fn enter_chat(state: &AppState, chat_mid: &str) {
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let draft = state
        .writing
        .borrow()
        .store
        .as_ref()
        .and_then(|store| store.draft(&account, chat_mid));
    {
        let mut writing = state.writing.borrow_mut();
        writing.holding = true;
        writing.owner = Some((account.clone(), chat_mid.into()));
    }
    state
        .composer
        .set_text(draft.as_ref().map_or("", |draft| draft.text.as_str()));
    *state.reply_target.borrow_mut() = draft
        .and_then(|draft| draft.quote)
        .filter(|quote| {
            !state
                .unsent_ids
                .borrow()
                .contains(&(account, quote.message_id.clone()))
        })
        .map(|quote| replies::ReplyTarget {
            chat_mid: chat_mid.into(),
            quote,
        });
    replies::refresh(state);
    state.writing.borrow_mut().holding = false;
    refresh(state);
}

pub(super) fn session(state: &AppState, account: &str) {
    let different = state
        .writing
        .borrow()
        .owner
        .as_ref()
        .is_some_and(|(owner, _)| owner != account);
    if different {
        // 保存失敗時は以前の本文を画面に残し、新しい宛先へ使い回さない。
        if !leave_chat(state) {
            state.composer.set_sensitive(false);
        }
        *state.current_chat.borrow_mut() = None;
        clear_messages(state);
    }
    let different_ui = state
        .writing
        .borrow()
        .ui
        .as_ref()
        .is_some_and(|ui| ui.account != account);
    if different_ui {
        close_recovery(state);
    }
}

pub(super) fn interrupt(state: &AppState) {
    flush(state);
    let pending: Vec<_> = state
        .pending
        .borrow()
        .values()
        .filter_map(|pending| {
            if let Pending::Send {
                saved_text: Some(saved),
                ..
            } = pending
            {
                Some(saved.clone())
            } else {
                None
            }
        })
        .collect();
    for (account, id) in pending {
        complete(state, &account, id, false, None, None);
    }
    close_recovery(state);
}

pub(super) fn prepare_send(
    state: &AppState,
    chat_mid: &str,
    text: &str,
    quote: Option<crate::protocol::ReplyInfo>,
) -> Option<(String, u64)> {
    if !state
        .writing
        .borrow()
        .store
        .as_ref()
        .is_some_and(Store::healthy)
    {
        storage_error(state);
        return None;
    }
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return None;
    }
    let account = state.self_mid.borrow().clone()?;
    if state.writing.borrow().owner.as_ref() != Some(&(account.clone(), chat_mid.into())) {
        storage_error(state);
        return None;
    }
    let chat_name = state
        .chats
        .borrow()
        .iter()
        .find(|chat| chat.mid == chat_mid)
        .map(|chat| chat.name.clone())
        .unwrap_or_else(|| chat_mid.into());
    let draft = Draft {
        account: account.clone(),
        chat_mid: chat_mid.into(),
        text: text.into(),
        quote,
    };
    let result = state
        .writing
        .borrow_mut()
        .store
        .as_mut()
        .map(|store| store.start_send(draft, chat_name));
    match result {
        Some(Ok(id)) => Some((account, id)),
        _ => {
            storage_error(state);
            None
        }
    }
}

pub(super) fn complete(
    state: &AppState,
    account: &str,
    id: u64,
    success: bool,
    error: Option<&str>,
    placeholder: Option<&str>,
) {
    let failure = if success {
        None
    } else {
        Some(match error {
            Some(
                "contact_blocked_send" | "empty_message" | "invalid_reply_target" | "not_logged_in",
            ) => Delivery::Failed,
            _ => Delivery::Unknown,
        })
    };
    let result = state
        .writing
        .borrow_mut()
        .store
        .as_mut()
        .map(|store| store.finish_send(account, id, failure));
    if !matches!(result, Some(Ok(()))) {
        storage_error(state);
    }
    if state.self_mid.borrow().as_deref() == Some(account) {
        if let Some(status) = failure && let Some(placeholder) = placeholder {
            mark_failed_row(state, placeholder, status);
        }
        refresh(state);
    }
}

fn mark_failed_row(state: &AppState, id: &str, status: Delivery) {
    let row = state.pending_rows.borrow().get(id).cloned();
    let Some(row) = row else {
        return;
    };
    fn clear_pending(widget: &gtk::Widget) {
        for class in [
            "line-msg-pending-row",
            "line-msg-pending",
            "line-bubble-pending",
        ] {
            widget.remove_css_class(class);
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            clear_pending(&widget);
        }
    }
    clear_pending(row.upcast_ref());
    if let Some(label) = state.receipt_slots.borrow_mut().remove(id) {
        label.set_text(&crate::i18n::t(status_key(status)));
    }
    let recover = gtk::Button::builder()
        .label(crate::i18n::t("writing_recover"))
        .halign(gtk::Align::End)
        .css_classes(["flat", "caption"])
        .build();
    let s = state.clone();
    recover.connect_clicked(move |_| open_recovery(&s));
    row.append(&recover);
}

fn status_key(status: Delivery) -> &'static str {
    match status {
        Delivery::Sending => "writing_sending",
        Delivery::Failed => "writing_failed",
        Delivery::Unknown => "writing_unknown",
    }
}

pub(super) fn refresh(state: &AppState) {
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let writing = state.writing.borrow();
    let entries = writing
        .store
        .as_ref()
        .map(|store| store.outgoing(&account))
        .unwrap_or_default();
    if let Some(button) = &writing.button {
        let n = entries
            .iter()
            .filter(|item| item.status != Delivery::Sending)
            .count();
        button.set_visible(n > 0);
        button.set_label(&crate::i18n::tf(
            "writing_pending_count",
            &[("n", &n.to_string())],
        ));
    }
    let drafts: Vec<_> = state
        .chats
        .borrow()
        .iter()
        .map(|chat| {
            let draft = writing
                .store
                .as_ref()
                .and_then(|store| store.draft(&account, &chat.mid));
            let preview = draft
                .map(|draft| {
                    crate::i18n::tf(
                        "writing_draft_preview",
                        &[(
                            "text",
                            &draft
                                .text
                                .lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(70)
                                .collect::<String>(),
                        )],
                    )
                })
                .unwrap_or_else(|| {
                    if chat.preview.is_empty() {
                        crate::i18n::t(if chat.last_activity > 0 {
                            "loading_last"
                        } else {
                            "no_recent"
                        })
                    } else {
                        chat.preview.clone()
                    }
                });
            (chat.mid.clone(), preview)
        })
        .collect();
    let ui = writing.ui.clone();
    drop(writing);
    for (mid, text) in drafts {
        if let Some(label) = state.chat_previews.borrow().get(&mid) {
            label.set_text(&text);
        }
    }
    let Some(ui) = ui else {
        return;
    };
    if ui.account != account {
        close_recovery(state);
        return;
    }
    while let Some(child) = ui.list.first_child() {
        ui.list.remove(&child);
    }
    for item in entries {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        row.append(
            &gtk::Label::builder()
                .label(format!(
                    "{} · {}",
                    item.chat_name,
                    crate::i18n::t(status_key(item.status))
                ))
                .xalign(0.0)
                .wrap(true)
                .css_classes(["heading"])
                .build(),
        );
        row.append(
            &gtk::Label::builder()
                .label(&item.draft.text)
                .xalign(0.0)
                .wrap(true)
                .selectable(true)
                .build(),
        );
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let copy = gtk::Button::with_label(&crate::i18n::t("writing_copy"));
        let clipboard = state.window.clipboard();
        let text = item.draft.text.clone();
        copy.connect_clicked(move |_| clipboard.set_text(&text));
        actions.append(&copy);
        if item.status != Delivery::Sending {
            let edit = gtk::Button::with_label(&crate::i18n::t("writing_restore"));
            let s = state.clone();
            let owner = account.clone();
            let id = item.id;
            edit.connect_clicked(move |_| restore(&s, &owner, id));
            actions.append(&edit);
            let remove = gtk::Button::with_label(&crate::i18n::t("writing_remove"));
            let s = state.clone();
            let owner = account.clone();
            remove.connect_clicked(move |_| {
                if s.self_mid.borrow().as_deref() != Some(owner.as_str()) {
                    return;
                }
                let result = s
                    .writing
                    .borrow_mut()
                    .store
                    .as_mut()
                    .map(|store| store.remove(&owner, id));
                if !matches!(result, Some(Ok(()))) {
                    storage_error(&s);
                }
                refresh(&s);
            });
            actions.append(&remove);
        }
        row.append(&actions);
        ui.list.append(&row);
    }
}

fn restore(state: &AppState, account: &str, id: u64) {
    if !*state.session_ready.borrow()
        || state.self_mid.borrow().as_deref() != Some(account)
        || !flush(state)
    {
        return;
    }
    let item = state.writing.borrow().store.as_ref().and_then(|store| {
        store
            .outgoing(account)
            .into_iter()
            .find(|item| item.id == id)
    });
    let Some(item) = item else {
        return;
    };
    let chat = state
        .chats
        .borrow()
        .iter()
        .find(|chat| chat.mid == item.draft.chat_mid)
        .cloned();
    let Some(chat) = chat else {
        toast(state, &crate::i18n::t("writing_missing_chat"));
        return;
    };
    if hidden_chats::is_hidden(state, &chat.mid) {
        toast(state, &crate::i18n::t("writing_missing_chat"));
        return;
    }
    let result = state
        .writing
        .borrow_mut()
        .store
        .as_mut()
        .map(|store| store.restore(account, id));
    let Some(Ok(_)) = result else {
        toast(state, &crate::i18n::t("writing_conflict"));
        return;
    };
    if state.current_chat.borrow().as_deref() == Some(chat.mid.as_str()) {
        enter_chat(state, &chat.mid);
    } else {
        open_chat(state, &chat);
    }
    close_recovery(state);
    state.composer.grab_focus();
}

fn close_recovery(state: &AppState) {
    let ui = state.writing.borrow_mut().ui.take();
    if let Some(ui) = ui {
        ui.window.close();
    }
}

fn open_recovery(state: &AppState) {
    if let Some(ui) = &state.writing.borrow().ui {
        ui.window.present();
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let window = gtk::Window::builder()
        .title(crate::i18n::t("writing_recover"))
        .transient_for(&state.window)
        .default_width(560)
        .default_height(420)
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
    body.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("writing_recover_help"))
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    body.append(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    window.set_child(Some(&body));
    let writing = state.writing.clone();
    window.connect_close_request(move |_| {
        writing.borrow_mut().ui = None;
        glib::Propagation::Proceed
    });
    state.writing.borrow_mut().ui = Some(RecoveryUi {
        window: window.clone(),
        list,
        account,
    });
    refresh(state);
    window.present();
}
