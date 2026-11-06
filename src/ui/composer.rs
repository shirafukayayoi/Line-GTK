use gtk::prelude::*;

/// 複数行の入力、案内文、絵文字ボタンをまとめたメッセージ入力欄。
#[derive(Clone)]
pub(super) struct Composer {
    pub view: gtk::TextView,
    pub container: gtk::Box,
    placeholder: gtk::Label,
    emoji: gtk::Button,
    preediting: std::rc::Rc<std::cell::Cell<bool>>,
}

impl std::ops::Deref for Composer {
    type Target = gtk::TextView;

    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

impl Composer {
    pub fn new() -> Self {
        let view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .hexpand(true)
            .left_margin(12)
            .right_margin(4)
            .top_margin(8)
            .bottom_margin(8)
            .css_classes(["line-composer-text"])
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(34)
            .max_content_height(160)
            .propagate_natural_height(true)
            .hexpand(true)
            .child(&view)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_hexpand(true);
        overlay.set_child(Some(&scroll));
        let placeholder = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start(12)
            .margin_top(8)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .can_target(false)
            .css_classes(["dim-label"])
            .build();
        overlay.add_overlay(&placeholder);
        let preediting = std::rc::Rc::new(std::cell::Cell::new(false));
        let active = preediting.clone();
        let weak = placeholder.downgrade();
        view.buffer().connect_changed(move |buffer| {
            if let Some(label) = weak.upgrade() {
                label.set_visible(buffer.char_count() == 0 && !active.get());
            }
        });
        let weak = placeholder.downgrade();
        let active = preediting.clone();
        view.connect_preedit_changed(move |view, text| {
            active.set(!text.is_empty());
            if let Some(label) = weak.upgrade() {
                label.set_visible(view.buffer().char_count() == 0 && text.is_empty());
            }
        });
        let emoji = gtk::Button::builder()
            .icon_name("face-smile-symbolic")
            .valign(gtk::Align::End)
            .css_classes(["flat", "circular", "line-composer-emoji"])
            .build();
        let weak = view.downgrade();
        emoji.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.grab_focus();
                view.emit_by_name::<()>("insert-emoji", &[]);
            }
        });
        let container = gtk::Box::builder()
            .hexpand(true)
            .valign(gtk::Align::Center)
            .css_classes(["line-composer-entry"])
            .build();
        container.append(&overlay);
        container.append(&emoji);
        Self {
            view,
            container,
            placeholder,
            emoji,
            preediting,
        }
    }

    pub fn text(&self) -> glib::GString {
        let buffer = self.view.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)
    }

    pub fn set_text(&self, text: &str) {
        self.view.buffer().set_text(text);
    }

    pub fn apply_language(&self, send_on_enter: bool) {
        let label = crate::i18n::t("type_message");
        self.placeholder.set_label(&label);
        self.view
            .update_property(&[gtk::accessible::Property::Label(&label)]);
        self.view
            .set_tooltip_text(Some(&crate::i18n::t(if send_on_enter {
                "composer_shortcuts_enter"
            } else {
                "composer_shortcuts"
            })));
        self.emoji
            .set_tooltip_text(Some(&crate::i18n::t("insert_emoji")));
    }

    pub fn connect_send(
        &self,
        send_on_enter: impl Fn() -> bool + 'static,
        send: impl Fn() + 'static,
    ) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.view.downgrade();
        let preediting = self.preediting.clone();
        keys.connect_key_pressed(move |controller, key, _, modifiers| {
            if !is_send_key(key, modifiers, send_on_enter()) {
                return glib::Propagation::Proceed;
            }
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            // IMEの確定を優先する。確定でpreeditが消えても同じキーで送信しない。
            let was_preediting = preediting.get();
            if controller
                .current_event()
                .is_some_and(|event| view.im_context_filter_keypress(event))
            {
                return glib::Propagation::Stop;
            }
            if was_preediting || preediting.get() {
                return glib::Propagation::Proceed;
            }
            send();
            glib::Propagation::Stop
        });
        self.view.add_controller(keys);
    }
}

fn is_send_key(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType, send_on_enter: bool) -> bool {
    use gtk::gdk::{Key, ModifierType as Mods};
    if !matches!(key, Key::Return | Key::KP_Enter | Key::ISO_Enter) {
        return false;
    }
    let shortcut = modifiers
        & (Mods::CONTROL_MASK
            | Mods::SHIFT_MASK
            | Mods::ALT_MASK
            | Mods::SUPER_MASK
            | Mods::HYPER_MASK
            | Mods::META_MASK);
    if send_on_enter {
        shortcut.is_empty()
    } else {
        shortcut == Mods::CONTROL_MASK
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::gdk::{Key, ModifierType as Mods};

    #[test]
    fn only_the_selected_enter_combination_sends_even_with_caps_lock() {
        let modifiers = [
            Mods::CONTROL_MASK,
            Mods::SHIFT_MASK,
            Mods::ALT_MASK,
            Mods::SUPER_MASK,
            Mods::HYPER_MASK,
            Mods::META_MASK,
        ];
        for bits in 0..64 {
            let combination = modifiers
                .iter()
                .enumerate()
                .fold(Mods::empty(), |acc, (i, flag)| {
                    if bits & (1 << i) != 0 {
                        acc | *flag
                    } else {
                        acc
                    }
                });
            for lock in [Mods::empty(), Mods::LOCK_MASK] {
                for key in [Key::Return, Key::KP_Enter, Key::ISO_Enter] {
                    assert_eq!(is_send_key(key, combination | lock, true), bits == 0);
                    assert_eq!(is_send_key(key, combination | lock, false), bits == 1);
                }
                assert!(!is_send_key(Key::a, combination | lock, true));
                assert!(!is_send_key(Key::a, combination | lock, false));
            }
        }
    }
}
