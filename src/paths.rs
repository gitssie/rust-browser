//! The on-disk layout owned by this application. Vendor cache paths are separate.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct AppPaths {
    root: PathBuf,
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
    /// Resolve the application's root once at CLI startup.
    pub fn new(override_dir: Option<PathBuf>) -> Result<Self> {
        let root = match override_dir {
            Some(path) => path,
            None => directories::ProjectDirs::from("io", "cazer", "rust-browser")
                .context("could not determine a platform data directory")?
                .data_dir()
                .to_path_buf(),
        };
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    /// Use a root already resolved by the caller, without creating files.
    pub fn for_root(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn database(&self) -> PathBuf {
        self.root.join("browser.sqlite")
    }
    pub fn profiles(&self) -> PathBuf {
        self.root.join("profiles")
    }
    pub fn profile(&self, id: &str) -> PathBuf {
        self.profiles().join(id)
    }
    pub fn browser(&self) -> PathBuf {
        self.root.join("browser")
    }
    pub fn browser_versions(&self) -> PathBuf {
        self.root.join("browser-versions")
    }
    pub fn browser_version(&self, key: &str) -> PathBuf {
        self.browser_versions().join(key)
    }
    pub fn browser_install_lock(&self) -> PathBuf {
        self.root.join("browser-install.lock")
    }
    pub fn browser_switch_previous(&self) -> PathBuf {
        self.root.join("browser-switch-previous")
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
        self.root.join("trash")
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
