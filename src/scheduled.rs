use crate::protocol::ReplyInfo;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const DISPATCH_WINDOW_MS: i64 = 2000;
const MAX_ENTRIES: usize = 1000;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Dispatching,
    Sent,
    Failed,
    Unknown,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub account: String,
    pub chat_mid: String,
    pub chat_name: String,
    pub text: String,
    pub quote: Option<ReplyInfo>,
    pub send_at: i64,
    pub status: Status,
}

#[derive(Clone, Serialize, Deserialize)]
struct Queue {
    version: u8,
    next_id: u64,
    entries: Vec<Entry>,
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            entries: Vec::new(),
        }
    }
}

pub struct Store {
    path: PathBuf,
    queue: Queue,
    poisoned: bool,
}

pub fn valid_content(entry: &Entry) -> bool {
    entry.account.starts_with('u')
        && entry.account.len() <= 128
        && entry.chat_mid.starts_with(['u', 'c', 'r'])
        && entry.chat_mid.len() <= 128
        && entry.chat_name.len() <= 1024
        && !entry.text.trim().is_empty()
        && entry.text.chars().count() <= 10_000
        && entry.send_at > 0
        && entry.quote.as_ref().is_none_or(|quote| {
            quote.message_id.parse::<u64>().is_ok_and(|id| id > 0)
                && quote.text.len() <= 8192
                && quote.from.len() <= 128
                && quote.content_type.len() <= 64
        })
}

impl Store {
    pub fn load(data_dir: &Path, now: i64) -> Result<Self> {
        let path = data_dir.join("scheduled-replies.json");
        let queue: Queue = match fs::metadata(&path) {
            Ok(metadata) => {
                ensure!(
                    metadata.len() <= MAX_FILE_BYTES,
                    "予約ファイルが大きすぎます"
                );
                serde_json::from_slice(&fs::read(&path)?)
                    .context("予約ファイルを読み込めませんでした")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Queue::default(),
            Err(error) => return Err(error.into()),
        };
        let mut ids = HashSet::new();
        ensure!(
            queue.version == 1
                && queue.entries.len() <= MAX_ENTRIES
                && queue.next_id > 0
                && queue.entries.iter().all(|entry| {
                    entry.id > 0
                        && entry.id < queue.next_id
                        && ids.insert(entry.id)
                        && valid_content(entry)
                }),
            "予約ファイルの形式が不正です"
        );
        let mut store = Self {
            path,
            queue,
            poisoned: false,
        };
        let mut recovered = store.queue.clone();
        for entry in &mut recovered.entries {
            if entry.status == Status::Dispatching {
                // 送信直後に終了した可能性があるため、再起動しても再送しない。
                entry.status = Status::Unknown;
            } else if entry.status == Status::Queued && entry.send_at <= now {
                entry.status = Status::Expired;
            }
        }
        if recovered.entries != store.queue.entries {
            store.commit(recovered)?;
        }
        Ok(store)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.queue.entries
    }

    pub fn healthy(&self) -> bool {
        !self.poisoned
    }

    pub fn save_entry(&mut self, mut entry: Entry, now: i64) -> Result<u64> {
        ensure!(
            valid_content(&entry) && entry.send_at > now,
            "本文と未来の日時を指定してください"
        );
        let mut next = self.queue.clone();
        entry.status = Status::Queued;
        if entry.id == 0 {
            ensure!(next.entries.len() < MAX_ENTRIES, "予約一覧がいっぱいです");
            entry.id = next.next_id;
            next.next_id = next
                .next_id
                .checked_add(1)
                .context("予約IDが上限に達しました")?;
            next.entries.push(entry.clone());
        } else {
            let existing = next
                .entries
                .iter_mut()
                .find(|old| {
                    old.id == entry.id
                        && old.account == entry.account
                        && old.status == Status::Queued
                })
                .context("この予約は変更できません")?;
            ensure!(
                existing.chat_mid == entry.chat_mid,
                "送信先は変更できません"
            );
            *existing = entry.clone();
        }
        self.commit(next)?;
        Ok(entry.id)
    }

    pub fn cancel_or_remove(&mut self, account: &str, id: u64) -> Result<()> {
        let mut next = self.queue.clone();
        let index = next
            .entries
            .iter()
            .position(|entry| entry.id == id && entry.account == account)
            .context("予約が見つかりません")?;
        ensure!(
            next.entries[index].status != Status::Dispatching,
            "送信開始後は取り消せません"
        );
        if next.entries[index].status == Status::Queued {
            next.entries[index].status = Status::Cancelled;
        } else {
            next.entries.remove(index);
        }
        self.commit(next)
    }

    // この直前まで連続して実行可能だった場合だけ、指定時刻をまたいで送信する。
    // 起動・復帰・切断後に期限内の予約を拾い直すことはしない。
    pub fn tick(
        &mut self,
        previous: i64,
        now: i64,
        ready_account: Option<&str>,
    ) -> Result<Vec<Entry>> {
        let mut next = self.queue.clone();
        let mut claimed = Vec::new();
        let continuous = now >= previous && now - previous <= 1500;
        for entry in &mut next.entries {
            if entry.status != Status::Queued || entry.send_at > now {
                continue;
            }
            entry.status = if continuous
                && previous < entry.send_at
                && now - entry.send_at <= DISPATCH_WINDOW_MS
                && ready_account == Some(entry.account.as_str())
            {
                Status::Dispatching
            } else {
                Status::Expired
            };
            if entry.status == Status::Dispatching {
                claimed.push(entry.clone());
            }
        }
        if next.entries != self.queue.entries {
            // この保存が成功する前には、呼び出し元へ送信対象を返さない。
            self.commit(next)?;
        }
        Ok(claimed)
    }

    pub fn finish(&mut self, account: &str, id: u64, status: Status) -> Result<()> {
        ensure!(
            matches!(
                status,
                Status::Sent | Status::Failed | Status::Unknown | Status::Expired
            ),
            "予約の完了状態が不正です"
        );
        let mut next = self.queue.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|entry| entry.id == id && entry.account == account)
            .context("予約が見つかりません")?;
        ensure!(
            entry.status == Status::Dispatching,
            "予約は送信中ではありません"
        );
        entry.status = status;
        self.commit(next)
    }

    fn commit(&mut self, next: Queue) -> Result<()> {
        ensure!(
            !self.poisoned,
            "保存に失敗したため予約送信を停止しています。再起動してください"
        );
        let bytes = serde_json::to_vec_pretty(&next)?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "予約ファイルが大きすぎます"
        );
        if let Err(error) = write_private(&self.path, &bytes) {
            // rename後の同期失敗でも、メモリの旧状態から送信し直さない。
            self.poisoned = true;
            return Err(error);
        }
        self.queue = next;
        Ok(())
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().context("予約の保存先がありません")?;
    crate::config::ensure_private_dir(directory)?;
    let temporary = directory.join(format!(".scheduled-replies-{}.tmp", std::process::id()));
    // 異常終了後の同じPIDやシンボリックリンクを上書きしない。
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn parse_local_date(date: &str, hour: u32, minute: u32) -> Result<i64> {
    use chrono::{Local, NaiveDate, TimeZone};
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")?;
    let local = date
        .and_hms_opt(hour, minute, 0)
        .context("日時が不正です")?;
    // 夏時間で存在しない時刻・2通りある時刻は、意図しない時刻へ補正しない。
    Local
        .from_local_datetime(&local)
        .single()
        .map(|time| time.timestamp_millis())
        .context("このPCのタイムゾーンでは一意に指定できない日時です")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fixture() -> (PathBuf, Store) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "line-gtk-schedule-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let store = Store::load(&dir, 1000).unwrap();
        (dir, store)
    }
    fn entry(account: &str, send_at: i64) -> Entry {
        Entry {
            id: 0,
            account: account.into(),
            chat_mid: "c-group".into(),
            chat_name: "グループ".into(),
            text: "改行付き\n予約返信🙂".into(),
            quote: None,
            send_at,
            status: Status::Queued,
        }
    }

    #[test]
    fn claims_once_and_persists_before_dispatch_even_across_restart() {
        let (dir, mut store) = fixture();
        let id = store.save_entry(entry("u-self", 3000), 1000).unwrap();
        assert!(store.tick(1000, 1500, Some("u-self")).unwrap().is_empty());
        assert_eq!(store.tick(2500, 3050, Some("u-self")).unwrap()[0].id, id);
        assert!(store.tick(2500, 3050, Some("u-self")).unwrap().is_empty());
        let mut restarted = Store::load(&dir, 3051).unwrap();
        assert_eq!(restarted.entries()[0].status, Status::Unknown);
        assert!(
            restarted
                .tick(3000, 3500, Some("u-self"))
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sleep_offline_late_start_and_other_accounts_expire_without_sending() {
        for (previous, now, account) in [
            (1000, 10000, Some("u-self")),
            (2500, 3050, None),
            (3001, 3050, Some("u-self")),
            (2500, 3050, Some("u-other")),
        ] {
            let (dir, mut store) = fixture();
            store.save_entry(entry("u-self", 3000), 1000).unwrap();
            assert!(store.tick(previous, now, account).unwrap().is_empty());
            assert_eq!(store.entries()[0].status, Status::Expired);
            fs::remove_dir_all(dir).unwrap();
        }
        let (dir, mut store) = fixture();
        store.save_entry(entry("u-self", 3000), 1000).unwrap();
        let restarted = Store::load(&dir, 3000).unwrap();
        assert_eq!(restarted.entries()[0].status, Status::Expired);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn edit_cancel_and_account_isolation_preserve_text_without_sending() {
        let (dir, mut store) = fixture();
        let id = store.save_entry(entry("u-self", 3000), 1000).unwrap();
        let mut edited = store.entries()[0].clone();
        edited.text = "変更後".into();
        edited.send_at = 4000;
        store.save_entry(edited, 2000).unwrap();
        assert!(store.cancel_or_remove("u-other", id).is_err());
        let reopened = Store::load(&dir, 2500).unwrap();
        assert_eq!(reopened.entries()[0].text, "変更後");
        store.cancel_or_remove("u-self", id).unwrap();
        assert!(store.tick(3500, 4000, Some("u-self")).unwrap().is_empty());
        assert_eq!(store.entries()[0].status, Status::Cancelled);
        store.cancel_or_remove("u-self", id).unwrap();
        assert!(store.entries().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_save_and_corrupt_files_never_allow_dispatch() {
        let (dir, mut store) = fixture();
        store.save_entry(entry("u-self", 3000), 1000).unwrap();
        let temporary = dir.join(format!(".scheduled-replies-{}.tmp", std::process::id()));
        fs::create_dir(&temporary).unwrap();
        assert!(store.tick(2500, 3000, Some("u-self")).is_err());
        fs::remove_dir(temporary).unwrap();
        assert!(store.tick(2500, 3000, Some("u-self")).is_err());
        fs::write(dir.join("scheduled-replies.json"), b"invalid").unwrap();
        assert!(Store::load(&dir, 3001).is_err());
        assert_eq!(
            fs::read(dir.join("scheduled-replies.json")).unwrap(),
            b"invalid"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn result_is_terminal_and_sending_cannot_be_edited_or_cancelled() {
        for status in [
            Status::Sent,
            Status::Failed,
            Status::Unknown,
            Status::Expired,
        ] {
            let (dir, mut store) = fixture();
            let id = store.save_entry(entry("u-self", 3000), 1000).unwrap();
            store.tick(2500, 3000, Some("u-self")).unwrap();
            assert!(store.cancel_or_remove("u-self", id).is_err());
            assert!(store.save_entry(store.entries()[0].clone(), 2000).is_err());
            store.finish("u-self", id, status).unwrap();
            assert_eq!(Store::load(&dir, 3001).unwrap().entries()[0].status, status);
            assert!(store.tick(2500, 3000, Some("u-self")).unwrap().is_empty());
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn invalid_dates_and_empty_text_are_rejected_and_permissions_are_private() {
        let (dir, mut store) = fixture();
        let mut invalid = entry("u-self", 3000);
        invalid.text = "\n ".into();
        assert!(store.save_entry(invalid, 1000).is_err());
        assert!(parse_local_date("2026-02-30", 12, 0).is_err());
        assert!(parse_local_date("2026-10-25", 24, 0).is_err());
        assert!(parse_local_date("2026-10-25", 12, 60).is_err());
        assert!(parse_local_date("2026-10-25", 12, 30).is_ok());
        store.save_entry(entry("u-self", 3000), 1000).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.join("scheduled-replies.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
