use super::*;

const LIMIT: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    account: String,
    chat: String,
    epoch: u64,
    order: u64,
}

#[derive(Clone)]
struct Attachment {
    id: u64,
    target: Target,
    path: PathBuf,
    name: String,
}

#[derive(Default)]
pub(super) struct State {
    epoch: u64,
    next: u64,
    items: Vec<Attachment>,
    approved: VecDeque<Attachment>,
    reading: HashMap<u64, Target>,
    sending: HashMap<u64, Attachment>,
    panel: Option<gtk::Box>,
}

impl State {
    fn capture(&mut self, account: String, chat: String) -> Target {
        self.next += 1;
        let target = Target {
            account,
            chat,
            epoch: self.epoch,
            order: self.next,
        };
        self.reading.insert(target.order, target.clone());
        target
    }

    fn clear(&mut self) -> Vec<PathBuf> {
        self.epoch += 1;
        self.reading.clear();
        std::mem::take(&mut self.items)
            .into_iter()
            .chain(self.approved.drain(..))
            .chain(self.sending.drain().map(|(_, item)| item))
            .map(|item| item.path)
            .collect()
    }

    fn ordered_insert(&mut self, item: Attachment) {
        self.items.push(item);
        self.items.sort_by_key(|item| (item.target.order, item.id));
    }

    fn complete(&mut self, request: u64, ok: bool) -> Option<PathBuf> {
        let item = self.sending.remove(&request)?;
        if ok {
            Some(item.path)
        } else {
            self.ordered_insert(item);
            let remaining: Vec<_> = self.approved.drain(..).collect();
            for item in remaining {
                self.ordered_insert(item);
            }
            None
        }
    }
}

fn current(state: &AppState) -> Option<(String, String)> {
    Some((
        state.self_mid.borrow().clone()?,
        state.current_chat.borrow().clone()?,
    ))
}

fn same_chat(target: &Target, account: &str, chat: &str) -> bool {
    target.account == account && target.chat == chat
}

fn valid(state: &AppState, target: &Target) -> bool {
    *state.session_ready.borrow()
        && state.self_mid.borrow().as_deref() == Some(target.account.as_str())
        && state.attachments.borrow().epoch == target.epoch
}

pub(super) fn capture(state: &AppState) -> Option<Target> {
    if !*state.session_ready.borrow() {
        return None;
    }
    let (account, chat) = current(state)?;
    let target = {
        let mut queue = state.attachments.borrow_mut();
        queue.capture(account, chat)
    };
    refresh(state);
    Some(target)
}

pub(super) fn finish_paste(state: &AppState, target: &Target) {
    state.attachments.borrow_mut().reading.remove(&target.order);
    refresh(state);
}

pub(super) fn reading(state: &AppState) -> bool {
    current(state).is_some_and(|(account, chat)| {
        state
            .attachments
            .borrow()
            .reading
            .values()
            .any(|target| same_chat(target, &account, &chat))
    })
}

pub(super) fn enqueue(state: &AppState, target: &Target, path: PathBuf, temporary: bool) {
    let result = (|| -> anyhow::Result<()> {
        if !valid(state, target) {
            return Ok(());
        }
        let id = {
            let mut queue = state.attachments.borrow_mut();
            let count = queue
                .items
                .iter()
                .chain(queue.approved.iter())
                .chain(queue.sending.values())
                .filter(|item| same_chat(&item.target, &target.account, &target.chat))
                .count();
            anyhow::ensure!(count < LIMIT, "{}", crate::i18n::t("attachment_limit"));
            queue.next += 1;
            queue.next
        };
        anyhow::ensure!(path.is_file(), "{}", crate::i18n::t("attachment_missing"));
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let directory = state.data_dir.join("cache/attachments");
        crate::config::ensure_private_dir(&directory)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let directory = directory.join(format!("queued-{stamp}-{id}"));
        crate::config::ensure_private_dir(&directory)?;
        let cached = directory.join(path.file_name().unwrap_or_default());
        // 元のファイルを後で変更・削除しても、確認した内容を送れるように複製する。
        if let Err(error) = std::fs::copy(&path, &cached) {
            remove_cached(&cached);
            return Err(error.into());
        }
        state.attachments.borrow_mut().ordered_insert(Attachment {
            id,
            target: target.clone(),
            path: cached,
            name,
        });
        Ok(())
    })();
    if temporary {
        let _ = std::fs::remove_file(path);
    }
    if let Err(error) = result {
        toast(
            state,
            &crate::i18n::tf("attachment_failed", &[("error", &error.to_string())]),
        );
    }
    refresh(state);
}

pub(super) fn send(state: &AppState) {
    let Some((account, chat)) = current(state) else {
        return;
    };
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    if reading(state) {
        toast(state, &crate::i18n::t("attachment_reading"));
        return;
    }
    if contact_blocks::blocked(state, &chat) == Some(true) {
        toast(state, &crate::i18n::t("contact_blocked_send"));
        return;
    }
    let items: Vec<_> = state
        .attachments
        .borrow()
        .items
        .iter()
        .filter(|item| same_chat(&item.target, &account, &chat))
        .cloned()
        .collect();
    // この操作で選ばれた分だけを確定する。後から貼り付けた写真は含めない。
    {
        let mut queue = state.attachments.borrow_mut();
        for item in items {
            queue.items.retain(|pending| pending.id != item.id);
            queue.approved.push_back(item);
        }
    }
    pump(state);
    refresh(state);
}

fn pump(state: &AppState) {
    let item = {
        let mut queue = state.attachments.borrow_mut();
        if !queue.sending.is_empty() {
            return;
        }
        queue.approved.pop_front()
    };
    let Some(item) = item else { return };
    if !valid(state, &item.target) {
        remove_cached(&item.path);
        reset(state);
        return;
    }
    let request = if contact_blocks::blocked(state, &item.target.chat) == Some(true) {
        toast(state, &crate::i18n::t("contact_blocked_send"));
        None
    } else {
        send_media_to(state, &item.target.chat, item.path.clone())
    };
    let mut queue = state.attachments.borrow_mut();
    if let Some(request) = request {
        queue.sending.insert(request, item);
    } else {
        queue.ordered_insert(item);
        let remaining: Vec<_> = queue.approved.drain(..).collect();
        for item in remaining {
            queue.ordered_insert(item);
        }
    }
}

pub(super) fn complete(state: &AppState, request: u64, ok: bool) {
    if !state.attachments.borrow().sending.contains_key(&request) {
        return;
    }
    let path = state.attachments.borrow_mut().complete(request, ok);
    if let Some(path) = path {
        remove_cached(&path);
    }
    // 先行する送信の成功後に、手動で確定済みの分だけを1件ずつ送る。
    if ok {
        pump(state);
    }
    refresh(state);
}

fn remove(state: &AppState, ids: &[u64]) {
    let paths: Vec<_> = {
        let mut queue = state.attachments.borrow_mut();
        let paths = queue
            .items
            .iter()
            .filter(|item| ids.contains(&item.id))
            .map(|item| item.path.clone())
            .collect();
        queue.items.retain(|item| !ids.contains(&item.id));
        paths
    };
    for path in paths {
        remove_cached(&path);
    }
    refresh(state);
    state.composer.grab_focus();
}

pub(super) fn hide(state: &AppState) {
    if let Some(panel) = &state.attachments.borrow().panel {
        panel.set_visible(false);
    }
}

pub(super) fn reset(state: &AppState) {
    let paths = state.attachments.borrow_mut().clear();
    for path in paths {
        remove_cached(&path);
    }
    refresh(state);
}

pub(super) fn wire(state: &AppState) {
    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .visible(false)
        .build();
    state
        .conversation
        .insert_child_after(&panel, Some(&state.reply_preview));
    state.attachments.borrow_mut().panel = Some(panel);
    let s = state.clone();
    state.app.connect_shutdown(move |_| reset(&s));
}

fn remove_cached(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    if let Some(directory) = path.parent()
        && directory
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("queued-"))
    {
        let _ = std::fs::remove_dir(directory);
    }
}

pub(super) fn refresh(state: &AppState) {
    let Some(panel) = state.attachments.borrow().panel.clone() else {
        return;
    };
    while let Some(child) = panel.first_child() {
        panel.remove(&child);
    }
    let Some((account, chat)) = current(state) else {
        panel.set_visible(false);
        return;
    };
    let items: Vec<_> = state
        .attachments
        .borrow()
        .items
        .iter()
        .filter(|item| same_chat(&item.target, &account, &chat))
        .cloned()
        .collect();
    let loading = reading(state);
    panel.set_visible(!items.is_empty() || loading);
    if items.is_empty() && !loading {
        return;
    }
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.append(
        &gtk::Label::builder()
            .label(if loading {
                crate::i18n::t("attachment_reading")
            } else {
                crate::i18n::tf("attachment_queue", &[("n", &items.len().to_string())])
            })
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["caption", "heading"])
            .tooltip_text(crate::i18n::t("attachment_send_hint"))
            .build(),
    );
    if !items.is_empty() {
        let clear = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .focus_on_click(false)
            .tooltip_text(crate::i18n::t("attachment_clear"))
            .css_classes(["flat", "circular"])
            .build();
        let ids: Vec<_> = items.iter().map(|item| item.id).collect();
        let s = state.clone();
        clear.connect_clicked(move |_| remove(&s, &ids));
        header.append(&clear);
    }
    panel.append(&header);
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for item in items {
        let card = gtk::Box::builder().spacing(6).css_classes(["card"]).build();
        if matches!(guess_media_o_type(&item.path), "image" | "gif") {
            let picture = gtk::Picture::builder()
                .width_request(64)
                .height_request(64)
                .can_shrink(true)
                .content_fit(gtk::ContentFit::Contain)
                .alternative_text(&item.name)
                .build();
            attach_texture_async(
                picture.clone(),
                item.path.to_string_lossy().into_owned(),
                64,
            );
            let preview = gtk::Button::builder()
                .child(&picture)
                .focus_on_click(false)
                .tooltip_text(crate::i18n::tf(
                    "attachment_preview",
                    &[("name", &item.name)],
                ))
                .css_classes(["flat"])
                .build();
            let s = state.clone();
            let path = item.path.to_string_lossy().into_owned();
            let name = item.name.clone();
            preview.connect_clicked(move |_| open_attachment_preview(&s, &path, &name));
            card.append(&preview);
        } else {
            card.append(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
        }
        card.append(
            &gtk::Label::builder()
                .label(&item.name)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .max_width_chars(14)
                .tooltip_text(&item.name)
                .build(),
        );
        let cancel = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .focus_on_click(false)
            .tooltip_text(crate::i18n::tf(
                "attachment_remove",
                &[("name", &item.name)],
            ))
            .css_classes(["flat", "circular"])
            .build();
        let s = state.clone();
        let id = item.id;
        cancel.connect_clicked(move |_| remove(&s, &[id]));
        card.append(&cancel);
        strip.append(&card);
    }
    panel.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&strip)
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64, target: Target) -> Attachment {
        Attachment {
            id,
            target,
            path: PathBuf::from(format!("/unused/{id}.png")),
            name: format!("{id}.png"),
        }
    }

    #[test]
    fn late_clipboard_results_and_failed_uploads_keep_paste_order() {
        let mut state = State::default();
        let first = state.capture("account".into(), "chat".into());
        let second = state.capture("account".into(), "chat".into());
        state.ordered_insert(item(4, second));
        state.ordered_insert(item(3, first));
        let upload = state.items.remove(0);
        state.sending.insert(10, upload);
        assert!(state.complete(10, false).is_none());
        assert_eq!(
            state.items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![3, 4]
        );
        state.complete(10, false);
        assert_eq!(state.items.len(), 2);
    }

    #[test]
    fn successful_upload_releases_only_its_queued_copy() {
        let mut state = State::default();
        let target = state.capture("account".into(), "chat".into());
        state.sending.insert(42, item(5, target));
        assert_eq!(
            state.complete(42, true),
            Some(PathBuf::from("/unused/5.png"))
        );
        assert!(state.items.is_empty());
        assert!(state.complete(42, true).is_none());
    }

    #[test]
    fn account_and_chat_must_both_match() {
        let target = State::default().capture("account".into(), "chat".into());
        assert!(same_chat(&target, "account", "chat"));
        assert!(!same_chat(&target, "other-account", "chat"));
        assert!(!same_chat(&target, "account", "other-chat"));
    }

    #[test]
    fn logout_invalidates_reads_and_late_upload_responses() {
        let mut state = State::default();
        let old = state.capture("account".into(), "chat".into());
        state.ordered_insert(item(3, old.clone()));
        state.sending.insert(10, item(4, old.clone()));
        assert_eq!(state.clear().len(), 2);
        let new = state.capture("account".into(), "chat".into());
        assert_ne!(old.epoch, new.epoch);
        assert_ne!(old.order, new.order);
        state.reading.remove(&old.order);
        assert_eq!(state.reading.len(), 1);
        assert!(state.complete(10, false).is_none());
        assert!(state.items.is_empty());
    }

    #[test]
    fn failure_restores_unsent_approved_items_but_does_not_approve_new_pastes() {
        let mut state = State::default();
        let target = state.capture("account".into(), "chat".into());
        state.sending.insert(10, item(2, target.clone()));
        state.approved.push_back(item(3, target.clone()));
        state.ordered_insert(item(4, target));
        assert!(state.complete(10, false).is_none());
        assert!(state.sending.is_empty());
        assert!(state.approved.is_empty());
        assert_eq!(
            state.items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }
}
