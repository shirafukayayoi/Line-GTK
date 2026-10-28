use super::*;
use crate::protocol::{PaidReaction, ReactionInfo};

#[derive(Clone, serde::Deserialize)]
struct EmojiPackage {
    id: String,
    name: String,
}
#[derive(Clone, serde::Deserialize)]
struct EmojiChoice {
    emoji: PaidReaction,
    key: String,
    label: String,
}

pub(super) struct Picker {
    account: String,
    chat: String,
    id: String,
    popover: gtk::Popover,
    content: gtk::Box,
    gallery: bool,
    loading: bool,
    error: Option<String>,
    warning: bool,
    packages: Vec<EmojiPackage>,
    selected_package: String,
    choices: Vec<EmojiChoice>,
    scroll: gtk::Adjustment,
}

fn emoji_key(emoji: &PaidReaction) -> String {
    format!("{}_{}", emoji.product_id, emoji.emoji_id)
}
fn same_emoji(a: &PaidReaction, b: &PaidReaction) -> bool {
    a.product_id == b.product_id && a.emoji_id == b.emoji_id
}
fn reaction_key(reaction: &ReactionInfo) -> String {
    reaction
        .emoji
        .as_ref()
        .map(emoji_key)
        .unwrap_or_else(|| format!("standard:{}", reaction.kind))
}
fn picture(state: &AppState, emoji: &PaidReaction, size: i32) -> gtk::Picture {
    let key = emoji_key(emoji);
    let picture = gtk::Picture::builder()
        .alternative_text(format!(
            "{} {}",
            crate::i18n::t("reaction_line_emoji"),
            emoji.emoji_id
        ))
        .width_request(size)
        .height_request(size)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .build();
    picture.set_paintable(state.emoji_textures.borrow().get(&key));
    let mut images = state.reaction_images.borrow_mut();
    if images.len() > 2000 {
        images.retain(|_, values| values.iter().any(|v| v.upgrade().is_some()));
    }
    let values = images.entry(key).or_default();
    values.retain(|v| v.upgrade().is_some());
    values.push(picture.downgrade());
    picture
}
pub(super) fn emoji_ready(state: &AppState, key: &str) {
    if let Some(values) = state.reaction_images.borrow_mut().get_mut(key) {
        let textures = state.emoji_textures.borrow();
        values.retain(|weak| {
            if let Some(picture) = weak.upgrade() {
                picture.set_paintable(textures.get(key));
                true
            } else {
                false
            }
        });
    }
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

pub(super) fn append(state: &AppState, message: &MessageInfo, row: &gtk::Box, content: &gtk::Box) {
    if !eligible(message) {
        return;
    }
    let slot = gtk::Box::builder()
        .valign(gtk::Align::Center)
        .css_classes(["line-reactions"])
        .build();
    content.append(&slot);
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
    let mut grouped = std::collections::BTreeMap::<String, Vec<&ReactionInfo>>::new();
    for reaction in &message.reactions {
        grouped
            .entry(reaction_key(reaction))
            .or_default()
            .push(reaction);
    }
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(2)
        .min_children_per_line(1)
        .row_spacing(2)
        .column_spacing(2)
        .build();
    for people in grouped.values() {
        let kind = people[0].kind;
        let key = reaction_key(people[0]);
        let button = gtk::Button::builder()
            .focus_on_click(false)
            .css_classes(["flat", "pill"])
            .build();
        if let Some(emoji) = &people[0].emoji {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 3);
            content.append(&picture(state, emoji, 24));
            content.append(&gtk::Label::new(Some(&people.len().to_string())));
            button.set_child(Some(&content));
            button.update_property(&[gtk::accessible::Property::Label(&format!(
                "LINE {} ({})",
                emoji.emoji_id,
                people.len()
            ))]);
        } else {
            button.set_label(&format!("{} {}", icon(kind), people.len()));
        }
        let names = people
            .iter()
            .take(12)
            .map(|r| name(state, &r.user_mid))
            .collect::<Vec<_>>()
            .join("、");
        let reaction_title = if people[0].emoji.is_some() {
            crate::i18n::t("reaction_line_emoji")
        } else {
            title(kind)
        };
        button.set_tooltip_text(Some(&format!("{}：{}", reaction_title, names)));
        button.update_property(&[gtk::accessible::Property::Description(&key)]);
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
        flow.insert(&button, -1);
    }
    slot.append(&flow);
    slot.set_visible(!message.reactions.is_empty());
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
        gallery: false,
        warning: false,
        loading: false,
        error: None,
        packages: Vec::new(),
        selected_package: String::new(),
        choices: Vec::new(),
        scroll: gtk::Adjustment::new(0.0, 0.0, 0.0, 10.0, 100.0, 0.0),
    });
    refresh_picker(state);
    popover.popup();
}

fn select(state: &AppState, id: &str, kind: Option<i32>, emoji: Option<PaidReaction>) {
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
        message.reactions.iter().any(|r| {
            r.user_mid == account
                && if let Some(emoji) = &emoji {
                    r.emoji.as_ref().is_some_and(|own| same_emoji(own, emoji))
                } else {
                    r.kind == kind && r.emoji.is_none()
                }
        })
    }) {
        None
    } else {
        kind
    };
    match state.sidecar.request(
        "set_reaction",
        Some(serde_json::json!({"chatMid":chat,"messageId":id,"kind":selected,"emoji": if selected.is_some() { emoji } else { None }})),
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
    let scroll = picker.scroll.value();
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    if picker.gallery {
        render_gallery(state, picker, &message, scroll);
        return;
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
        button.connect_clicked(move |_| select(&s, &id, Some(kind), None));
        choices.append(&button);
    }
    content.append(&choices);
    let line = gtk::Button::with_label(&crate::i18n::t("reaction_line_emoji"));
    let s = state.clone();
    line.connect_clicked(move |_| catalog_request(&s, None, false));
    content.append(&line);
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
        remove.connect_clicked(move |_| select(&s, &id, None, None));
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
        if let Some(emoji) = &r.emoji {
            row.append(&picture(state, emoji, 24));
        } else {
            row.append(
                &gtk::Label::builder()
                    .label(icon(r.kind))
                    .tooltip_text(title(r.kind))
                    .build(),
            );
        }
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

fn catalog_request(state: &AppState, package: Option<String>, force: bool) {
    let already_loading = state
        .reaction_picker
        .borrow()
        .as_ref()
        .is_some_and(|p| p.loading);
    if already_loading {
        if let Some(picker) = state.reaction_picker.borrow_mut().as_mut() {
            picker.gallery = true;
        }
        refresh_picker(state);
        return;
    }
    let (account, popover) = {
        let mut picker = state.reaction_picker.borrow_mut();
        let Some(picker) = picker.as_mut() else {
            return;
        };
        if picker.loading {
            return;
        }
        picker.gallery = true;
        picker.loading = true;
        picker.error = None;
        if let Some(id) = &package {
            if picker.selected_package != *id {
                picker.scroll.set_value(0.0);
            }
            picker.selected_package = id.clone();
            picker.choices.clear();
        }
        (picker.account.clone(), picker.popover.downgrade())
    };
    let request = if let Some(id) = &package {
        state.sidecar.request(
            "list_reaction_emojis",
            Some(serde_json::json!({"productId":id})),
        )
    } else {
        state.sidecar.request(
            "list_reaction_emoji_packages",
            Some(serde_json::json!({"force":force})),
        )
    };
    match request {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::ReactionCatalog {
                    account,
                    picker: popover,
                    package,
                },
            );
        }
        Err(_) => {
            if let Some(picker) = state.reaction_picker.borrow_mut().as_mut() {
                picker.loading = false;
                picker.error = Some(crate::i18n::t("reaction_emoji_failed"));
            }
        }
    }
    refresh_picker(state);
}

pub(super) fn catalog_response(
    state: &AppState,
    account: &str,
    popover: &glib::WeakRef<gtk::Popover>,
    package: Option<String>,
    ok: bool,
    result: &serde_json::Value,
) {
    let mut next = None;
    {
        let mut current = state.reaction_picker.borrow_mut();
        let Some(picker) = current.as_mut() else {
            return;
        };
        if picker.account != account
            || state.self_mid.borrow().as_deref() != Some(account)
            || popover.upgrade().as_ref() != Some(&picker.popover)
        {
            return;
        }
        picker.loading = false;
        if !picker.gallery {
            return;
        }
        if !ok {
            picker.error = Some(crate::i18n::t("reaction_emoji_failed"));
        } else if package.is_some() {
            match serde_json::from_value(result.get("emojis").cloned().unwrap_or_default()) {
                Ok(choices) => picker.choices = choices,
                Err(_) => picker.error = Some(crate::i18n::t("reaction_emoji_failed")),
            }
        } else {
            picker.packages =
                serde_json::from_value(result.get("packages").cloned().unwrap_or_default())
                    .unwrap_or_default();
            picker.warning = result.get("warning").and_then(|v| v.as_bool()) == Some(true);
            next = picker
                .packages
                .iter()
                .find(|p| p.id == picker.selected_package)
                .or(picker.packages.first())
                .map(|p| p.id.clone());
        }
    }
    if let Some(id) = next {
        catalog_request(state, Some(id), false);
    } else {
        refresh_picker(state);
    }
}

fn render_gallery(state: &AppState, picker: &Picker, message: &MessageInfo, scroll: f64) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text(crate::i18n::t("reaction_emoji_back"))
        .css_classes(["flat"])
        .build();
    let s = state.clone();
    back.connect_clicked(move |_| {
        if let Some(picker) = s.reaction_picker.borrow_mut().as_mut() {
            picker.gallery = false;
        }
        refresh_picker(&s);
    });
    header.append(&back);
    header.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("reaction_line_emoji"))
            .hexpand(true)
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    let reload = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(crate::i18n::t("reaction_emoji_refresh"))
        .sensitive(!picker.loading)
        .css_classes(["flat"])
        .build();
    let s = state.clone();
    reload.connect_clicked(move |_| catalog_request(&s, None, true));
    header.append(&reload);
    picker.content.append(&header);
    if !picker.packages.is_empty() {
        let names: Vec<_> = picker.packages.iter().map(|p| p.name.as_str()).collect();
        let dropdown = gtk::DropDown::from_strings(&names);
        dropdown.set_selected(
            picker
                .packages
                .iter()
                .position(|p| p.id == picker.selected_package)
                .unwrap_or(0) as u32,
        );
        dropdown.set_sensitive(!picker.loading);
        dropdown.set_enable_search(true);
        fn limit_label(widget: &gtk::Widget) {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_max_width_chars(24);
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                limit_label(&widget);
            }
        }
        limit_label(dropdown.upcast_ref());
        let ids: Vec<_> = picker.packages.iter().map(|p| p.id.clone()).collect();
        let s = state.clone();
        dropdown.connect_selected_notify(move |dropdown| {
            if let Some(id) = ids.get(dropdown.selected() as usize) {
                catalog_request(&s, Some(id.clone()), false);
            }
        });
        picker.content.append(&dropdown);
    }
    if let Some(error) = &picker.error {
        picker.content.append(
            &gtk::Label::builder()
                .label(error)
                .wrap(true)
                .max_width_chars(28)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
    }
    if picker.warning {
        picker.content.append(
            &gtk::Label::builder()
                .label(crate::i18n::t("reaction_emoji_partial"))
                .wrap(true)
                .max_width_chars(28)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
    }
    if picker.loading {
        picker
            .content
            .append(&gtk::Spinner::builder().spinning(true).build());
    }
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(6)
        .max_children_per_line(6)
        .row_spacing(2)
        .column_spacing(2)
        .build();
    for choice in &picker.choices {
        let image = picture(state, &choice.emoji, 32);
        let button = gtk::Button::builder()
            .child(&image)
            .tooltip_text(&choice.label)
            .css_classes(["flat"])
            .build();
        button.update_property(&[
            gtk::accessible::Property::Label(&choice.label),
            gtk::accessible::Property::Description(&choice.key),
        ]);
        button.set_sensitive(
            available(message)
                && !picker.loading
                && !busy(state, &picker.id)
                && image.paintable().is_some(),
        );
        let weak = button.downgrade();
        let s = state.clone();
        let id = picker.id.clone();
        let account = picker.account.clone();
        image.connect_paintable_notify(move |image| {
            if let Some(button) = weak.upgrade() {
                let valid = s.self_mid.borrow().as_deref() == Some(&account)
                    && s.reply_messages.borrow().get(&id).is_some_and(available);
                button.set_sensitive(
                    valid
                        && !s
                            .reaction_picker
                            .borrow()
                            .as_ref()
                            .is_some_and(|p| p.loading)
                        && !busy(&s, &id)
                        && image.paintable().is_some(),
                );
            }
        });
        if message.reactions.iter().any(|r| {
            r.user_mid == picker.account
                && r.emoji
                    .as_ref()
                    .is_some_and(|emoji| same_emoji(emoji, &choice.emoji))
        }) {
            button.add_css_class("suggested-action");
        }
        let s = state.clone();
        let id = picker.id.clone();
        let emoji = choice.emoji.clone();
        button.connect_clicked(move |_| select(&s, &id, Some(0), Some(emoji.clone())));
        flow.insert(&button, -1);
    }
    picker.content.append(
        &gtk::ScrolledWindow::builder()
            .child(&flow)
            .vadjustment(&picker.scroll)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(280)
            .propagate_natural_height(true)
            .build(),
    );
    let adjustment = picker.scroll.clone();
    let weak = picker.popover.downgrade();
    let s = state.clone();
    glib::idle_add_local_once(move || {
        if s.reaction_picker
            .borrow()
            .as_ref()
            .is_some_and(|p| weak.upgrade().as_ref() == Some(&p.popover))
        {
            adjustment.set_value(scroll);
        }
    });
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
    fn line_emoji_groups_use_product_and_emoji_ids() {
        let mut a: ReactionInfo = serde_json::from_value(serde_json::json!({
            "userMid":"u-a", "kind":0, "emoji":{"productId":"670e0cce840a8236ddd4ee4c","emojiId":"001","resourceType":2,"version":123}
        })).unwrap();
        let key = reaction_key(&a);
        a.emoji.as_mut().unwrap().emoji_id = "002".into();
        assert_ne!(key, reaction_key(&a));
        let mut b = a.emoji.clone().unwrap();
        b.version = 124;
        assert!(same_emoji(a.emoji.as_ref().unwrap(), &b));
        b.product_id = "1234567890abcdef12345678".into();
        assert!(!same_emoji(a.emoji.as_ref().unwrap(), &b));
    }

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
