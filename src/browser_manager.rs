//! Camoufox release management for the local settings page.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use camoufox_pkgman::version::CamoufoxVersion;
use camoufox_pkgman::{CamoufoxFetcher, extract_zip, make_executable};
use fs2::FileExt;

use crate::paths::AppPaths;
use crate::proxy::ProxySettings;

#[derive(Clone)]
pub struct BrowserManager {
    paths: AppPaths,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: CamoufoxVersion,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct InstalledVersion {
    pub version: CamoufoxVersion,
    pub path: PathBuf,
    pub active: bool,
}

/// The validated browser installation held under a runtime lock by callers.
#[derive(Clone, Debug)]
pub struct ActiveInstallation {
    pub root: PathBuf,
    pub version: CamoufoxVersion,
    pub executable_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadStage {
    Download,
    Verify,
    Install,
}

#[derive(Clone, Debug)]
pub struct DownloadProgress {
    pub stage: DownloadStage,
    pub received: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Default)]
pub struct DownloadControl {
    paused: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}

impl DownloadControl {
    pub fn pause(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn ensure_active(&self) -> Result<()> {
        if self.cancelled.load(Ordering::SeqCst) {
            bail!("下载已取消");
        }
        Ok(())
    }

    async fn wait_if_paused(&self) -> Result<()> {
        while self.is_paused() {
            self.ensure_active()?;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.ensure_active()
    }
}

impl BrowserManager {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            paths: AppPaths::new(Some(root.as_ref().to_path_buf()))?,
        })
    }

    pub fn with_paths(paths: AppPaths) -> Self {
        Self { paths }
    }

    pub fn active_path(&self) -> PathBuf {
        self.paths.browser()
    }

    fn archive_root(&self) -> PathBuf {
        self.paths.browser_versions()
    }

    fn lock_file(&self) -> Result<File> {
        fs::create_dir_all(self.paths.root())?;
        Ok(OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.paths.browser_install_lock())?)
    }

    /// Hold a shared lock for the full lifetime of a launched browser.
    pub fn runtime_guard(&self) -> Result<File> {
        loop {
            let file = self.lock_file()?;
            file.try_lock_shared()
                .context("浏览器版本正在切换，请稍后再打开")?;
            if !self.paths.browser_switch_previous().exists() {
                return Ok(file);
            }
            drop(file);
            self.recover_interrupted_switch()?;
        }
    }

    fn install_guard(&self) -> Result<File> {
        let file = self.lock_file()?;
        file.try_lock_exclusive()
            .context("浏览器正在运行或版本管理正在进行")?;
        Ok(file)
    }

    fn recover_interrupted_switch(&self) -> Result<()> {
        let _guard = self.install_guard()?;
        self.recover_interrupted_switch_locked()
    }

    fn recover_interrupted_switch_locked(&self) -> Result<()> {
        let previous = self.paths.browser_switch_previous();
        if !previous.exists() {
            return Ok(());
        }
        let active = self.active_path();
        if !active.exists() {
            fs::rename(&previous, &active).context("恢复被中断的浏览器版本切换")?;
            return Ok(());
        }
        // A new version reached the active path. Finish archiving its predecessor.
        let _ = version_at(&active)?.context("当前浏览器缺少版本信息，无法恢复切换")?;
        let old = version_at(&previous)?.context("待恢复的浏览器缺少版本信息")?;
        let archive = self.paths.browser_version(&version_key(&old)?);
        if archive.exists() {
            bail!("旧浏览器版本的归档目录已存在：{}", archive.display());
        }
        fs::create_dir_all(self.archive_root())?;
        fs::rename(&previous, &archive).context("完成被中断的浏览器版本归档")
    }

    pub fn active_version(&self) -> Result<Option<CamoufoxVersion>> {
        if self.paths.browser_switch_previous().exists() {
            self.recover_interrupted_switch()?;
        }
        version_at(&self.active_path())
    }

    /// Ensure the active installation is supported and has the files needed by
    /// camoufox-rust's launch path.
    pub fn prepare_active(&self) -> Result<ActiveInstallation> {
        let root = self.active_path();
        let Some(version) = self.active_version()? else {
            bail!(
                "Camoufox is not installed in {}; run `browserctl-rs fetch`",
                root.display()
            );
        };
        if !version.is_supported() {
            bail!("当前 Camoufox 版本不受支持，请先运行 browserctl-rs fetch");
        }
        prepare_browser_files(&root)?;
        let executable_path = browser_executable(&root);
        if !executable_path.is_file() {
            bail!("浏览器可执行文件不存在：{}", executable_path.display());
        }
        Ok(ActiveInstallation {
            root,
            version,
            executable_path: dunce::canonicalize(executable_path)?,
        })
    }

    pub fn installed_versions(&self) -> Result<Vec<InstalledVersion>> {
        let mut versions = Vec::new();
        if let Some(version) = self.active_version()? {
            versions.push(InstalledVersion {
                version,
                path: self.active_path(),
                active: true,
            });
        }
        let archived = self.archive_root();
        if archived.exists() {
            for entry in fs::read_dir(&archived)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let path = entry.path();
                if let Some(version) = version_at(&path)? {
                    versions.push(InstalledVersion {
                        version,
                        path,
                        active: false,
                    });
                }
            }
        }
        let start = usize::from(versions.first().is_some_and(|version| version.active));
        versions[start..].sort_by(|a, b| {
            if a.version.less_than(&b.version) {
                std::cmp::Ordering::Greater
            } else if b.version.less_than(&a.version) {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        Ok(versions)
    }

    pub async fn latest(&self, proxy: Option<&ProxySettings>) -> Result<Release> {
        let proxy_url = proxy.map(ProxySettings::request_url);
        let (version, url) = CamoufoxFetcher::new()
            .latest_asset_with_proxy(proxy_url.as_deref(), proxy.is_none())
            .await?;
        Ok(Release { version, url })
    }

    pub async fn download_and_install(
        &self,
        release: &Release,
        proxy: Option<&ProxySettings>,
        control: &DownloadControl,
        mut progress: impl FnMut(DownloadProgress),
    ) -> Result<()> {
        control.ensure_active()?;
        fs::create_dir_all(self.paths.browser_root())?;
        let staging = tempfile::Builder::new()
            .prefix("browser-download-")
            .tempdir_in(self.paths.browser_root())?;
        let archive = staging.path().join("camoufox.zip");
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("cazer-browser/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30));
        if let Some(proxy) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy.request_url())?);
        } else {
            builder = builder.no_proxy();
        }
        let client = builder.build()?;
        let mut request = client.get(&release.url);
        for (key, value) in camoufox_pkgman::github_authorization_headers(&release.url) {
            request = request.header(&key, &value);
        }
        let mut response = tokio::select! {
            response = request.send() => response?.error_for_status()?,
            _ = wait_for_cancel(control) => bail!("下载已取消"),
        };
        let total = response.content_length();
        let mut file = File::create(&archive)?;
        let mut received = 0u64;
        progress(DownloadProgress {
            stage: DownloadStage::Download,
            received,
            total,
        });
        loop {
            control.wait_if_paused().await?;
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk?,
                _ = wait_for_cancel(control) => bail!("下载已取消"),
            };
            let Some(chunk) = chunk else { break };
            control.ensure_active()?;
            file.write_all(&chunk)?;
            received += chunk.len() as u64;
            progress(DownloadProgress {
                stage: DownloadStage::Download,
                received,
                total,
            });
        }
        file.sync_all()?;
        drop(file);
        if let Some(total) = total
            && received != total
        {
            bail!("浏览器安装包不完整：收到 {received} / {total} 字节");
        }
        control.ensure_active()?;
        progress(DownloadProgress {
            stage: DownloadStage::Verify,
            received,
            total,
        });
        let extracted = staging.path().join("extracted");
        fs::create_dir(&extracted)?;
        extract_zip(&fs::read(&archive)?, &extracted)?;
        validate_executable(&extracted)?;
        prepare_browser_files(&extracted)?;
        make_executable(&extracted)?;
        fs::write(
            extracted.join("version.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": release.version.version,
                "release": release.version.release,
            }))?,
        )?;
        control.ensure_active()?;
        progress(DownloadProgress {
            stage: DownloadStage::Install,
            received,
            total,
        });
        self.install_extracted(&extracted, &release.version)?;
        Ok(())
    }

    fn install_extracted(&self, extracted: &Path, version: &CamoufoxVersion) -> Result<()> {
        let _guard = self.install_guard()?;
        self.recover_interrupted_switch_locked()?;
        let archived = self.paths.browser_version(&version_key(version)?);
        if archived.exists() {
            bail!("版本 {} 已存在于历史版本中", version.full_string());
        }
        self.promote_locked(extracted, "激活下载的浏览器版本")
    }

    pub fn activate(&self, version: &CamoufoxVersion) -> Result<()> {
        let _guard = self.install_guard()?;
        self.recover_interrupted_switch_locked()?;
        if self.active_version()?.as_ref() == Some(version) {
            return Ok(());
        }
        let archived = self.paths.browser_version(&version_key(version)?);
        if version_at(&archived)?.as_ref() != Some(version) {
            bail!("未找到已安装版本 {}", version.full_string());
        }
        self.promote_locked(&archived, "切换浏览器版本")
    }

    /// Called with the exclusive installation lock held.
    fn promote_locked(&self, candidate: &Path, action: &str) -> Result<()> {
        let active = self.active_path();
        let old = version_at(&active)?;
        let previous = self.paths.browser_switch_previous();
        if previous.exists() {
            bail!("待恢复的浏览器版本仍在：{}", previous.display());
        }
        if let Some(old) = &old {
            let archive = self.paths.browser_version(&version_key(old)?);
            if archive.exists() {
                bail!("当前版本归档已存在：{}", archive.display());
            }
            fs::create_dir_all(self.archive_root())?;
            fs::rename(&active, &previous).context("暂存当前浏览器版本")?;
        } else if active.exists() {
            bail!("安装目录已存在但缺少有效版本信息：{}", active.display());
        }
        if let Err(error) = fs::rename(candidate, &active) {
            if old.is_some() {
                fs::rename(&previous, &active).context("恢复原浏览器版本")?;
            }
            return Err(error).with_context(|| action.to_string());
        }
        self.recover_interrupted_switch_locked()
    }

    pub fn delete_archived(&self, version: &CamoufoxVersion) -> Result<()> {
        let _guard = self.install_guard()?;
        self.recover_interrupted_switch_locked()?;
        if self.active_version()?.as_ref() == Some(version) {
            bail!("当前使用的浏览器版本不能删除");
        }
        let path = self.paths.browser_version(&version_key(version)?);
        if version_at(&path)?.as_ref() != Some(version) {
            bail!("未找到已安装版本 {}", version.full_string());
        }
        fs::remove_dir_all(path)?;
        Ok(())
    }
}

async fn wait_for_cancel(control: &DownloadControl) {
    while !control.cancelled.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn version_at(path: &Path) -> Result<Option<CamoufoxVersion>> {
    if !path.exists() {
        return Ok(None);
    }
    CamoufoxVersion::from_path(path)
        .map(Some)
        .map_err(Into::into)
}

fn version_key(version: &CamoufoxVersion) -> Result<String> {
    let key = version.full_string();
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        bail!("无效的浏览器版本号：{key}");
    }
    Ok(key)
}

fn validate_executable(root: &Path) -> Result<()> {
    let binary = browser_executable(root);
    if !binary.is_file() {
        return Err(anyhow!("安装包缺少浏览器可执行文件：{}", binary.display()));
    }
    Ok(())
}

fn browser_executable(root: &Path) -> PathBuf {
    let relative = if cfg!(target_os = "macos") {
        "Camoufox.app/Contents/MacOS/camoufox"
    } else if cfg!(target_os = "windows") {
        "camoufox.exe"
    } else {
        "camoufox-bin"
    };
    root.join(relative)
}

fn prepare_browser_files(root: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let source = root.join("Camoufox.app/Contents/Resources/properties.json");
        if !source.is_file() {
            bail!("安装包缺少 properties.json");
        }
        for target in [
            root.join("properties.json"),
            root.join("Camoufox.app/Contents/MacOS/properties.json"),
        ] {
            if !target.exists() || fs::read(&source)? != fs::read(&target)? {
                fs::copy(&source, target)?;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = root;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    fn fake_version(path: &Path, release: &str) {
        fs::create_dir_all(path).unwrap();
        fs::write(
            path.join("version.json"),
            format!(r#"{{"version":"132.0","release":"{release}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn prepared_installation_uses_its_own_root_and_checks_executable() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(first.path()).unwrap();
        let other = BrowserManager::new(second.path()).unwrap();
        fake_version(&manager.active_path(), "0.9.1");
        fake_version(&other.active_path(), "0.9.2");
        let binary = browser_executable(&manager.active_path());
        assert!(manager.prepare_active().is_err());
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"browser").unwrap();
        #[cfg(target_os = "macos")]
        {
            let properties = manager
                .active_path()
                .join("Camoufox.app/Contents/Resources/properties.json");
            fs::create_dir_all(properties.parent().unwrap()).unwrap();
            fs::write(properties, b"{}").unwrap();
        }
        let installation = manager.prepare_active().unwrap();
        assert_eq!(installation.version.full_string(), "132.0-0.9.1");
        assert_eq!(
            installation.executable_path,
            dunce::canonicalize(binary).unwrap()
        );
        assert_ne!(
            installation.version,
            other.active_version().unwrap().unwrap()
        );
    }

    #[tokio::test]
    async fn default_addon_resolves_from_explicit_installation() {
        let root = tempfile::tempdir().unwrap();
        let resources = if cfg!(target_os = "macos") {
            root.path().join("Camoufox.app/Contents/Resources")
        } else {
            root.path().to_path_buf()
        };
        let addon = resources.join("addons/UBO");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("manifest.json"), "{}").unwrap();
        let mut found = Vec::new();
        camoufox_pkgman::add_default_addons_at(root.path(), &mut found, &[])
            .await
            .unwrap();
        assert_eq!(found, vec![addon.to_string_lossy().into_owned()]);
    }

    #[test]
    fn switches_and_deletes_archived_versions_without_touching_active() {
        let dir = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(dir.path()).unwrap();
        fake_version(&manager.active_path(), "0.9.1");
        let older = CamoufoxVersion::new("0.9.0", Some("132.0".into()));
        fake_version(
            &manager.archive_root().join(version_key(&older).unwrap()),
            "0.9.0",
        );
        assert_eq!(manager.installed_versions().unwrap().len(), 2);
        manager.activate(&older).unwrap();
        assert_eq!(manager.active_version().unwrap(), Some(older.clone()));
        assert!(manager.delete_archived(&older).is_err());
        let newer = CamoufoxVersion::new("0.9.1", Some("132.0".into()));
        manager.delete_archived(&newer).unwrap();
        assert_eq!(manager.installed_versions().unwrap().len(), 1);
    }

    #[test]
    fn installing_a_new_version_archives_the_previous_one() {
        let root = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(root.path()).unwrap();
        fake_version(&manager.active_path(), "0.9.1");
        let staging = root.path().join("staging");
        fake_version(&staging, "0.9.2");
        let newer = CamoufoxVersion::new("0.9.2", Some("132.0".into()));
        manager.install_extracted(&staging, &newer).unwrap();
        assert_eq!(manager.active_version().unwrap(), Some(newer));
        let older = CamoufoxVersion::new("0.9.1", Some("132.0".into()));
        assert_eq!(
            version_at(&manager.archive_root().join(version_key(&older).unwrap())).unwrap(),
            Some(older)
        );
    }

    #[test]
    fn interrupted_switch_restores_previous_browser_before_launch() {
        let root = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(root.path()).unwrap();
        fake_version(&manager.active_path(), "0.9.1");
        fs::rename(
            manager.active_path(),
            manager.paths.browser_switch_previous(),
        )
        .unwrap();
        let _runtime = manager.runtime_guard().unwrap();
        assert_eq!(manager.active_version().unwrap().unwrap().release, "0.9.1");
        assert!(!manager.paths.browser_switch_previous().exists());
    }

    #[test]
    fn interrupted_switch_archives_previous_browser_after_activation() {
        let root = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(root.path()).unwrap();
        fake_version(&manager.paths.browser_switch_previous(), "0.9.1");
        fake_version(&manager.active_path(), "0.9.2");
        let versions = manager.installed_versions().unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version.release, "0.9.2");
        assert!(versions[0].active);
        assert_eq!(versions[1].version.release, "0.9.1");
        assert!(!versions[1].active);
        assert!(!manager.paths.browser_switch_previous().exists());
    }

    #[test]
    fn running_browser_blocks_version_switch() {
        let root = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(root.path()).unwrap();
        fake_version(&manager.active_path(), "0.9.1");
        let older = CamoufoxVersion::new("0.9.0", Some("132.0".into()));
        fake_version(
            &manager.archive_root().join(version_key(&older).unwrap()),
            "0.9.0",
        );
        let runtime = manager.runtime_guard().unwrap();
        assert!(manager.activate(&older).is_err());
        assert_eq!(manager.active_version().unwrap().unwrap().release, "0.9.1");
        drop(runtime);
        manager.activate(&older).unwrap();
    }

    #[tokio::test]
    async fn downloads_and_installs_a_release_from_a_local_server() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            let binary = if cfg!(target_os = "macos") {
                "Camoufox.app/Contents/MacOS/camoufox"
            } else if cfg!(target_os = "windows") {
                "camoufox.exe"
            } else {
                "camoufox-bin"
            };
            zip.start_file(binary, options).unwrap();
            zip.write_all(b"test binary").unwrap();
            if cfg!(target_os = "macos") {
                zip.start_file("Camoufox.app/Contents/Resources/properties.json", options)
                    .unwrap();
                zip.write_all(b"{}").unwrap();
            }
            zip.finish().unwrap();
        }
        let body = bytes.into_inner();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/camoufox.zip", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(root.path()).unwrap();
        let release = Release {
            version: CamoufoxVersion::new("0.9.9", Some("132.0".into())),
            url,
        };
        let mut stages = Vec::new();
        manager
            .download_and_install(&release, None, &DownloadControl::default(), |progress| {
                stages.push(progress.stage);
            })
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(manager.active_version().unwrap(), Some(release.version));
        assert!(stages.contains(&DownloadStage::Verify));
        assert!(stages.contains(&DownloadStage::Install));
        assert!(!root.path().read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("browser-download-")
        }));
    }

    #[tokio::test]
    async fn cancellation_before_download_preserves_current_version() {
        let root = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(root.path()).unwrap();
        fake_version(&manager.active_path(), "0.9.1");
        let control = DownloadControl::default();
        control.cancel();
        let release = Release {
            version: CamoufoxVersion::new("0.9.2", Some("132.0".into())),
            url: "http://127.0.0.1:1/never.zip".into(),
        };
        assert!(
            manager
                .download_and_install(&release, None, &control, |_| {})
                .await
                .is_err()
        );
        assert_eq!(manager.active_version().unwrap().unwrap().release, "0.9.1");
    }

    #[tokio::test]
    async fn cancel_interrupts_wait_for_response_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/slow.zip", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            std::thread::sleep(Duration::from_millis(800));
        });
        let root = tempfile::tempdir().unwrap();
        let manager = BrowserManager::new(root.path()).unwrap();
        let control = DownloadControl::default();
        let cancel = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });
        let release = Release {
            version: CamoufoxVersion::new("0.9.9", Some("132.0".into())),
            url,
        };
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            manager.download_and_install(&release, None, &control, |_| {}),
        )
        .await;
        assert!(result.unwrap().unwrap_err().to_string().contains("取消"));
        server.join().unwrap();
    }
}
