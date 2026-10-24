use super::*;

#[derive(Default)]
pub(super) struct History {
    cursor: Option<serde_json::Value>,
    ready: bool,
    has_more: bool,
    loading: Option<u64>,
    failed: bool,
    inserting: bool,
    pub loaded: bool,
}

pub(super) fn reset(state: &AppState) {
    *state.history.borrow_mut() = History::default();
    refresh(state);
}

pub(super) fn refresh(state: &AppState) {
    let history = state.history.borrow();
    let key = if history.loading.is_some() {
        "history_loading"
    } else if history.failed {
        "history_retry"
    } else if history.has_more {
        "history_load_older"
    } else {
        "history_end"
    };
    state.history_btn.set_label(&crate::i18n::t(key));
    state.history_btn.set_visible(history.ready);
    state
        .history_btn
        .set_sensitive(history.has_more && history.loading.is_none());
}

pub(super) fn snapshot(state: &AppState, result: &serde_json::Value) {
    let mut history = state.history.borrow_mut();
    if !history.loaded {
        history.cursor = result
            .get("historyCursor")
            .filter(|v| !v.is_null())
            .cloned();
        history.has_more = result
            .get("hasMore")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    }
    history.ready = true;
    drop(history);
    refresh(state);
}

fn failed(state: &AppState) {
    let mut history = state.history.borrow_mut();
    history.loading = None;
    history.failed = true;
    drop(history);
    refresh(state);
}

pub(super) fn request(state: &AppState) {
    let history = state.history.borrow();
    if !history.ready
        || !history.has_more
        || history.loading.is_some()
        || history.inserting
        || !*state.session_ready.borrow()
        || resume_sync::in_progress(state)
        || state.seen_msg_ids.borrow().len() < state.reply_messages.borrow().len()
    {
        return;
    }
    let cursor = history.cursor.clone();
    drop(history);
    let Some(mid) = state.current_chat.borrow().clone() else {
        return;
    };
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    match state.sidecar.request(
        "fetch_older_messages",
        Some(serde_json::json!({"chatMid": mid, "cursor":cursor,"limit":40})),
    ) {
        Ok(id) => {
            let mut history = state.history.borrow_mut();
            history.loading = Some(id);
            history.failed = false;
            drop(history);
            state.pending.borrow_mut().insert(
                id,
                Pending::OlderMessages {
                    chat_mid: mid,
                    account,
                },
            );
            refresh(state);
            let s = state.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(35), move || {
                if s.history.borrow().loading == Some(id) {
                    s.pending.borrow_mut().remove(&id);
                    failed(&s);
                }
            });
        }
        Err(_) => failed(state),
    }
}

pub(super) fn response(
    state: &AppState,
    id: u64,
    mid: &str,
    account: &str,
    ok: bool,
    result: &serde_json::Value,
) {
    if state.history.borrow().loading != Some(id)
        || state.current_chat.borrow().as_deref() != Some(mid)
        || state.self_mid.borrow().as_deref() != Some(account)
        || !*state.session_ready.borrow()
    {
        return;
    }
    if !ok {
        failed(state);
        return;
    }
    let Some(messages) = result
        .get("messages")
        .cloned()
        .and_then(|v| serde_json::from_value::<Vec<MessageInfo>>(v).ok())
    else {
        failed(state);
        return;
    };
    let mut messages: Vec<_> = messages
        .into_iter()
        .filter(|m| !state.reply_messages.borrow().contains_key(&m.id))
        .collect();
    messages.sort_by_key(|m| m.created_time);
    state.history.borrow_mut().inserting = true;
    if !messages.is_empty() {
        state.history.borrow_mut().loaded = true;
        *state.stick_bottom.borrow_mut() = false;
        *state.scroll_pin_gen.borrow_mut() += 1;
        *state.scroll_pinning.borrow_mut() = false;
        let mut existing: Vec<_> = state.reply_messages.borrow().values().cloned().collect();
        existing.sort_by_key(|message| message.created_time);
        let last_day = state.last_msg_day.borrow_mut().take();
        // 古い保存済みイベントが混ざっていても、本文を時刻順の位置へ追加する。
        let mut start = 0;
        while start < messages.len() {
            let next_index = existing
                .partition_point(|message| message.created_time <= messages[start].created_time);
            let next_time = existing.get(next_index).map(|message| message.created_time);
            let mut end = start + 1;
            while end < messages.len()
                && next_time.is_none_or(|time| messages[end].created_time < time)
            {
                end += 1;
            }
            let next = existing
                .get(next_index)
                .and_then(|message| state.message_rows.borrow().get(&message.id).cloned());
            let new_day = day_key(messages[end - 1].created_time);
            if next_time.map(day_key).as_ref() == Some(&new_day)
                && let Some(row) = next
                    .as_ref()
                    .and_then(|row| state.message_list.previous(row.upcast_ref()))
                && row.has_css_class("line-day-sep-row")
            {
                state.message_list.remove(&row);
            }
            *state.last_msg_day.borrow_mut() = existing
                .get(next_index.wrapping_sub(1))
                .map(|message| day_key(message.created_time));
            state
                .message_list
                .insert_before(next.as_ref().map(|row| row.upcast_ref()), || {
                    for message in &messages[start..end] {
                        append_message(state, message, false);
                    }
                });
            existing.splice(next_index..next_index, messages[start..end].iter().cloned());
            start = end;
        }
        *state.last_msg_day.borrow_mut() = last_day;
        *state.msg_list_fp.borrow_mut() = None;
        state.chat_subtitle.set_text(&crate::i18n::tf(
            "messages_count",
            &[("n", &state.reply_messages.borrow().len().to_string())],
        ));
    }
    let mut history = state.history.borrow_mut();
    history.cursor = result
        .get("historyCursor")
        .filter(|v| !v.is_null())
        .cloned();
    history.has_more = result
        .get("hasMore")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    history.loading = None;
    history.failed = false;
    drop(history);
    refresh(state);
    let s = state.clone();
    glib::idle_add_local_once(move || {
        s.history.borrow_mut().inserting = false;
    });
}

pub(super) fn wire(state: &AppState) {
    let s = state.clone();
    state.history_btn.connect_clicked(move |_| request(&s));
    let s = state.clone();
    state
        .message_scroll
        .vadjustment()
        .connect_value_changed(move |adj| {
            if adj.value() <= 80.0
                && !*s.stick_bottom.borrow()
                && !*s.scroll_pinning.borrow()
                && !s.history.borrow().failed
            {
                request(&s);
            }
        });
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s = state.clone();
    scroll.connect_scroll(move |_, _, dy| {
        if dy < 0.0 && s.message_scroll.vadjustment().value() <= 80.0 && !*s.scroll_pinning.borrow()
        {
            *s.stick_bottom.borrow_mut() = false;
            if !s.history.borrow().failed {
                request(&s);
            }
        }
        glib::Propagation::Proceed
    });
    state.message_scroll.add_controller(scroll);
}
