use super::*;

#[derive(Default)]
pub(super) struct State {
    ui: Option<SearchUi>,
    ids: Vec<String>,
    index: usize,
    rows: Vec<gtk::Box>,
    scheduled: bool,
    scroll_requested: bool,
}

#[derive(Clone)]
struct SearchUi {
    revealer: gtk::Revealer,
    entry: gtk::SearchEntry,
    count: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    close: gtk::Button,
    hint: gtk::Label,
    open: gtk::Button,
}

fn matching<'a>(messages: impl Iterator<Item = &'a MessageInfo>, query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() || query.chars().count() > 256 {
        return Vec::new();
    }
    let mut matches: Vec<_> = messages
        .filter(|message| {
            !message.id.is_empty()
                && !message.id.starts_with("pending-")
                && !message.decryption_failed
                && message.content_type.eq_ignore_ascii_case("NONE")
                && message.text.to_lowercase().contains(&query)
        })
        .collect();
    matches.sort_by_key(|message| {
        (
            message.created_time,
            message.id.parse::<u128>().unwrap_or(0),
        )
    });
    matches
        .into_iter()
        .map(|message| message.id.clone())
        .collect()
}

pub(super) fn wire(state: &AppState) {
    let entry = gtk::SearchEntry::builder().hexpand(true).build();
    let count = gtk::Label::new(Some("0/0"));
    let previous = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .css_classes(["flat"])
        .build();
    let next = gtk::Button::builder()
        .icon_name("go-down-symbolic")
        .css_classes(["flat"])
        .build();
    let dismiss = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .css_classes(["flat"])
        .build();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.append(&entry);
    row.append(&count);
    row.append(&previous);
    row.append(&next);
    row.append(&dismiss);
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let hint = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption", "dim-label"])
        .build();
    body.append(&row);
    body.append(&hint);
    let revealer = gtk::Revealer::builder()
        .child(&body)
        .transition_type(gtk::RevealerTransitionType::None)
        .build();
    state
        .conversation
        .insert_child_after(&revealer, state.conversation.first_child().as_ref());
    let open = gtk::Button::builder()
        .icon_name("system-search-symbolic")
        .css_classes(["flat", "circular"])
        .build();
    if let Some(header) = state.mute_btn.parent().and_downcast::<gtk::Box>() {
        header.insert_child_after(&open, Some(&state.mute_btn));
    }
    state.message_search.borrow_mut().ui = Some(SearchUi {
        revealer,
        entry: entry.clone(),
        count,
        previous: previous.clone(),
        next: next.clone(),
        close: dismiss.clone(),
        hint,
        open: open.clone(),
    });
    let action = gio::SimpleAction::new("find-messages", None);
    let s = state.clone();
    action.connect_activate(move |_, _| show(&s));
    state.window.add_action(&action);
    state
        .app
        .set_accels_for_action("win.find-messages", &["<Primary>f"]);
    let s = state.clone();
    open.connect_clicked(move |_| show(&s));
    let s = state.clone();
    entry.connect_search_changed(move |_| rebuild(&s, true));
    let s = state.clone();
    entry.connect_activate(move |_| step(&s, false));
    let s = state.clone();
    entry.connect_stop_search(move |_| {
        close(&s);
        s.composer.grab_focus();
    });
    let s = state.clone();
    previous.connect_clicked(move |_| step(&s, true));
    let s = state.clone();
    next.connect_clicked(move |_| step(&s, false));
    let s = state.clone();
    dismiss.connect_clicked(move |_| {
        close(&s);
        s.composer.grab_focus();
    });
    let key = gtk::EventControllerKey::new();
    let s = state.clone();
    key.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gdk::Key::Return && modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
            step(&s, true);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(key);
    if let Some(display) = gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(".line-message-search-hit { background-color: rgba(70,110,200,0.10); border-radius: 8px; } .line-message-search-current { background-color: rgba(70,110,200,0.25); }");
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    apply_language(state);
}

pub(super) fn apply_language(state: &AppState) {
    let Some(ui) = state.message_search.borrow().ui.clone() else {
        return;
    };
    ui.entry
        .set_placeholder_text(Some(&crate::i18n::t("message_search_placeholder")));
    ui.entry
        .update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
            "message_search_placeholder",
        ))]);
    ui.hint.set_text(&crate::i18n::t("message_search_scope"));
    for (button, key) in [
        (&ui.previous, "message_search_previous"),
        (&ui.next, "message_search_next"),
        (&ui.close, "message_search_close"),
        (&ui.open, "message_search_open"),
    ] {
        button.set_tooltip_text(Some(&crate::i18n::t(key)));
        button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(key))]);
    }
}

fn show(state: &AppState) {
    if state.current_chat.borrow().is_none() {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    }
    let Some(ui) = state.message_search.borrow().ui.clone() else {
        return;
    };
    ui.revealer.set_reveal_child(true);
    ui.entry.grab_focus();
    ui.entry.select_region(0, -1);
    rebuild(state, false);
}

pub(super) fn close(state: &AppState) {
    reset_rows(state);
    let ui = state.message_search.borrow().ui.clone();
    if let Some(ui) = ui {
        ui.revealer.set_reveal_child(false);
        ui.entry.set_text("");
    }
}

pub(super) fn reset_rows(state: &AppState) {
    let mut search = state.message_search.borrow_mut();
    for row in search.rows.drain(..) {
        row.remove_css_class("line-message-search-hit");
        row.remove_css_class("line-message-search-current");
    }
    search.ids.clear();
    search.index = 0;
    search.scroll_requested = false;
}

pub(super) fn changed(state: &AppState) {
    let mut search = state.message_search.borrow_mut();
    if search.scheduled
        || search
            .ui
            .as_ref()
            .is_none_or(|ui| !ui.revealer.reveals_child())
    {
        return;
    }
    search.scheduled = true;
    drop(search);
    let s = state.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
        s.message_search.borrow_mut().scheduled = false;
        rebuild(&s, false);
    });
}

fn rebuild(state: &AppState, jump: bool) {
    let Some(ui) = state.message_search.borrow().ui.clone() else {
        return;
    };
    if !ui.revealer.reveals_child() {
        return;
    }
    let ids = matching(
        state.reply_messages.borrow().values(),
        ui.entry.text().as_str(),
    );
    let mut search = state.message_search.borrow_mut();
    let selected = search.ids.get(search.index).cloned();
    for row in search.rows.drain(..) {
        row.remove_css_class("line-message-search-hit");
        row.remove_css_class("line-message-search-current");
    }
    search.index = if jump {
        0
    } else {
        selected
            .and_then(|id| ids.iter().position(|item| item == &id))
            .unwrap_or(0)
    };
    search.ids = ids;
    let ids = search.ids.clone();
    search.rows = ids
        .iter()
        .filter_map(|id| state.message_rows.borrow().get(id).cloned())
        .collect();
    for row in &search.rows {
        row.add_css_class("line-message-search-hit");
    }
    search.scroll_requested |= jump;
    drop(search);
    select(state);
}

fn step(state: &AppState, backwards: bool) {
    let mut search = state.message_search.borrow_mut();
    let n = search.ids.len();
    if n == 0 {
        return;
    }
    search.index = if backwards {
        (search.index + n - 1) % n
    } else {
        (search.index + 1) % n
    };
    search.scroll_requested = true;
    drop(search);
    select(state);
}

fn select(state: &AppState) {
    let mut search = state.message_search.borrow_mut();
    let Some(ui) = search.ui.clone() else {
        return;
    };
    let n = search.ids.len();
    ui.count.set_text(&format!(
        "{}/{}",
        if n == 0 { 0 } else { search.index + 1 },
        n
    ));
    ui.previous.set_sensitive(n > 0);
    ui.next.set_sensitive(n > 0);
    for row in &search.rows {
        row.remove_css_class("line-message-search-current");
    }
    let row = search
        .ids
        .get(search.index)
        .and_then(|id| state.message_rows.borrow().get(id).cloned());
    if let Some(row) = row {
        row.add_css_class("line-message-search-current");
        if search.scroll_requested {
            *state.stick_bottom.borrow_mut() = false;
            let generation = state.scroll_pin_gen.borrow().wrapping_add(1);
            *state.scroll_pin_gen.borrow_mut() = generation;
            *state.scroll_pinning.borrow_mut() = false;
            search.scroll_requested = !state.message_list.scroll_to_row(row.upcast_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: &str, time: i64, text: &str, kind: &str) -> MessageInfo {
        serde_json::from_value(
            serde_json::json!({"id":id,"createdTime":time,"text":text,"contentType":kind}),
        )
        .unwrap()
    }
    #[test]
    fn search_is_unicode_safe_case_insensitive_and_in_time_order() {
        let messages = [
            message("12", 20, "待ち合わせ\n駅前🙂", "NONE"),
            message("11", 10, "駅前でMeet", "NONE"),
            message("10", 20, "駅前", "NONE"),
        ];
        assert_eq!(matching(messages.iter(), "駅前"), vec!["11", "10", "12"]);
        assert_eq!(matching(messages.iter(), "MEET"), vec!["11"]);
        assert_eq!(matching(messages.iter(), "🙂"), vec!["12"]);
        assert!(matching(messages.iter(), " ").is_empty());
    }
    #[test]
    fn cancelled_encrypted_and_pending_text_are_not_search_results() {
        let mut encrypted = message("3", 3, "検索語", "NONE");
        encrypted.decryption_failed = true;
        let messages = [
            message("1", 1, "検索語", "UNSENT"),
            message("pending-2", 2, "検索語", "NONE"),
            encrypted,
            message("4", 4, "検索語", "CALL"),
        ];
        assert!(matching(messages.iter(), "検索語").is_empty());
        assert!(matching(messages.iter(), &"語".repeat(257)).is_empty());
    }
}
