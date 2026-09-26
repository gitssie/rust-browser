//! The on-disk layout owned by this application. Vendor cache paths are separate.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::settings::DataDirectories;

#[derive(Clone, Debug)]
pub struct AppPaths {
    root: PathBuf,
    browser_root: PathBuf,
    profiles_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ScanArtifacts {
    pub screenshot: PathBuf,
    pub report: PathBuf,
    pub verification: PathBuf,
    pub fields: PathBuf,
}

impl ScanArtifacts {
    pub fn all(self) -> [PathBuf; 4] {
        [self.screenshot, self.report, self.verification, self.fields]
    }
}

impl AppPaths {
    pub fn program_root(override_dir: Option<&Path>) -> Result<PathBuf> {
        let root = match override_dir {
            Some(path) => path.to_path_buf(),
            None => std::env::current_exe()?
                .parent()
                .context("could not determine the program directory")?
                .to_path_buf(),
        };
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        dunce::canonicalize(root).map_err(Into::into)
    }

    /// Resolve the program directory and its configured storage locations.
    pub fn new(override_dir: Option<PathBuf>) -> Result<Self> {
        let root = Self::program_root(override_dir.as_deref())?;
        let directories = DataDirectories::load(&root)
            .with_context(|| format!("程序目录不可写或数据无法读取：{}", root.display()))?;
        Self::with_directories(root, directories)
    }

    /// Use a root already resolved by the caller, without creating files.
    pub fn for_root(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            browser_root: root.clone(),
            profiles_root: root.clone(),
            root,
        }
    }

    pub fn with_directories(root: PathBuf, directories: DataDirectories) -> Result<Self> {
        let browser_root = resolve_storage_root(&root, directories.browser_root)?;
        let profiles_root = resolve_storage_root(&root, directories.profiles_root)?;
        for browser in [
            browser_root.join("browser"),
            browser_root.join("browser-versions"),
            browser_root.join("browser-switch-previous"),
        ] {
            for profile in [profiles_root.join("profiles"), profiles_root.join("trash")] {
                if browser.starts_with(&profile) || profile.starts_with(&browser) {
                    anyhow::bail!("浏览器目录与用户数据目录不能相互嵌套");
                }
            }
        }
        Ok(Self {
            root,
            browser_root,
            profiles_root,
        })
    }

    pub fn browser_root(&self) -> &Path {
        &self.browser_root
    }

    pub fn profiles_root(&self) -> &Path {
        &self.profiles_root
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn database(&self) -> PathBuf {
        self.root.join("browser.sqlite")
    }
    pub fn profiles(&self) -> PathBuf {
        self.profiles_root.join("profiles")
    }
    pub fn profile(&self, id: &str) -> PathBuf {
        self.profiles().join(id)
    }
    pub fn browser(&self) -> PathBuf {
        self.browser_root.join("browser")
    }
    pub fn browser_versions(&self) -> PathBuf {
        self.browser_root.join("browser-versions")
    }
    pub fn browser_version(&self, key: &str) -> PathBuf {
        self.browser_versions().join(key)
    }
    pub fn browser_install_lock(&self) -> PathBuf {
        self.root.join("browser-install.lock")
    }
    pub fn browser_switch_previous(&self) -> PathBuf {
        self.browser_root.join("browser-switch-previous")
    }
    pub fn locks(&self) -> PathBuf {
        self.root.join("locks")
    }
    pub fn profile_lock(&self, id: &str) -> PathBuf {
        self.locks().join(format!("{id}.lock"))
    }
    pub fn profile_tags_lock(&self, id: &str) -> PathBuf {
        self.locks().join(format!("{id}.tags.lock"))
    }
    pub fn proxy_lock(&self, address: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        address.hash(&mut hasher);
        self.locks()
            .join(format!("proxy-{:016x}.lock", hasher.finish()))
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn profile_log(&self, id: &str) -> PathBuf {
        self.logs().join(format!("{id}.log"))
    }
    pub fn launch_progress(&self, id: &str, nonce: u64) -> PathBuf {
        self.logs().join(format!("launch-{id}-{nonce:016x}.jsonl"))
    }
    pub fn artifacts(&self) -> PathBuf {
        self.root.join("artifacts")
    }
    pub fn scan_artifacts(&self, id: &str) -> ScanArtifacts {
        let dir = self.artifacts();
        ScanArtifacts {
            screenshot: dir.join(format!("{id}-browserscan.png")),
            report: dir.join(format!("{id}-browserscan.txt")),
            verification: dir.join(format!("{id}-fingerprint.json")),
            fields: dir.join(format!("{id}-browserscan-fields.json")),
        }
    }
    pub fn trash(&self) -> PathBuf {
        self.profiles_root.join("trash")
    }
    pub fn trashed_profile(&self, id: &str, nonce: u64) -> PathBuf {
        self.trash().join(format!("{id}-{nonce}"))
    }
    pub fn runtime_sockets(&self) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        self.root.hash(&mut hasher);
        std::env::temp_dir().join(format!("cazer-browser-{:016x}", hasher.finish()))
    }
    pub fn runtime_socket(&self, id: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        self.runtime_sockets()
            .join(format!("{:016x}.sock", hasher.finish()))
    }
    #[cfg(windows)]
    pub fn runtime_pipe(&self, id: &str) -> String {
        let mut hasher = DefaultHasher::new();
        self.root.hash(&mut hasher);
        id.hash(&mut hasher);
        format!(r"\\.\pipe\cazer-browser-{:016x}", hasher.finish())
    }
}

fn resolve_storage_root(program_root: &Path, configured: Option<PathBuf>) -> Result<PathBuf> {
    let Some(path) = configured else {
        return Ok(program_root.to_path_buf());
    };
    if !path.is_absolute() {
        anyhow::bail!("storage directory must be absolute: {}", path.display());
    }
    if !path.is_dir() {
        anyhow::bail!("数据目录不存在：{}", path.display());
    }
    dunce::canonicalize(path).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_paths_are_not_verbatim() {
        let program = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let root = AppPaths::program_root(Some(program.path())).unwrap();
        assert!(!root.to_string_lossy().starts_with(r"\\?\"));
        let paths = AppPaths::with_directories(
            root,
            DataDirectories {
                browser_root: Some(storage.path().to_path_buf()),
                profiles_root: None,
            },
        )
        .unwrap();
        assert!(!paths.browser().to_string_lossy().starts_with(r"\\?\"));
        assert!(!paths.profile("test").to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn established_layout_is_stable() {
        let paths = AppPaths::for_root("/tmp/cazer-test-root");
        let root = paths.root();
        assert_eq!(paths.database(), root.join("browser.sqlite"));
        assert_eq!(paths.profile("alpha"), root.join("profiles/alpha"));
        assert_eq!(paths.browser(), root.join("browser"));
        assert_eq!(
            paths.browser_version("152.0-beta.31"),
            root.join("browser-versions/152.0-beta.31")
        );
        assert_eq!(paths.profile_lock("alpha"), root.join("locks/alpha.lock"));
        assert_eq!(
            paths.profile_tags_lock("alpha"),
            root.join("locks/alpha.tags.lock")
        );
        assert_eq!(paths.profile_log("alpha"), root.join("logs/alpha.log"));
        assert_eq!(
            paths.launch_progress("alpha", 1),
            root.join("logs/launch-alpha-0000000000000001.jsonl")
        );
        let artifacts = paths.scan_artifacts("alpha");
        assert_eq!(
            artifacts.fields,
            root.join("artifacts/alpha-browserscan-fields.json")
        );
        assert_eq!(artifacts.all().len(), 4);
        assert_eq!(
            paths.trashed_profile("alpha", 42),
            root.join("trash/alpha-42")
        );
    }

    #[test]
    fn runtime_socket_is_stable_for_the_same_root_and_id() {
        let paths = AppPaths::for_root("/tmp/cazer-test-root");
        assert_eq!(paths.runtime_socket("alpha"), paths.runtime_socket("alpha"));
        assert_ne!(paths.runtime_socket("alpha"), paths.runtime_socket("beta"));
        assert!(
            paths
                .runtime_socket("alpha")
                .parent()
                .unwrap()
                .starts_with(std::env::temp_dir())
        );
    }

    #[test]
    fn configured_roots_keep_database_in_program_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("program");
        fs::create_dir_all(&root).unwrap();
        let browser = temp.path().join("browser-files");
        let profiles = temp.path().join("profile-files");
        fs::create_dir_all(&browser).unwrap();
        fs::create_dir_all(&profiles).unwrap();
        let browser = dunce::canonicalize(browser).unwrap();
        let profiles = dunce::canonicalize(profiles).unwrap();
        DataDirectories {
            browser_root: Some(browser.clone()),
            profiles_root: Some(profiles.clone()),
        }
        .save(&root)
        .unwrap();
        let paths = AppPaths::new(Some(root.clone())).unwrap();
        assert_eq!(
            paths.database(),
            dunce::canonicalize(&root).unwrap().join("browser.sqlite")
        );
        assert_eq!(paths.browser(), browser.join("browser"));
        assert_eq!(paths.browser_versions(), browser.join("browser-versions"));
        assert_eq!(
            paths.browser_switch_previous(),
            browser.join("browser-switch-previous")
        );
        assert_eq!(paths.profile("alice"), profiles.join("profiles/alice"));
        assert_eq!(paths.trash(), profiles.join("trash"));
        let manager = crate::browser_manager::BrowserManager::new(&root).unwrap();
        assert_eq!(manager.active_path(), browser.join("browser"));
        let service =
            crate::profiles::ProfileService::new(&root, "socks5://127.0.0.1:12334").unwrap();
        assert_eq!(service.profiles_dir(), profiles.join("profiles"));
    }

    #[test]
    fn configured_roots_cannot_overlap_managed_data() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let inside_browser = root.join("browser/profile-storage");
        fs::create_dir_all(&inside_browser).unwrap();
        assert!(
            AppPaths::with_directories(
                root.clone(),
                DataDirectories {
                    profiles_root: Some(inside_browser),
                    ..Default::default()
                }
            )
            .is_err()
        );
        let inside_profiles = root.join("profiles/browser-storage");
        fs::create_dir_all(&inside_profiles).unwrap();
        assert!(
            AppPaths::with_directories(
                root,
                DataDirectories {
                    browser_root: Some(inside_profiles),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
