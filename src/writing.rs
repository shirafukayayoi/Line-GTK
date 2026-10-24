use crate::private_file::write_private;
use crate::protocol::ReplyInfo;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Draft {
    pub account: String,
    pub chat_mid: String,
    pub text: String,
    pub quote: Option<ReplyInfo>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Sending,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Outgoing {
    pub id: u64,
    pub draft: Draft,
    pub chat_name: String,
    pub status: Delivery,
}

#[derive(Default, Clone, Serialize, Deserialize, PartialEq)]
struct Data {
    version: u8,
    next_id: u64,
    drafts: Vec<Draft>,
    outgoing: Vec<Outgoing>,
}

pub struct Store {
    path: PathBuf,
    data: Data,
    healthy: bool,
}

fn valid(draft: &Draft) -> bool {
    draft.account.starts_with('u')
        && draft.account.len() <= 128
        && draft.chat_mid.starts_with(['u', 'c', 'r'])
        && draft.chat_mid.len() <= 128
        && draft.text.len() <= 1024 * 1024
        && draft.quote.as_ref().is_none_or(|quote| {
            quote.message_id.parse::<u128>().is_ok_and(|id| id > 0)
                && quote.text.len() <= 8192
                && quote.from.len() <= 128
                && quote.content_type.len() <= 64
        })
}

impl Store {
    pub fn load(directory: &Path) -> Result<Self> {
        let path = directory.join("writing-state.json");
        let data = match fs::metadata(&path) {
            Ok(metadata) => {
                ensure!(
                    metadata.len() <= 8 * 1024 * 1024,
                    "下書きファイルが大きすぎます"
                );
                serde_json::from_slice::<Data>(&fs::read(&path)?)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Data {
                version: 1,
                next_id: 1,
                ..Default::default()
            },
            Err(error) => return Err(error.into()),
        };
        let mut drafts = HashSet::new();
        let mut ids = HashSet::new();
        ensure!(
            data.version == 1
                && data.next_id > 0
                && data.drafts.len() <= 1000
                && data.outgoing.len() <= 1000
                && data
                    .drafts
                    .iter()
                    .all(|draft| valid(draft) && drafts.insert((&draft.account, &draft.chat_mid)))
                && data.outgoing.iter().all(|item| item.id > 0
                    && item.id < data.next_id
                    && ids.insert(item.id)
                    && valid(&item.draft)
                    && !item.draft.text.trim().is_empty()
                    && item.chat_name.len() <= 1024),
            "下書きファイルの形式が不正です"
        );
        let mut store = Self {
            path,
            data,
            healthy: true,
        };
        let mut recovered = store.data.clone();
        for item in &mut recovered.outgoing {
            if item.status == Delivery::Sending {
                item.status = Delivery::Unknown;
            }
        }
        store.commit(recovered)?;
        Ok(store)
    }

    pub fn healthy(&self) -> bool {
        self.healthy
    }

    pub fn draft(&self, account: &str, chat_mid: &str) -> Option<Draft> {
        self.data
            .drafts
            .iter()
            .find(|draft| draft.account == account && draft.chat_mid == chat_mid)
            .cloned()
    }

    pub fn outgoing(&self, account: &str) -> Vec<Outgoing> {
        self.data
            .outgoing
            .iter()
            .filter(|item| item.draft.account == account)
            .cloned()
            .collect()
    }

    pub fn save_draft(&mut self, draft: Draft) -> Result<()> {
        ensure!(valid(&draft), "下書きが不正です");
        let mut next = self.data.clone();
        next.drafts
            .retain(|old| old.account != draft.account || old.chat_mid != draft.chat_mid);
        if !draft.text.is_empty() || draft.quote.is_some() {
            next.drafts.push(draft);
        }
        self.commit(next)
    }

    pub fn start_send(&mut self, draft: Draft, chat_name: String) -> Result<u64> {
        ensure!(
            valid(&draft) && !draft.text.trim().is_empty() && chat_name.len() <= 1024,
            "送信する本文が不正です"
        );
        let mut next = self.data.clone();
        let id = next.next_id;
        next.next_id = id
            .checked_add(1)
            .context("送信記録のIDが上限に達しました")?;
        next.drafts
            .retain(|old| old.account != draft.account || old.chat_mid != draft.chat_mid);
        next.outgoing.push(Outgoing {
            id,
            draft,
            chat_name,
            status: Delivery::Sending,
        });
        // 送信開始前に本文を確実に残す。保存失敗なら通信へ進ませない。
        self.commit(next)?;
        Ok(id)
    }

    pub fn finish_send(&mut self, account: &str, id: u64, failure: Option<Delivery>) -> Result<()> {
        let mut next = self.data.clone();
        let index = next
            .outgoing
            .iter()
            .position(|item| item.id == id && item.draft.account == account)
            .context("送信記録が見つかりません")?;
        if let Some(status) = failure {
            next.outgoing[index].status = status;
        } else {
            next.outgoing.remove(index);
        }
        self.commit(next)
    }

    pub fn restore(&mut self, account: &str, id: u64) -> Result<Draft> {
        let item = self
            .data
            .outgoing
            .iter()
            .find(|item| item.id == id && item.draft.account == account)
            .context("送信記録が見つかりません")?;
        ensure!(item.status != Delivery::Sending, "まだ送信中です");
        let draft = item.draft.clone();
        ensure!(
            self.draft(account, &draft.chat_mid)
                .is_none_or(|old| old == draft),
            "別の下書きがあるため上書きしません"
        );
        let mut next = self.data.clone();
        next.outgoing.retain(|item| item.id != id);
        next.drafts
            .retain(|old| old.account != account || old.chat_mid != draft.chat_mid);
        next.drafts.push(draft.clone());
        self.commit(next)?;
        Ok(draft)
    }

    pub fn remove(&mut self, account: &str, id: u64) -> Result<()> {
        let mut next = self.data.clone();
        ensure!(
            next.outgoing.iter().any(|item| item.id == id
                && item.draft.account == account
                && item.status != Delivery::Sending),
            "送信記録を削除できません"
        );
        next.outgoing.retain(|item| item.id != id);
        self.commit(next)
    }

    fn commit(&mut self, next: Data) -> Result<()> {
        ensure!(self.healthy, "下書きを保存できないため送信を停止しています");
        ensure!(
            next.drafts.len() <= 1000 && next.outgoing.len() <= 1000,
            "保存できる件数が上限に達しました"
        );
        if next == self.data {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(&next)?;
        ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "下書きファイルが大きすぎます"
        );
        if let Err(error) = write_private(&self.path, &bytes) {
            self.healthy = false;
            return Err(error);
        }
        self.data = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    fn fixture() -> (PathBuf, Store) {
        static ID: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "line-gtk-writing-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let store = Store::load(&dir).unwrap();
        (dir, store)
    }
    fn draft(account: &str, chat: &str, text: &str) -> Draft {
        Draft {
            account: account.into(),
            chat_mid: chat.into(),
            text: text.into(),
            quote: None,
        }
    }
    #[test]
    fn drafts_survive_restart_without_crossing_accounts_or_chats() {
        let (dir, mut store) = fixture();
        store
            .save_draft(draft("u-me", "u-a", "改行\n本文🙂"))
            .unwrap();
        store
            .save_draft(draft("u-me", "c-group", "別の文章"))
            .unwrap();
        store
            .save_draft(draft("u-other", "u-a", "別アカウント"))
            .unwrap();
        let mut store = Store::load(&dir).unwrap();
        assert_eq!(store.draft("u-me", "u-a").unwrap().text, "改行\n本文🙂");
        store.save_draft(draft("u-me", "u-a", "")).unwrap();
        assert!(store.draft("u-me", "u-a").is_none());
        assert_eq!(store.draft("u-other", "u-a").unwrap().text, "別アカウント");
        assert!(store.draft("u-me", "c-group").is_some());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn sending_is_saved_first_and_a_crash_never_discards_or_resends_the_text() {
        let (dir, mut store) = fixture();
        let body = draft("u-me", "u-a", "失いたくない本文");
        store.save_draft(body.clone()).unwrap();
        let id = store.start_send(body.clone(), "相手".into()).unwrap();
        assert!(store.draft("u-me", "u-a").is_none());
        let mut store = Store::load(&dir).unwrap();
        assert_eq!(store.outgoing("u-me")[0].status, Delivery::Unknown);
        assert_eq!(store.outgoing("u-me")[0].draft, body);
        assert!(store.outgoing("u-other").is_empty());
        assert!(store.restore("u-other", id).is_err());
        store.restore("u-me", id).unwrap();
        assert_eq!(store.draft("u-me", "u-a").unwrap(), body);
        assert!(store.outgoing("u-me").is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_messages_preserve_quotes_and_cannot_overwrite_new_writing() {
        let (dir, mut store) = fixture();
        let mut body = draft("u-me", "u-a", "返信");
        body.quote = Some(ReplyInfo {
            message_id: "9007199254740993".into(),
            text: "引用元".into(),
            from: "u-a".into(),
            mine: false,
            content_type: "NONE".into(),
        });
        let id = store.start_send(body.clone(), "相手".into()).unwrap();
        assert!(store.remove("u-me", id).is_err());
        store
            .finish_send("u-me", id, Some(Delivery::Failed))
            .unwrap();
        store
            .save_draft(draft("u-me", "u-a", "書き始めた別の文章"))
            .unwrap();
        assert!(store.restore("u-me", id).is_err());
        assert_eq!(store.outgoing("u-me")[0].draft.quote, body.quote);
        store.finish_send("u-me", id, None).unwrap();
        assert!(store.outgoing("u-me").is_empty());
        assert_eq!(
            store.draft("u-me", "u-a").unwrap().text,
            "書き始めた別の文章"
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn corruption_and_write_failure_do_not_erase_the_saved_copy() {
        let (dir, mut store) = fixture();
        store
            .save_draft(draft("u-me", "u-a", "元の下書き"))
            .unwrap();
        let path = dir.join("writing-state.json");
        let before = fs::read(&path).unwrap();
        let temporary = dir.join(format!(".writing-state-{}.tmp", std::process::id()));
        fs::create_dir(&temporary).unwrap();
        assert!(
            store
                .start_send(draft("u-me", "u-a", "本文"), "相手".into())
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!store.healthy());
        fs::remove_dir(temporary).unwrap();
        fs::write(&path, "broken").unwrap();
        assert!(Store::load(&dir).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
        fs::remove_dir_all(dir).unwrap();
    }
}
