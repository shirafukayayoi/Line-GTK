use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().context("保存先がありません")?;
    crate::config::ensure_private_dir(directory)?;
    let temporary = directory.join(format!(
        ".{}-{}.tmp",
        path.file_stem()
            .context("ファイル名がありません")?
            .to_string_lossy(),
        std::process::id()
    ));
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
