use super::*;
use crate::protocol::ReactionInfo;

pub(super) struct Picker {
    account: String,
    chat: String,
    id: String,
    popover: gtk::Popover,
    content: gtk::Box,
}

fn icon(kind: i32) -> &'static str {
    match kind {
        2 => "👍",
        3 => "❤️",
        4 => "😆",
        5 => "😮",
        6 => "😢",
        7 => "😡",
        _ => "❔",
    }
}
fn title(kind: i32) -> String {
    crate::i18n::t(match kind {
        2 => "reaction_nice",
        3 => "reaction_love",
        4 => "reaction_fun",
        5 => "reaction_amazing",
        6 => "reaction_sad",
        7 => "reaction_omg",
        _ => "reaction_unknown",
    })
}
pub(super) fn eligible(message: &MessageInfo) -> bool {
    message.id.parse::<u128>().is_ok_and(|id| id > 0)
        && !message.decryption_failed
        && !message.unsent_retained
        && message.chat_event.is_none()
        && !["CALL", "CHAT_EVENT", "UNSENT"].contains(&message.content_type.as_str())
}
fn available(message: &MessageInfo) -> bool {
    let now = chrono::Utc::now().timestamp_millis();
    eligible(message)
        && message.created_time > 0
        && message.created_time <= now + 60_000
        && now.saturating_sub(message.created_time) < 7 * 86_400_000
}
fn name(state: &AppState, mid: &str) -> String {
    if state.self_mid.borrow().as_deref() == Some(mid) {
        crate::i18n::t("you")
    } else {
        message_senders::name(state, mid)
    }
}
fn busy(state: &AppState, id: &str) -> bool {
    let account = state.self_mid.borrow();
    state.pending.borrow().values().any(|pending| matches!(pending,
        Pending::Reaction { account: owner, message_id, .. } if Some(owner) == account.as_ref() && message_id == id))
}

pub(super) fn updated(state: &AppState, message: &MessageInfo) -> MessageInfo {
    let mut message = message.clone();
    if eligible(&message)
        && let Some(known) = state.reply_messages.borrow().get(&message.id)
        && known.reaction_version > message.reaction_version
    {
        message.reactions = known.reactions.clone();
        message.reaction_version = known.reaction_version;
    }
    if !eligible(&message) {
        message.reactions.clear();
    }
    message
}

pub(super) fn button(state: &AppState, message: &MessageInfo) -> Option<gtk::Button> {
    if !eligible(message) {
        return None;
    }
    let button = gtk::Button::builder()
        .focus_on_click(false)
        .icon_name("face-smile-symbolic")
        .tooltip_text(crate::i18n::t("reactions"))
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular", "line-reaction-button"])
        .build();
    button.update_property(&[
        gtk::accessible::Property::Label(&crate::i18n::t("reactions")),
        gtk::accessible::Property::Description(&message.text.chars().take(100).collect::<String>()),
    ]);
    let s = state.clone();
    let id = message.id.clone();
    let weak = button.downgrade();
    button.connect_clicked(move |_| {
        if let Some(button) = weak.upgrade() {
            open(&s, &id, button.upcast_ref());
        }
    });
    Some(button)
}

pub(super) fn append(state: &AppState, message: &MessageInfo, row: &gtk::Box) {
    if !eligible(message) {
        return;
    }
    let slot = gtk::Box::builder()
        .spacing(4)
        .margin_top(3)
        .margin_start(8)
        .margin_end(8)
        .halign(if message.mine {
            gtk::Align::End
        } else {
            gtk::Align::Start
        })
        .build();
    row.set_orientation(gtk::Orientation::Vertical);
    row.append(&slot);
    state
        .reaction_slots
        .borrow_mut()
        .insert(message.id.clone(), slot);
    let s = state.clone();
    let id = message.id.clone();
    row.connect_unrealize(move |_| {
        if s.reaction_picker
            .borrow()
            .as_ref()
            .is_some_and(|p| p.id == id)
        {
            close(&s);
        }
    });
    refresh_row(state, &message.id);
}
fn refresh_row(state: &AppState, id: &str) {
    let Some(slot) = state.reaction_slots.borrow().get(id).cloned() else {
        return;
    };
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }
    let Some(message) = state.reply_messages.borrow().get(id).cloned() else {
        return;
    };
    if !eligible(&message) {
        slot.set_visible(false);
        return;
    }
    for kind in [2, 3, 4, 5, 6, 7, 0] {
        let people: Vec<_> = message
            .reactions
            .iter()
            .filter(|r| r.kind == kind)
            .collect();
        if people.is_empty() {
            continue;
        }
        let button = gtk::Button::builder()
            .focus_on_click(false)
            .label(format!("{} {}", icon(kind), people.len()))
            .css_classes(["flat", "pill"])
            .build();
        let names = people
            .iter()
            .take(12)
            .map(|r| name(state, &r.user_mid))
            .collect::<Vec<_>>()
            .join("、");
        button.set_tooltip_text(Some(&format!("{}：{}", title(kind), names)));
        if people
            .iter()
            .any(|r| state.self_mid.borrow().as_deref() == Some(&r.user_mid))
        {
            button.add_css_class("accent");
        }
        let s = state.clone();
        let id = id.to_string();
        button.connect_clicked(move |_| {
            if let Some(row) = s.message_rows.borrow().get(&id).cloned() {
                open(&s, &id, row.upcast_ref());
            }
        });
        slot.append(&button);
    }
    slot.set_visible(slot.first_child().is_some());
}

pub(super) fn close(state: &AppState) {
    let picker = state.reaction_picker.borrow_mut().take();
    if let Some(picker) = picker {
        picker.popover.popdown();
    }
}
fn open(state: &AppState, id: &str, anchor: &gtk::Widget) {
    close(state);
    if !*state.session_ready.borrow() {
        return;
    }
    let Some(message) = state.reply_messages.borrow().get(id).cloned() else {
        return;
    };
    if !eligible(&message) {
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let Some(chat) = state.current_chat.borrow().clone() else {
        return;
    };
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(10)
        .margin_end(10)
        .build();
    let popover = gtk::Popover::builder().child(&content).build();
    let previous_focus =
        gtk::prelude::RootExt::focus(&state.window).map(|widget| widget.downgrade());
    let anchor_weak = anchor.downgrade();
    let return_account = account.clone();
    let return_chat = chat.clone();
    popover.set_parent(anchor);
    let s = state.clone();
    popover.connect_closed(move |popover| {
        if s.reaction_picker
            .borrow()
            .as_ref()
            .is_some_and(|p| p.popover == *popover)
        {
            s.reaction_picker.borrow_mut().take();
        }
        let popover = popover.clone();
        let s = s.clone();
        let previous_focus = previous_focus.clone();
        let anchor_weak = anchor_weak.clone();
        let account = return_account.clone();
        let chat = return_chat.clone();
        glib::idle_add_local_once(move || {
            // ポップアップに残ったフォーカスだけを戻し、外側で選び直した入力先は奪わない。
            let restore = s.reaction_picker.borrow().is_none()
                && s.self_mid.borrow().as_deref() == Some(&account)
                && s.current_chat.borrow().as_deref() == Some(&chat)
                && gtk::prelude::RootExt::focus(&s.window).is_none_or(|focus| {
                    focus == popover
                        || focus.is_ancestor(&popover)
                        || anchor_weak
                            .upgrade()
                            .is_some_and(|anchor| focus == anchor || focus.is_ancestor(&anchor))
                });
            if popover.parent().is_some() {
                popover.unparent();
            }
            if restore
                && let Some(focus) = previous_focus.and_then(|weak| weak.upgrade())
                && focus.root().is_some()
                && focus.is_sensitive()
            {
                focus.grab_focus();
            }
        });
    });
    *state.reaction_picker.borrow_mut() = Some(Picker {
        account,
        chat,
        id: id.into(),
        popover: popover.clone(),
        content,
    });
    refresh_picker(state);
    popover.popup();
}

fn select(state: &AppState, id: &str, kind: Option<i32>) {
    if !*state.session_ready.borrow() || busy(state, id) {
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let Some(chat) = state.current_chat.borrow().clone() else {
        return;
    };
    let Some(message) = state.reply_messages.borrow().get(id).cloned() else {
        return;
    };
    if !available(&message) {
        toast(state, &crate::i18n::t("reaction_expired"));
        return;
    }
    let selected = if kind.is_some_and(|kind| {
        message
            .reactions
            .iter()
            .any(|r| r.user_mid == account && r.kind == kind)
    }) {
        None
    } else {
        kind
    };
    match state.sidecar.request(
        "set_reaction",
        Some(serde_json::json!({"chatMid":chat,"messageId":id,"kind":selected})),
    ) {
        Ok(request) => {
            state.pending.borrow_mut().insert(
                request,
                Pending::Reaction {
                    account,
                    chat_mid: chat,
                    message_id: id.into(),
                },
            );
        }
        Err(_) => toast(state, &crate::i18n::t("reaction_failed")),
    }
    refresh_picker(state);
}
fn refresh_picker(state: &AppState) {
    let picker = state.reaction_picker.borrow();
    let Some(picker) = picker.as_ref() else {
        return;
    };
    if state.self_mid.borrow().as_deref() != Some(&picker.account)
        || state.current_chat.borrow().as_deref() != Some(&picker.chat)
    {
        return;
    }
    let content = picker.content.clone();
    let id = picker.id.clone();
    let account = picker.account.clone();
    let Some(message) = state.reply_messages.borrow().get(&id).cloned() else {
        return;
    };
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    content.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("reactions"))
            .css_classes(["heading"])
            .xalign(0.0)
            .build(),
    );
    let choices = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    for kind in 2..=7 {
        let button = gtk::Button::builder()
            .label(icon(kind))
            .tooltip_text(title(kind))
            .css_classes(["flat", "circular"])
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&title(kind))]);
        if message
            .reactions
            .iter()
            .any(|r| r.user_mid == account && r.kind == kind)
        {
            button.add_css_class("suggested-action");
        }
        button.set_sensitive(available(&message) && !busy(state, &id));
        let s = state.clone();
        let id = id.clone();
        button.connect_clicked(move |_| select(&s, &id, Some(kind)));
        choices.append(&button);
    }
    content.append(&choices);
    if !available(&message) {
        content.append(
            &gtk::Label::builder()
                .label(crate::i18n::t("reaction_expired"))
                .wrap(true)
                .max_width_chars(28)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
    }
    if message.reactions.iter().any(|r| r.user_mid == account) {
        let remove = gtk::Button::with_label(&crate::i18n::t("reaction_remove"));
        remove.set_sensitive(available(&message) && !busy(state, &id));
        let s = state.clone();
        let id = id.clone();
        remove.connect_clicked(move |_| select(&s, &id, None));
        content.append(&remove);
    }
    let people = gtk::Box::new(gtk::Orientation::Vertical, 5);
    people.append(
        &gtk::Label::builder()
            .label(crate::i18n::tf(
                "reaction_people",
                &[("n", &message.reactions.len().to_string())],
            ))
            .xalign(0.0)
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    for r in &message.reactions {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(
            &gtk::Label::builder()
                .label(icon(r.kind))
                .tooltip_text(title(r.kind))
                .build(),
        );
        let label = gtk::Label::builder()
            .label(name(state, &r.user_mid))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(22)
            .build();
        label.set_tooltip_text(Some(&name(state, &r.user_mid)));
        row.append(&label);
        people.append(&row);
    }
    content.append(
        &gtk::ScrolledWindow::builder()
            .child(&people)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(260)
            .propagate_natural_height(true)
            .build(),
    );
}
pub(super) fn refresh(state: &AppState) {
    let ids: Vec<_> = state.reaction_slots.borrow().keys().cloned().collect();
    for id in ids {
        refresh_row(state, &id);
    }
    refresh_picker(state);
}
pub(super) fn apply(
    state: &AppState,
    account: &str,
    chat: &str,
    id: &str,
    reaction: ReactionInfo,
    version: u64,
) {
    if !*state.session_ready.borrow()
        || state.self_mid.borrow().as_deref() != Some(account)
        || state.current_chat.borrow().as_deref() != Some(chat)
        || !reaction.user_mid.starts_with('u')
        || !(-1..=7).contains(&reaction.kind)
        || reaction.kind == 1
    {
        return;
    }
    {
        let mut messages = state.reply_messages.borrow_mut();
        let Some(message) = messages.get_mut(id) else {
            return;
        };
        if !eligible(message) || version < message.reaction_version {
            return;
        }
        message
            .reactions
            .retain(|r| r.user_mid != reaction.user_mid);
        if reaction.kind != -1 {
            message.reactions.push(reaction);
        }
        message.reaction_version = version;
    }
    *state.msg_list_fp.borrow_mut() = None;
    refresh_row(state, id);
    refresh_picker(state);
}
pub(super) fn response(state: &AppState, account: &str, chat: &str, id: &str, ok: bool) {
    if state.self_mid.borrow().as_deref() != Some(account)
        || state.current_chat.borrow().as_deref() != Some(chat)
    {
        return;
    }
    if !ok {
        toast(state, &crate::i18n::t("reaction_failed"));
    }
    refresh_row(state, id);
    refresh_picker(state);
}
pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new("message-reactions", Some(glib::VariantTy::STRING));
    let s = state.clone();
    action.connect_activate(move |_, value| {
        if let Some(id) = value.and_then(|v| v.str())
            && let Some(row) = s.message_rows.borrow().get(id).cloned()
        {
            open(&s, id, row.upcast_ref());
        }
    });
    state.window.add_action(&action);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_readable_messages_can_have_reactions() {
        let mut message: MessageInfo = serde_json::from_value(serde_json::json!({
            "id":"9007199254740993", "text":"本文", "contentType":"IMAGE",
            "createdTime":chrono::Utc::now().timestamp_millis()
        }))
        .unwrap();
        assert!(available(&message));
        message.created_time -= 8 * 86_400_000;
        assert!(eligible(&message));
        assert!(!available(&message));
        message.unsent_retained = true;
        assert!(!eligible(&message));
        message.unsent_retained = false;
        message.content_type = "UNSENT".into();
        assert!(!eligible(&message));
        message.content_type = "NONE".into();
        message.decryption_failed = true;
        assert!(!eligible(&message));
        message.decryption_failed = false;
        message.id = "pending-1".into();
        assert!(!eligible(&message));
    }
}
