use super::*;
use crate::protocol::ChatEventInfo;

fn text(state: &AppState, event: &ChatEventInfo) -> String {
    let name = |mid: &str| {
        if state.self_mid.borrow().as_deref() == Some(mid) {
            crate::i18n::t("you")
        } else {
            message_senders::name(state, mid)
        }
    };
    let actor = name(&event.actor_mid);
    let targets = event
        .target_mids
        .iter()
        .map(|mid| name(mid))
        .collect::<Vec<_>>()
        .join("、");
    let key = match event.kind.as_str() {
        "invited" => "chat_event_invited",
        "joined" => "chat_event_joined",
        "left" => "chat_event_left",
        "removed" => "chat_event_removed",
        _ => "chat_event_unknown",
    };
    crate::i18n::tf(key, &[("actor", &actor), ("targets", &targets)])
}
pub(super) fn append(state: &AppState, event: &ChatEventInfo) -> gtk::Box {
    let label = gtk::Label::builder()
        .label(text(state, event))
        .use_markup(false)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(60)
        .xalign(0.5)
        .halign(gtk::Align::Center)
        .hexpand(true)
        .margin_start(24)
        .margin_end(24)
        .margin_top(8)
        .margin_bottom(8)
        .css_classes(["dim-label", "caption", "line-chat-event"])
        .build();
    let row = gtk::Box::builder()
        .css_classes(["line-message-row", "line-chat-event-row"])
        .build();
    row.append(&label);
    state.message_list.append(&row);
    state
        .chat_event_labels
        .borrow_mut()
        .push((event.clone(), label));
    row
}
pub(super) fn refresh(state: &AppState) {
    for (event, label) in state.chat_event_labels.borrow().iter() {
        label.set_text(&text(state, event));
    }
}
