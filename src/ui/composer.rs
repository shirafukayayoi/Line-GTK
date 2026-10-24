use gtk::prelude::*;

/// 複数行の入力、案内文、絵文字ボタンをまとめたメッセージ入力欄。
#[derive(Clone)]
pub(super) struct Composer {
    pub view: gtk::TextView,
    pub container: gtk::Box,
    placeholder: gtk::Label,
    emoji: gtk::Button,
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
        view.connect_preedit_changed(move |view, text| {
            preediting.set(!text.is_empty());
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
        }
    }

    pub fn text(&self) -> glib::GString {
        let buffer = self.view.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)
    }

    pub fn set_text(&self, text: &str) {
        self.view.buffer().set_text(text);
    }

    pub fn apply_language(&self) {
        let label = crate::i18n::t("type_message");
        self.placeholder.set_label(&label);
        self.view
            .update_property(&[gtk::accessible::Property::Label(&label)]);
        self.view
            .set_tooltip_text(Some(&crate::i18n::t("composer_shortcuts")));
        self.emoji
            .set_tooltip_text(Some(&crate::i18n::t("insert_emoji")));
    }

    pub fn connect_send(&self, send: impl Fn() + 'static) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.view.downgrade();
        keys.connect_key_pressed(move |controller, key, _, modifiers| {
            if !matches!(
                key,
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::ISO_Enter
            ) {
                return glib::Propagation::Proceed;
            }
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            // 個人用の操作: EnterはGTK標準の改行、Ctrl＋Enterだけを送信にする。
            if !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                || modifiers.intersects(
                    gtk::gdk::ModifierType::SHIFT_MASK
                        | gtk::gdk::ModifierType::ALT_MASK
                        | gtk::gdk::ModifierType::SUPER_MASK,
                )
            {
                return glib::Propagation::Proceed;
            }
            // 変換の確定を優先し、IMEが処理したCtrl＋Enterでは送信しない。
            if controller
                .current_event()
                .is_some_and(|event| view.im_context_filter_keypress(event))
            {
                return glib::Propagation::Stop;
            }
            send();
            glib::Propagation::Stop
        });
        self.view.add_controller(keys);
    }
}
