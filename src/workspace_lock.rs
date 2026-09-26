//! Coordinate ordinary CLI/UI use with storage-directory relocation.

use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use fs2::FileExt;

pub struct WorkspaceLock {
    file: Mutex<File>,
}

impl WorkspaceLock {
    pub fn shared(program_root: &Path) -> Result<Self> {
        fs::create_dir_all(program_root)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(program_root.join("workspace.lock"))?;
        file.try_lock_shared()
            .context("数据目录正在搬迁，请稍后重试")?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    /// Temporarily replace this process's shared lock with an exclusive lock.
    /// Other CLI/UI processes and launched browsers hold shared locks for their
    /// entire lifetime, so a relocation cannot delete data they are using.
    pub fn with_exclusive<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        let file = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("目录锁已损坏"))?;
        FileExt::unlock(&*file)?;
        if let Err(error) = file.try_lock_exclusive() {
            file.lock_shared()?;
            bail!("其他程序或浏览器正在使用数据目录：{error}");
        }
        let result = action();
        FileExt::unlock(&*file)?;
        file.lock_shared()?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_move_refuses_another_active_process() {
        let temp = tempfile::tempdir().unwrap();
        let first = WorkspaceLock::shared(temp.path()).unwrap();
        let second = WorkspaceLock::shared(temp.path()).unwrap();
        assert!(first.with_exclusive(|| Ok(())).is_err());
        drop(second);
        first.with_exclusive(|| Ok(())).unwrap();
    }
}
