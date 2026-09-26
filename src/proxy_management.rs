//! Managed proxy policy shared by CLI launches and the native manager.

use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use url::Url;

use crate::geo::ProfileGeo;
use crate::launch_progress::LaunchStage;
use crate::paths::AppPaths;
use crate::profiles::{ProfileQuery, ProfileService, ProxyChoice};
use crate::proxy::ProxySettings;
use crate::runtime::BrowserRuntime;
use crate::storage;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyPolicy {
    #[default]
    AllowParallel,
    RejectNew,
    ClosePrevious,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SwitchMethod {
    #[default]
    Get,
    Post,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpSwitch {
    pub url: String,
    pub method: SwitchMethod,
    pub on_start: bool,
    /// Wait after a request before checking the SOCKS exit; also separates retries.
    pub wait_seconds: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedProxy {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub credentials: Option<ProxyCredentials>,
    pub policy: ProxyPolicy,
    pub ip_switch: Option<IpSwitch>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProxyCredentials {
    pub username: String,
    pub password: String,
}

impl ManagedProxy {
    fn validate(mut self) -> Result<Self> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self.id.as_bytes()[0].is_ascii_alphanumeric()
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            bail!("proxy id must be 1-64 ASCII letters, digits, _ or -");
        }
        self.name = self.name.trim().to_string();
        if self.name.is_empty()
            || self.name.chars().count() > 100
            || self.name.chars().any(char::is_control)
        {
            bail!("proxy name must contain 1-100 printable characters");
        }
        let address = ProxySettings::parse(&self.url)?;
        if self.credentials.as_ref().is_some_and(|credentials| {
            credentials.username.is_empty() && credentials.password.is_empty()
        }) {
            self.credentials = None;
        }
        if let Some(credentials) = &self.credentials {
            ProxySettings::configured(
                address.host(),
                address.port(),
                &credentials.username,
                &credentials.password,
                true,
                10,
            )?;
        }
        self.url = address.browser_url();
        if let Some(config) = self.ip_switch.as_mut() {
            let url = Url::parse(&config.url).context("invalid switch-IP URL")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                bail!("switch-IP URL must be absolute HTTP(S) without credentials or fragment");
            }
            if !(1..=60).contains(&config.wait_seconds) {
                bail!("switch-IP wait must be 1-60 seconds");
            }
            config.url = url.to_string();
        }
        Ok(self)
    }
}

#[derive(Clone)]
pub struct ProxyCatalog {
    root: PathBuf,
}

impl ProxyCatalog {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn list(&self) -> Result<Vec<ManagedProxy>> {
        let conn = storage::connection(&self.root)?;
        let mut stmt = conn.prepare("SELECT data FROM managed_proxies ORDER BY id")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(serde_json::from_str(&row?)?);
        }
        Ok(records)
    }

    pub fn upsert(&self, record: ManagedProxy) -> Result<ManagedProxy> {
        let record = record.validate()?;
        let mut conn = storage::connection(&self.root)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let duplicate: Option<String> = tx
            .query_row(
                "SELECT id FROM managed_proxies WHERE url = ?1 AND id != ?2",
                params![record.url, record.id],
                |row| row.get(0),
            )
            .optional()?;
        if duplicate.is_some() {
            bail!("another managed proxy already uses this SOCKS5 address");
        }
        tx.execute(
            "INSERT INTO managed_proxies (id, url, data) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET url = excluded.url, data = excluded.data",
            params![record.id, record.url, serde_json::to_string(&record)?],
        )?;
        tx.commit()?;
        Ok(record)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let conn = storage::connection(&self.root)?;
        if conn.execute("DELETE FROM managed_proxies WHERE id = ?1", [id])? == 0 {
            bail!("managed proxy {id} not found");
        }
        Ok(())
    }

    pub fn find_url(&self, address: &str) -> Result<Option<ManagedProxy>> {
        let url = ProxySettings::parse(address)?.browser_url();
        Ok(self.list()?.into_iter().find(|record| record.url == url))
    }

    pub fn resolve_url(&self, address: &str) -> Result<ProxySettings> {
        let proxy = ProxySettings::parse(address)?;
        let Some(record) = self.find_url(address)? else {
            return Ok(proxy);
        };
        let Some(credentials) = record.credentials else {
            return Ok(proxy);
        };
        ProxySettings::configured(
            proxy.host(),
            proxy.port(),
            &credentials.username,
            &credentials.password,
            true,
            10,
        )
    }
}

pub struct ProxyLaunchGuard(File);

impl ProxyLaunchGuard {
    async fn acquire(root: &Path, address: &str) -> Result<Self> {
        let paths = AppPaths::for_root(root);
        let locks = paths.locks();
        fs::create_dir_all(&locks)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(paths.proxy_lock(address))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self(file)),
                Err(error) if lock_contended(&error) && Instant::now() < deadline => {
                    sleep(Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error).context("could not acquire proxy launch lock"),
            }
        }
    }
}

impl Drop for ProxyLaunchGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub async fn prepare_browser_launch(
    root: &Path,
    profile_id: &str,
    proxy: &ProxySettings,
    global_proxy: &str,
) -> Result<Option<ProxyLaunchGuard>> {
    prepare_browser_launch_with_progress(root, profile_id, proxy, global_proxy, |_, _| Ok(())).await
}

pub async fn prepare_browser_launch_with_progress(
    root: &Path,
    profile_id: &str,
    proxy: &ProxySettings,
    global_proxy: &str,
    mut report: impl FnMut(LaunchStage, Option<String>) -> Result<()>,
) -> Result<Option<ProxyLaunchGuard>> {
    let Some(record) = ProxyCatalog::new(root).find_url(&proxy.browser_url())? else {
        return Ok(None);
    };
    let guard = ProxyLaunchGuard::acquire(root, &record.url).await?;
    let should_rotate = record
        .ip_switch
        .as_ref()
        .is_some_and(|config| config.on_start);
    if record.policy != ProxyPolicy::AllowParallel || should_rotate {
        let service = ProfileService::new(root, global_proxy)?;
        let runtime = BrowserRuntime::new(root);
        let mut peers = Vec::new();
        let mut page = 1;
        loop {
            let result = service
                .list(ProfileQuery {
                    search: None,
                    page,
                    page_size: 100,
                })
                .await?;
            for item in result.items {
                if item.id != profile_id
                    && matches!(item.proxy, ProxyChoice::Custom(_))
                    && item.effective_proxy == record.url
                    && profile_is_active(root, &item.id, &runtime).await?
                {
                    peers.push(item.id);
                }
            }
            if page >= result.total_pages {
                break;
            }
            page += 1;
        }
        if record.policy == ProxyPolicy::ClosePrevious && !peers.is_empty() {
            report(LaunchStage::ClosePeers, Some(peers.len().to_string()))?;
        }
        enforce_policy(root, &record, should_rotate, &peers, &runtime).await?;
    }
    if let Some(config) = record.ip_switch.as_ref().filter(|config| config.on_start) {
        report(LaunchStage::SwitchIp, None)?;
        let before = ProfileGeo::lookup(proxy)
            .await
            .context("check proxy IP before switching")?;
        let after = switch_ip(config, proxy, &before.ip).await?;
        println!(
            "Proxy {} switched IP: {} -> {}",
            record.id, before.ip, after.ip
        );
    }
    Ok(Some(guard))
}

async fn enforce_policy(
    root: &Path,
    record: &ManagedProxy,
    should_rotate: bool,
    peers: &[String],
    runtime: &BrowserRuntime,
) -> Result<()> {
    if peers.is_empty() {
        return Ok(());
    }
    match record.policy {
        ProxyPolicy::AllowParallel if should_rotate => bail!(
            "cannot switch IP while profiles share proxy {}: {}",
            record.id,
            peers.join(", ")
        ),
        ProxyPolicy::AllowParallel => Ok(()),
        ProxyPolicy::RejectNew => bail!("proxy {} is in use by {}", record.id, peers.join(", ")),
        ProxyPolicy::ClosePrevious => {
            for id in peers {
                if runtime.is_running(id).await
                    && let Err(error) = runtime.close(id).await
                    && runtime.is_running(id).await
                {
                    return Err(error).with_context(|| format!("close browser {id}"));
                }
            }
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let mut active = Vec::new();
                for id in peers {
                    if profile_is_active(root, id, runtime).await? {
                        active.push(id.clone());
                    }
                }
                if active.is_empty() {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    bail!(
                        "timed out waiting for browser(s) to close: {}",
                        active.join(", ")
                    );
                }
                sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

async fn profile_is_active(root: &Path, id: &str, runtime: &BrowserRuntime) -> Result<bool> {
    Ok(runtime.is_running(id).await || !profile_lock_released(root, id)?)
}

fn profile_lock_released(root: &Path, id: &str) -> Result<bool> {
    let path = AppPaths::for_root(root).profile_lock(id);
    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error).with_context(|| format!("open profile lock for {id}")),
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            file.unlock()?;
            Ok(true)
        }
        Err(error) if lock_contended(&error) => Ok(false),
        Err(error) => Err(error).with_context(|| format!("check profile lock for {id}")),
    }
}

fn lock_contended(error: &std::io::Error) -> bool {
    error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

pub async fn switch_ip(
    config: &IpSwitch,
    proxy: &ProxySettings,
    before_ip: &str,
) -> Result<ProfileGeo> {
    switch_ip_with(
        config,
        before_ip,
        Duration::from_secs(config.wait_seconds),
        || ProfileGeo::lookup(proxy),
    )
    .await
}

async fn switch_ip_with<F, Fut>(
    config: &IpSwitch,
    before_ip: &str,
    wait: Duration,
    mut observe: F,
) -> Result<ProfileGeo>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<ProfileGeo>>,
{
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut last_error = anyhow::anyhow!("proxy exit IP did not change");
    for _ in 0..2 {
        let request = match config.method {
            SwitchMethod::Get => client.get(&config.url),
            SwitchMethod::Post => client.post(&config.url),
        };
        let response = request.send().await;
        // A 429 (or a lost response) does not prove the provider ignored the
        // action. Check the proxy exit after the configured propagation delay.
        sleep(wait).await;
        let observation = match observe().await {
            Ok(geo) if geo.ip != before_ip => return Ok(geo),
            Ok(_) => anyhow::anyhow!("proxy exit IP remained {before_ip}"),
            Err(error) => error.context("could not verify proxy IP after switching"),
        };
        match response {
            Ok(response) if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                return Err(observation).context(
                    "switch-IP endpoint returned HTTP 429; wait for the provider's rate limit before trying again",
                );
            }
            Ok(response) if response.status().is_success() => last_error = observation,
            Ok(response) => {
                last_error = observation.context(format!(
                    "switch-IP endpoint returned HTTP {}",
                    response.status()
                ));
            }
            Err(error) => {
                last_error = observation.context(format!(
                    "switch-IP HTTP request failed: {}",
                    error.without_url()
                ));
            }
        }
    }
    Err(last_error).context("switch-IP failed after one retry")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo(ip: &str) -> ProfileGeo {
        ProfileGeo {
            ip: ip.into(),
            country_code: "FR".into(),
            country: "France".into(),
            region: Some("Île-de-France".into()),
            region_code: Some("IDF".into()),
            city: Some("Paris".into()),
            timezone: "Europe/Paris".into(),
            latitude: 48.85,
            longitude: 2.35,
            locale: "fr-FR".into(),
            source: "test".into(),
        }
    }

    #[test]
    fn catalog_normalizes_address_and_rejects_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = ProxyCatalog::new(dir.path());
        let record = ManagedProxy {
            id: "local".into(),
            name: "Local".into(),
            url: "socks://localhost:1234".into(),
            credentials: None,
            policy: ProxyPolicy::ClosePrevious,
            ip_switch: None,
        };
        assert_eq!(
            catalog.upsert(record.clone()).unwrap().url,
            "socks5://localhost:1234"
        );
        assert_eq!(
            catalog
                .find_url("socks5h://localhost:1234")
                .unwrap()
                .unwrap()
                .id,
            "local"
        );
        assert!(
            catalog
                .upsert(ManagedProxy {
                    id: "duplicate".into(),
                    ..record
                })
                .is_err()
        );
        catalog.remove("local").unwrap();
        assert!(catalog.list().unwrap().is_empty());
    }

    #[test]
    fn managed_credentials_resolve_for_custom_address() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = ProxyCatalog::new(dir.path());
        catalog
            .upsert(ManagedProxy {
                id: "fr".into(),
                name: "France".into(),
                url: "socks5://proxy.example:1080".into(),
                credentials: Some(ProxyCredentials {
                    username: "account".into(),
                    password: "s:ecret".into(),
                }),
                policy: ProxyPolicy::ClosePrevious,
                ip_switch: None,
            })
            .unwrap();
        let proxy = catalog.resolve_url("socks5h://proxy.example:1080").unwrap();
        assert_eq!(proxy.browser_url(), "socks5://proxy.example:1080");
        assert_eq!(
            proxy.request_url(),
            "socks5h://account:s%3Aecret@proxy.example:1080"
        );
        assert_eq!(proxy.credentials(), Some(("account", "s:ecret")));
        let other = catalog.resolve_url("socks5://other.example:1080").unwrap();
        assert_eq!(other.credentials(), None);
        let saved = catalog.list().unwrap().pop().unwrap();
        assert_eq!(saved.credentials.unwrap().password, "s:ecret");
    }

    #[test]
    fn legacy_managed_record_without_credentials_deserializes() {
        let record: ManagedProxy = serde_json::from_str(
            r#"{"id":"old","name":"Old","url":"socks5://localhost:1080","policy":"allow-parallel","ip_switch":null}"#,
        ).unwrap();
        assert!(record.credentials.is_none());
    }

    #[test]
    fn switch_config_validates_http_and_wait() {
        let base = ManagedProxy {
            id: "one".into(),
            name: "One".into(),
            url: "socks5://localhost:1234".into(),
            credentials: None,
            policy: ProxyPolicy::AllowParallel,
            ip_switch: Some(IpSwitch {
                url: "https://example.com/switch".into(),
                method: SwitchMethod::Post,
                on_start: true,
                wait_seconds: 5,
            }),
        };
        assert!(base.clone().validate().is_ok());
        let mut bad = base;
        bad.ip_switch.as_mut().unwrap().wait_seconds = 0;
        assert!(bad.validate().is_err());
    }

    #[tokio::test]
    async fn held_profile_lock_counts_as_active_without_control_socket() {
        let dir = tempfile::tempdir().unwrap();
        let lock_dir = dir.path().join("locks");
        fs::create_dir_all(&lock_dir).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_dir.join("old.lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        let runtime = BrowserRuntime::new(dir.path());
        assert!(
            profile_is_active(dir.path(), "old", &runtime)
                .await
                .unwrap()
        );
        drop(lock);
        assert!(
            !profile_is_active(dir.path(), "old", &runtime)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn close_previous_policy_waits_for_browser_exit() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = BrowserRuntime::new(dir.path());
        let listener = runtime.bind("old").await.unwrap();
        let lock_dir = dir.path().join("locks");
        fs::create_dir_all(&lock_dir).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_dir.join("old.lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        let task = tokio::spawn(async move {
            listener.wait_for_close().await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            drop(lock);
        });
        let record = ManagedProxy {
            id: "shared".into(),
            name: "Shared".into(),
            url: "socks5://localhost:1234".into(),
            credentials: None,
            policy: ProxyPolicy::RejectNew,
            ip_switch: None,
        };
        let peers = vec!["old".to_string()];
        assert!(
            enforce_policy(dir.path(), &record, false, &peers, &runtime)
                .await
                .is_err()
        );
        assert!(runtime.is_running("old").await);
        let start = Instant::now();
        enforce_policy(
            dir.path(),
            &ManagedProxy {
                policy: ProxyPolicy::ClosePrevious,
                ..record
            },
            false,
            &peers,
            &runtime,
        )
        .await
        .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(150));
        task.await.unwrap();
        assert!(!runtime.is_running("old").await);
    }

    #[tokio::test]
    async fn post_switch_waits_and_retries_once_until_ip_changes() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 1024];
                let size = stream.read(&mut request).await.unwrap();
                assert!(
                    std::str::from_utf8(&request[..size])
                        .unwrap()
                        .starts_with("POST /switch ")
                );
                server_requests.fetch_add(1, Ordering::SeqCst);
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
        });
        let config = IpSwitch {
            url: format!("http://{address}/switch"),
            method: SwitchMethod::Post,
            on_start: true,
            wait_seconds: 1,
        };
        let observations = Arc::new(AtomicUsize::new(0));
        let result = switch_ip_with(&config, "203.0.113.10", Duration::from_millis(5), || {
            let observations = observations.clone();
            async move {
                let count = observations.fetch_add(1, Ordering::SeqCst);
                Ok(geo(if count == 0 {
                    "203.0.113.10"
                } else {
                    "203.0.113.11"
                }))
            }
        })
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(result.ip, "203.0.113.11");
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(observations.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn http_429_can_still_mean_switch_succeeded() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream
                .write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let config = IpSwitch {
            url: format!("http://{address}/private-action-token"),
            method: SwitchMethod::Get,
            on_start: true,
            wait_seconds: 10,
        };
        let result = switch_ip_with(
            &config,
            "203.0.113.10",
            Duration::from_millis(5),
            || async { Ok(geo("203.0.113.11")) },
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(result.ip, "203.0.113.11");
    }

    #[tokio::test]
    async fn second_request_429_succeeds_if_first_switch_becomes_visible() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for status in ["200 OK", "429 Too Many Requests"] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 1024];
                assert!(stream.read(&mut request).await.unwrap() > 0);
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let config = IpSwitch {
            url: format!("http://{address}/switch"),
            method: SwitchMethod::Get,
            on_start: true,
            wait_seconds: 10,
        };
        let observations = Arc::new(AtomicUsize::new(0));
        let result = switch_ip_with(&config, "203.0.113.10", Duration::from_millis(5), || {
            let observations = observations.clone();
            async move {
                let attempt = observations.fetch_add(1, Ordering::SeqCst);
                Ok(geo(if attempt == 0 {
                    "203.0.113.10"
                } else {
                    "203.0.113.11"
                }))
            }
        })
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(result.ip, "203.0.113.11");
        assert_eq!(observations.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn http_429_without_ip_change_does_not_repeat_action_or_expose_url() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream
                .write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let config = IpSwitch {
            url: format!("http://{address}/private-action-token"),
            method: SwitchMethod::Get,
            on_start: true,
            wait_seconds: 10,
        };
        let error = switch_ip_with(
            &config,
            "203.0.113.10",
            Duration::from_millis(5),
            || async { Ok(geo("203.0.113.10")) },
        )
        .await
        .unwrap_err();
        server.await.unwrap();
        let message = format!("{error:#}");
        assert!(message.contains("429"));
        assert!(!message.contains("private-action-token"));
    }
}
