//! Move browser installations or profile data before changing their configured roots.

use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Stage selected application subdirectories at another storage root. Call
/// `remove_source_storage` only after saving the new directory settings.
pub fn copy_storage(source_root: &Path, target_root: &Path, names: &[&str]) -> Result<()> {
    fs::create_dir_all(target_root)
        .with_context(|| format!("创建目标目录 {}", target_root.display()))?;
    let source_root = source_root.canonicalize()?;
    let target_root = target_root.canonicalize()?;
    if source_root == target_root {
        return Ok(());
    }
    for name in names {
        let source = source_root.join(name);
        if target_root.starts_with(&source) {
            bail!("目标目录不能位于正在搬迁的 {name} 目录内");
        }
        if fs::symlink_metadata(target_root.join(name)).is_ok() {
            bail!("目标目录已有 {name}，为避免覆盖，请选择其他目录");
        }
    }

    let staging = target_root.join(format!(".cazer-copy-{:016x}", rand::random::<u64>()));
    fs::create_dir(&staging)?;
    let mut promoted = Vec::new();
    let result = (|| -> Result<()> {
        for name in names {
            let source = source_root.join(name);
            if source.exists() {
                copy_tree(&source, &staging.join(name))?;
                verify_tree(&source, &staging.join(name))?;
            }
        }
        for name in names {
            let copied = staging.join(name);
            if copied.exists() {
                if fs::symlink_metadata(target_root.join(name)).is_ok() {
                    bail!("目标目录已有 {name}，为避免覆盖，请选择其他目录");
                }
                fs::rename(&copied, target_root.join(name))
                    .with_context(|| format!("完成 {name} 目录搬迁"))?;
                promoted.push(*name);
            }
        }
        Ok(())
    })();
    if result.is_err() {
        for name in promoted {
            let target = target_root.join(name);
            if fs::symlink_metadata(&target).is_ok() {
                let _ = remove_path(&target);
            }
        }
    }
    let _ = fs::remove_dir_all(staging);
    result
}

/// Remove the old storage after the new copy has been verified and activated.
pub fn remove_source_storage(source_root: &Path, target_root: &Path, names: &[&str]) -> Result<()> {
    let source_root = source_root.canonicalize()?;
    let target_root = target_root.canonicalize()?;
    if source_root == target_root {
        return Ok(());
    }
    for name in names {
        let source = source_root.join(name);
        if fs::symlink_metadata(&source).is_ok() {
            remove_path(&source).with_context(|| format!("清理旧目录 {}", source.display()))?;
        }
    }
    let _ = fs::remove_dir(source_root);
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        copy_symlink(source, target)?;
    } else if metadata.is_dir() {
        fs::create_dir(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
        fs::set_permissions(target, metadata.permissions())?;
    } else if metadata.is_file() {
        fs::copy(source, target)?;
    } else {
        bail!("不支持搬迁特殊文件：{}", source.display());
    }
    Ok(())
}

fn verify_tree(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    let copied = fs::symlink_metadata(target)?;
    if metadata.file_type().is_symlink() {
        if !copied.file_type().is_symlink() || fs::read_link(source)? != fs::read_link(target)? {
            bail!("符号链接核对失败：{}", source.display());
        }
    } else if metadata.is_dir() {
        if !copied.is_dir() {
            bail!("目录核对失败：{}", source.display());
        }
        let mut source_entries: Vec<_> = fs::read_dir(source)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<_>>()?;
        let mut target_entries: Vec<_> = fs::read_dir(target)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<_>>()?;
        source_entries.sort();
        target_entries.sort();
        if source_entries != target_entries {
            bail!("目录内容核对失败：{}", source.display());
        }
        for name in source_entries {
            verify_tree(&source.join(&name), &target.join(name))?;
        }
    } else if metadata.is_file() {
        if !copied.is_file() || metadata.len() != copied.len() || !files_equal(source, target)? {
            bail!("文件核对失败：{}", source.display());
        }
    } else {
        bail!("不支持核对特殊文件：{}", source.display());
    }
    Ok(())
}

fn files_equal(left: &Path, right: &Path) -> Result<bool> {
    let mut left = fs::File::open(left)?;
    let mut right = fs::File::open(right)?;
    let mut a = [0_u8; 64 * 1024];
    let mut b = [0_u8; 64 * 1024];
    loop {
        let count = left.read(&mut a)?;
        if count == 0 {
            return Ok(right.read(&mut b)? == 0);
        }
        right.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
    }
}

#[cfg(unix)]
fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(fs::read_link(source)?, target)?;
    Ok(())
}

#[cfg(windows)]
fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    let link = fs::read_link(source)?;
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(link, target)
            .context("Windows 无法复制目录符号链接；请启用开发者模式或以管理员运行")?;
    } else {
        std::os::windows::fs::symlink_file(link, target)
            .context("Windows 无法复制文件符号链接；请启用开发者模式或以管理员运行")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_and_verifies_data() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        fs::create_dir_all(old.join("profiles/alice")).unwrap();
        fs::write(old.join("profiles/alice/data"), b"profile data").unwrap();
        copy_storage(&old, &new, &["profiles"]).unwrap();
        assert_eq!(
            fs::read(new.join("profiles/alice/data")).unwrap(),
            b"profile data"
        );
        assert!(old.join("profiles/alice/data").exists());
        remove_source_storage(&old, &new, &["profiles"]).unwrap();
        assert!(!old.join("profiles").exists());
    }

    #[test]
    fn refuses_to_overwrite_existing_data() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        fs::create_dir_all(old.join("profiles")).unwrap();
        fs::create_dir_all(new.join("profiles")).unwrap();
        assert!(copy_storage(&old, &new, &["profiles"]).is_err());
        assert!(old.join("profiles").exists());
    }
}
