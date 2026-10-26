use super::*;

#[derive(Default)]
pub(super) struct State {
    generation: u64,
    ui: Option<HoverUi>,
}

#[derive(Clone)]
struct HoverUi {
    frame: gtk::Frame,
    motion: gtk::EventControllerMotion,
    anchor: glib::WeakRef<gtk::Label>,
    title: gtk::Label,
    list: gtk::Box,
    rows: Rc<RefCell<Vec<(String, gtk::Label)>>>,
    account: String,
    chat: String,
    message: String,
}

pub(super) fn wire(state: &AppState) {
    let s = state.clone();
    state
        .reader_overlay
        .connect_get_child_position(move |overlay, child| {
            let ui = s.reader_hover.borrow().ui.clone()?;
            if child != ui.frame.upcast_ref::<gtk::Widget>() {
                return None;
            }
            let anchor = ui.anchor.upgrade()?;
            let bounds = anchor.compute_bounds(overlay)?;
            let width = 264.min((overlay.width() - 16).max(1));
            let (_, height, _, _) = child.measure(gtk::Orientation::Vertical, width);
            Some(placement(
                (overlay.width(), overlay.height()),
                (
                    bounds.x() as i32,
                    bounds.y() as i32,
                    bounds.width() as i32,
                    bounds.height() as i32,
                ),
                (width, height),
            ))
        });
}

fn placement(view: (i32, i32), anchor: (i32, i32, i32, i32), size: (i32, i32)) -> gdk::Rectangle {
    let (view_width, view_height) = view;
    let (x, y, anchor_width, anchor_height) = anchor;
    let (width, height) = size;
    let width = width.min((view_width - 16).max(1));
    let height = height.min((view_height - 16).max(1));
    let left = (x + anchor_width - width).clamp(8, (view_width - width - 8).max(8));
    let top = if y - height - 6 >= 8 {
        y - height - 6
    } else {
        y + anchor_height + 6
    };
    gdk::Rectangle::new(
        left,
        top.clamp(8, (view_height - height - 8).max(8)),
        width,
        height,
    )
}

fn advance(state: &AppState) -> u64 {
    let mut hover = state.reader_hover.borrow_mut();
    hover.generation = hover.generation.wrapping_add(1);
    hover.generation
}

pub(super) fn close(state: &AppState) {
    advance(state);
    let ui = state.reader_hover.borrow_mut().ui.take();
    if let Some(ui) = ui {
        state.reader_overlay.remove_overlay(&ui.frame);
    }
}

fn hide_later(state: &AppState) {
    let generation = advance(state);
    let s = state.clone();
    // 表示元と一覧の間の隙間を通過できるよう、少し待ってから閉じる。
    glib::timeout_add_local_once(std::time::Duration::from_millis(220), move || {
        if s.reader_hover.borrow().generation == generation {
            let inside = s
                .reader_hover
                .borrow()
                .ui
                .as_ref()
                .is_some_and(|ui| ui.motion.contains_pointer());
            if !inside {
                close(&s);
            }
        }
    });
}

fn current(state: &AppState, frame: &gtk::Frame) -> bool {
    state
        .reader_hover
        .borrow()
        .ui
        .as_ref()
        .is_some_and(|ui| ui.frame == *frame)
}

pub(super) fn attach(state: &AppState, label: &gtk::Label, message: &str) {
    if !message_senders::is_group(state) || message.starts_with("pending-") {
        return;
    }
    label.set_has_tooltip(false);
    label.set_can_target(true);
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let Some(chat) = state.current_chat.borrow().clone() else {
        return;
    };
    let motion = gtk::EventControllerMotion::new();
    let s = state.clone();
    let weak = label.downgrade();
    let message = message.to_owned();
    motion.connect_enter(move |_, _, _| {
        let same = s
            .reader_hover
            .borrow()
            .ui
            .as_ref()
            .is_some_and(|ui| ui.anchor.upgrade() == weak.upgrade());
        if !same {
            close(&s);
        }
        let generation = advance(&s);
        let s = s.clone();
        let weak = weak.clone();
        let account = account.clone();
        let chat = chat.clone();
        let message = message.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
            if s.reader_hover.borrow().generation == generation
                && let Some(label) = weak.upgrade()
            {
                show(&s, &label, &account, &chat, &message);
            }
        });
    });
    let s = state.clone();
    motion.connect_leave(move |_| hide_later(&s));
    label.add_controller(motion);
    let s = state.clone();
    label.connect_unrealize(move |label| {
        let active = s
            .reader_hover
            .borrow()
            .ui
            .as_ref()
            .is_some_and(|ui| ui.anchor.upgrade().as_ref() == Some(label));
        if active {
            close(&s);
        }
    });
}

fn show(state: &AppState, anchor: &gtk::Label, account: &str, chat: &str, message: &str) {
    if !*state.session_ready.borrow()
        || state.self_mid.borrow().as_deref() != Some(account)
        || state.current_chat.borrow().as_deref() != Some(chat)
        || !anchor.is_mapped()
        || state.receipt_slots.borrow().get(message) != Some(anchor)
        || state.peer_reads.borrow().count(chat, message, account) == 0
    {
        return;
    }
    let same = state
        .reader_hover
        .borrow()
        .ui
        .as_ref()
        .is_some_and(|ui| ui.anchor.upgrade().as_ref() == Some(anchor));
    if same {
        refresh(state);
        return;
    }
    close(state);
    let title = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .max_content_height(260)
        .propagate_natural_height(true)
        .build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    body.append(&title);
    body.append(&scroll);
    let frame = gtk::Frame::builder()
        .child(&body)
        .css_classes(["line-readers-popup"])
        .build();
    let motion = gtk::EventControllerMotion::new();
    let s = state.clone();
    let weak = frame.downgrade();
    motion.connect_enter(move |_, _, _| {
        if let Some(frame) = weak.upgrade()
            && current(&s, &frame)
        {
            advance(&s);
        }
    });
    let s = state.clone();
    let weak = frame.downgrade();
    motion.connect_leave(move |_| {
        if let Some(frame) = weak.upgrade()
            && current(&s, &frame)
        {
            hide_later(&s);
        }
    });
    frame.add_controller(motion.clone());
    state.reader_hover.borrow_mut().ui = Some(HoverUi {
        frame: frame.clone(),
        motion,
        anchor: anchor.downgrade(),
        title,
        list,
        rows: Rc::new(RefCell::new(Vec::new())),
        account: account.into(),
        chat: chat.into(),
        message: message.into(),
    });
    state.reader_overlay.add_overlay(&frame);
    state.reader_overlay.set_clip_overlay(&frame, true);
    refresh(state);
}

pub(super) fn refresh(state: &AppState) {
    let Some(ui) = state.reader_hover.borrow().ui.clone() else {
        return;
    };
    if !*state.session_ready.borrow()
        || state.self_mid.borrow().as_deref() != Some(ui.account.as_str())
        || state.current_chat.borrow().as_deref() != Some(ui.chat.as_str())
        || ui.anchor.upgrade().is_none_or(|anchor| !anchor.is_mapped())
    {
        close(state);
        return;
    }
    let readers = state
        .peer_reads
        .borrow()
        .readers(&ui.chat, &ui.message, &ui.account);
    if readers.is_empty() {
        close(state);
        return;
    }
    let title = crate::i18n::tf(
        "receipt_readers_title",
        &[("n", &readers.len().to_string())],
    );
    ui.title.set_text(&title);
    ui.frame
        .update_property(&[gtk::accessible::Property::Label(&title)]);
    let same = ui
        .rows
        .borrow()
        .iter()
        .map(|(mid, _)| mid)
        .eq(readers.iter());
    if !same {
        while let Some(child) = ui.list.first_child() {
            ui.list.remove(&child);
        }
        let mut rows = ui.rows.borrow_mut();
        rows.clear();
        for mid in readers {
            let label = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .max_width_chars(28)
                .build();
            ui.list.append(&label);
            rows.push((mid, label));
        }
    }
    // 名前だけの更新では行を作り直さず、一覧のスクロール位置を保つ。
    for (mid, label) in ui.rows.borrow().iter() {
        let name = message_senders::known_name(state, mid).unwrap_or_else(|| {
            let suffix: String = mid
                .chars()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            crate::i18n::tf("receipt_reader_unknown", &[("id", &suffix)])
        });
        label.set_text(&name);
    }
    state.reader_overlay.queue_allocate();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_stays_inside_the_conversation_at_all_edges() {
        for (x, y) in [(0, 0), (320, 0), (0, 550), (320, 550)] {
            let rect = placement((360, 600), (x, y, 40, 20), (264, 330));
            assert!(rect.x() >= 8 && rect.y() >= 8);
            assert!(rect.x() + rect.width() <= 352);
            assert!(rect.y() + rect.height() <= 592);
        }
        let compact = placement((180, 140), (170, 130, 10, 10), (264, 330));
        assert_eq!(compact.width(), 164);
        assert_eq!(compact.height(), 124);
    }
}
