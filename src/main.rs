use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use camoufox::builder::{HeadlessMode, LaunchOptions, PreparedLaunch, ProxyConfig, prepare};
use camoufox_core::config::get_env_vars;
use camoufox_core::fingerprint::determine_ua_os;
use camoufox_core::os::SupportedOs;
use camoufox_core::persona::PersonaRecord;
use camoufox_juggler::{JugglerBrowser, launch_with_juggler, verify_fingerprint};
use clap::{Parser, Subcommand, ValueEnum};
use fs2::FileExt;
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use url::Url;

use rust_browser::browser_manager::{
    ActiveInstallation, BrowserManager, DownloadControl, DownloadStage,
};
use rust_browser::geo::ProfileGeo;
use rust_browser::launch_progress::{LaunchEvent, LaunchProgressWriter, LaunchStage};
use rust_browser::paths::AppPaths as Paths;
use rust_browser::profiles::{CreateProfile, ProfileOs, ProfileService, ProxyChoice};
use rust_browser::proxy::ProxySettings;
use rust_browser::proxy_management::{
    IpSwitch, ManagedProxy, ProxyCatalog, ProxyPolicy, SwitchMethod,
    prepare_browser_launch_with_progress,
};
use rust_browser::runtime::BrowserRuntime;
use rust_browser::settings::{BrowserDownloadSettings, GeneralSettings};
use rust_browser::storage;
use rust_browser::workspace_lock::WorkspaceLock;

mod ui;

const BROWSERSCAN_URL: &str = "https://www.browserscan.net/";

const SCAN_IDENTITY_FIELDS: &[&str] = &[
    "visitor ID",
    "Canvas",
    "WebGL",
    "WebGL Report",
    "Audio",
    "Client Rects",
    "WebGPU Report",
    "Unmasked Vendor",
    "Unmasked Renderer",
    "Screen Resolution",
    "Available Screen Size",
    "Color Depth",
    "Hardware Concurrency",
    "OS",
    "Browser",
    "Browser Version",
    "Header",
    "JavaScript",
    "Time Zone",
    "Languages",
    "Accept-Language header",
    "Internationalization API",
    "WebRTC",
    "WebRTC STUN",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ScanIdentity(BTreeMap<String, String>);

impl ScanIdentity {
    fn parse(report: &str) -> Result<Self> {
        let lines: Vec<&str> = report.lines().map(str::trim).collect();
        let mut values = BTreeMap::new();
        for label in SCAN_IDENTITY_FIELDS {
            let index = lines
                .iter()
                .rposition(|line| line == label)
                .with_context(|| format!("BrowserScan missing {label}"))?;
            let value = lines[index + 1..]
                .iter()
                .find(|line| !line.is_empty())
                .with_context(|| format!("BrowserScan has no value for {label}"))?;
            if SCAN_IDENTITY_FIELDS.contains(value) {
                bail!("BrowserScan has not loaded {label} yet");
            }
            if [
                "visitor ID",
                "Canvas",
                "WebGL",
                "WebGL Report",
                "Audio",
                "Client Rects",
                "WebGPU Report",
            ]
            .contains(label)
                && (value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            {
                bail!("BrowserScan has an invalid {label} hash: {value}");
            }
            values.insert((*label).to_string(), (*value).to_string());
        }
        Ok(Self(values))
    }
}

#[derive(Parser)]
#[command(
    name = "browserctl-rs",
    about = "Manage persistent Camoufox browser profiles"
)]
struct Cli {
    /// Root for this CLI's personas, browser installation, and browser profiles.
    #[arg(long, global = true, env = "RUST_BROWSER_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Global SOCKS5 proxy; profiles without a custom proxy follow this value.
    #[arg(long = "global-proxy", global = true, env = "RUST_BROWSER_PROXY")]
    global_proxy: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download a Camoufox browser release into this CLI's own data directory.
    Fetch,
    /// Create and persist a fingerprint identity.
    Create {
        id: String,
        /// Proxy for this profile instead of the global proxy.
        #[arg(long)]
        proxy: Option<String>,
        #[arg(long, value_enum, default_value_t = BrowserOs::Macos)]
        os: BrowserOs,
        #[arg(long)]
        url: Option<String>,
        /// Startup URLs. Repeat to open several tabs.
        #[arg(long = "tab")]
        tabs: Vec<String>,
    },
    /// List saved identities.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show a saved identity and its browser data path.
    Show { id: String },
    /// Print locations of this CLI's data and a profile's browser directory.
    Paths { id: Option<String> },
    /// Show or replace startup tabs.
    Tabs {
        id: String,
        #[arg(long = "set", conflicts_with = "clear")]
        urls: Vec<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Open a persistent, visible browser through the profile's configured SOCKS5 proxy.
    Open {
        id: String,
        #[arg(long)]
        url: Option<String>,
        /// Progress events for the native manager.
        #[arg(long, hide = true)]
        progress_file: Option<PathBuf>,
    },
    /// Capture BrowserScan plus camoufox-rust's fingerprint verification report.
    Scan { id: String },
    /// Accept the current proxy location while keeping fingerprint noise seeds.
    Refresh { id: String },
    /// Permanently delete a profile and its browser data.
    Delete {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Check local proxy and browser installation without launching it.
    Doctor,
    /// Open the native GPUI profile manager.
    Ui,
    /// Manage proxy-wide launch and IP switching rules.
    Proxy {
        #[command(subcommand)]
        action: ProxyAction,
    },
}

#[derive(Subcommand)]
enum ProxyAction {
    /// List managed proxies.
    List,
    /// Create or replace one managed proxy rule.
    Set {
        id: String,
        url: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, value_enum, default_value_t = ProxyPolicyArg::AllowParallel)]
        policy: ProxyPolicyArg,
        #[arg(long)]
        switch_url: Option<String>,
        #[arg(long, value_enum, default_value_t = SwitchMethodArg::Get)]
        switch_method: SwitchMethodArg,
        #[arg(long, requires = "switch_url")]
        rotate_on_start: bool,
        #[arg(long, default_value_t = 5)]
        wait_seconds: u64,
    },
    /// Remove a managed rule; saved profiles and proxy URLs are untouched.
    Remove { id: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum ProxyPolicyArg {
    AllowParallel,
    RejectNew,
    ClosePrevious,
}

impl From<ProxyPolicyArg> for ProxyPolicy {
    fn from(value: ProxyPolicyArg) -> Self {
        match value {
            ProxyPolicyArg::AllowParallel => Self::AllowParallel,
            ProxyPolicyArg::RejectNew => Self::RejectNew,
            ProxyPolicyArg::ClosePrevious => Self::ClosePrevious,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum SwitchMethodArg {
    Get,
    Post,
}

impl From<SwitchMethodArg> for SwitchMethod {
    fn from(value: SwitchMethodArg) -> Self {
        match value {
            SwitchMethodArg::Get => Self::Get,
            SwitchMethodArg::Post => Self::Post,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BrowserOs {
    Macos,
    Windows,
    Linux,
}

impl BrowserOs {
    fn supported(self) -> SupportedOs {
        match self {
            Self::Macos => SupportedOs::Macos,
            Self::Windows => SupportedOs::Windows,
            Self::Linux => SupportedOs::Linux,
        }
    }

    fn as_str(self) -> &'static str {
        self.supported().as_str()
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        bail!(
            "profile id must start with a letter/digit and contain only ASCII letters, digits, _ or - (max 64)"
        );
    }
    PersonaRecord::validate_id(id)?;
    Ok(())
}

fn validate_url(input: &str) -> Result<String> {
    let parsed = Url::parse(input).context("URL must be an absolute http(s) URL")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        bail!("URL must be an absolute http(s) URL without credentials");
    }
    Ok(parsed.to_string())
}

fn validate_tabs(values: &[String]) -> Result<Vec<String>> {
    let mut tabs = Vec::new();
    for value in values {
        let url = validate_url(value)?;
        if !tabs.contains(&url) {
            tabs.push(url);
        }
    }
    Ok(tabs)
}

fn startup_tabs(persona: &PersonaRecord) -> Result<Vec<String>> {
    if let Some(values) = persona.metadata.get("tabs") {
        let values = values.as_array().context("saved tabs must be an array")?;
        let values = values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .context("saved tab must be a URL string")
            })
            .collect::<Result<Vec<_>>>()?;
        return validate_tabs(&values);
    }
    Ok(vec![validate_url(metadata_str(persona, "url")?)?])
}

fn metadata_str<'a>(persona: &'a PersonaRecord, key: &str) -> Result<&'a str> {
    persona
        .metadata
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("profile {} has no valid {key}", persona.id))
}

fn persona_os(persona: &PersonaRecord) -> Result<SupportedOs> {
    match metadata_str(persona, "os")? {
        "macos" => Ok(SupportedOs::Macos),
        "windows" => Ok(SupportedOs::Windows),
        "linux" => Ok(SupportedOs::Linux),
        value => bail!("unsupported saved OS: {value}"),
    }
}

fn profile_lock(paths: &Paths, id: &str) -> Result<File> {
    fs::create_dir_all(paths.locks())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.profile_lock(id))?;
    lock.try_lock_exclusive()
        .with_context(|| format!("profile {id} is already open"))?;
    Ok(lock)
}

fn profile_proxy(
    root: &Path,
    persona: &PersonaRecord,
    global_proxy: &ProxySettings,
) -> Result<ProxySettings> {
    match persona.metadata.get("proxy_mode").and_then(Value::as_str) {
        Some("custom") => ProxyCatalog::new(root).resolve_url(metadata_str(persona, "proxy_url")?),
        Some("global") | None => Ok(global_proxy.clone()),
        Some(other) => bail!("invalid proxy mode: {other}"),
    }
}

async fn create(
    paths: &Paths,
    id: &str,
    os: BrowserOs,
    url: Option<&str>,
    tabs: &[String],
    proxy_override: Option<&str>,
    global_proxy: &ProxySettings,
) -> Result<()> {
    let service = ProfileService::with_proxy(paths.root(), global_proxy.clone())?;
    let view = service
        .create(CreateProfile {
            id: id.to_string(),
            name: None,
            os: match os {
                BrowserOs::Macos => ProfileOs::Macos,
                BrowserOs::Windows => ProfileOs::Windows,
                BrowserOs::Linux => ProfileOs::Linux,
            },
            tabs: if tabs.is_empty() {
                url.map(|value| vec![value.to_string()]).unwrap_or_default()
            } else {
                tabs.to_vec()
            },
            proxy: match proxy_override {
                Some(proxy) => ProxyChoice::Custom(proxy.to_string()),
                None => ProxyChoice::Global,
            },
            geo: None,
        })
        .await?;
    println!("Created {id} ({})", os.as_str());
    println!(
        "Saved location: {} | {} | {} | {}",
        view.saved_geo.ip,
        view.saved_geo.country_code,
        view.saved_geo.region.as_deref().unwrap_or("-"),
        view.saved_geo.timezone
    );
    println!("Browser data: {}", view.browser_data_dir.display());
    Ok(())
}

async fn open(
    paths: &Paths,
    id: &str,
    url: Option<&str>,
    global_proxy: &ProxySettings,
    progress: &LaunchProgressWriter,
) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    let mut persona = storage::open_store(paths.root())?.require(id).await?;
    let proxy = profile_proxy(paths.root(), &persona, global_proxy)?;
    let targets = match url {
        Some(value) => vec![validate_url(value)?],
        None => startup_tabs(&persona)?,
    };
    let targets = if targets.is_empty() {
        vec!["about:blank".to_string()]
    } else {
        targets
    };
    // A missing proxy must never turn this launch into direct traffic.
    progress.stage(LaunchStage::CheckProxy, None)?;
    proxy.check().await?;
    let proxy_guard =
        if persona.metadata.get("proxy_mode").and_then(Value::as_str) == Some("custom") {
            prepare_browser_launch_with_progress(
                paths.root(),
                id,
                &proxy,
                &global_proxy.browser_url(),
                |stage, detail| progress.stage(stage, detail),
            )
            .await?
        } else {
            None
        };
    progress.stage(LaunchStage::GeoIp, None)?;
    let (saved_geo, _) = checked_location_with_progress(&persona, id, &proxy, || {
        progress.stage(LaunchStage::VerifyGeo, None)
    })
    .await?;
    progress.stage(LaunchStage::StartBrowser, None)?;
    let browser_manager = BrowserManager::with_paths(paths.clone());
    let _browser_guard = browser_manager.runtime_guard()?;
    let installation = browser_manager.prepare_active()?;
    println!(
        "Opening {id} via {}: {}",
        proxy.browser_url(),
        targets.join(", ")
    );
    let options =
        pinned_launch_options(paths, id, &mut persona, &saved_geo, &proxy, &installation).await?;
    let runtime_listener = BrowserRuntime::new(paths.root()).bind(id).await?;
    let mut browser = launch_with_juggler(&options).await?;
    drop(proxy_guard);
    let run = tokio::select! {
        result = async {
            for target in &targets {
                let page = browser.new_page().await?;
                page.goto(target).await?;
            }
            progress.emit(LaunchEvent::Ready)?;
            println!("Browser PID: {:?}. Close the browser to exit.", browser.child.id());
            let status = browser.child.wait().await?;
            println!("Browser exited: {status}");
            Ok::<_, anyhow::Error>(())
        } => result,
        signal = tokio::signal::ctrl_c() => {
            signal?;
            println!("Closing browser");
            Ok(())
        },
        result = runtime_listener.wait_for_close() => {
            result?;
            println!("Closing browser on local request");
            Ok(())
        },
    };
    if run.is_err() || browser.child.try_wait()?.is_none() {
        close_browser(&mut browser).await?;
    }
    run
}

async fn open_reported(
    paths: &Paths,
    id: &str,
    url: Option<&str>,
    global_proxy: &ProxySettings,
    progress_file: Option<PathBuf>,
) -> Result<()> {
    let progress = LaunchProgressWriter::new(progress_file);
    if let Err(error) = open(paths, id, url, global_proxy, &progress).await {
        let _ = progress.emit(LaunchEvent::Failed {
            message: format!("{error:#}"),
        });
        return Err(error);
    }
    Ok(())
}

async fn close_browser(browser: &mut JugglerBrowser) -> Result<()> {
    if let Err(error) = browser.close().await {
        browser
            .kill()
            .await
            .with_context(|| format!("could not close or kill browser; close error: {error}"))?;
    }
    Ok(())
}

fn launch_options(
    paths: &Paths,
    id: &str,
    persona: PersonaRecord,
    proxy: &ProxySettings,
    installation: &ActiveInstallation,
) -> Result<LaunchOptions> {
    let mut options = LaunchOptions {
        os: vec![persona_os(&persona)?],
        persona: Some(persona),
        i_know_what_im_doing: true,
        persistent_profile: Some(paths.profile(id)),
        proxy: Some(ProxyConfig {
            server: proxy.browser_url(),
            username: proxy
                .credentials()
                .map(|(username, _)| username.to_string()),
            password: proxy
                .credentials()
                .map(|(_, password)| password.to_string()),
            ..Default::default()
        }),
        geoip: None,
        block_webrtc: true,
        enable_cache: true,
        headless: HeadlessMode::Off,
        // macOS's Resources/../MacOS path must be normalized for XPCOM loading.
        executable_path: Some(installation.executable_path.clone()),
        install_root: Some(installation.root.clone()),
        ff_version: Some(
            installation
                .version
                .full_string()
                .split('.')
                .next()
                .unwrap_or_default()
                .to_string(),
        ),
        ..Default::default()
    };
    // Apply proxy prefs before Firefox's startup networking; Juggler then
    // configures the same proxy through its native protocol for page traffic.
    proxy.apply_firefox_prefs(&mut options.firefox_user_prefs);
    // Camoufox 152 declares canvas:seed, but Firefox's baseline protection
    // still adds a fresh per-process image-export salt. Disable that layer so
    // the saved canvas/font configuration remains observable across launches.
    options.firefox_user_prefs.insert(
        "privacy.baselineFingerprintingProtection".into(),
        json!(false),
    );
    Ok(options)
}

async fn pinned_launch_options(
    paths: &Paths,
    id: &str,
    persona: &mut PersonaRecord,
    geo: &ProfileGeo,
    proxy: &ProxySettings,
    installation: &ActiveInstallation,
) -> Result<LaunchOptions> {
    let mut options = launch_options(paths, id, persona.clone(), proxy, installation)?;
    if let Some(value) = persona.metadata.get("pinned_launch") {
        let pinned_version = metadata_str(persona, "pinned_browser_version")?;
        if pinned_version != installation.version.full_string() {
            bail!(
                "Camoufox version changed; this pinned identity needs a compatible browser installation"
            );
        }
        let mut prepared: PreparedLaunch =
            serde_json::from_value(value.clone()).context("saved launch identity is invalid")?;
        if prepared.executable_path != *options.executable_path.as_ref().unwrap() {
            bail!(
                "browser installation moved; restore the original browser installation for this pinned identity"
            );
        }
        prepared.proxy = Some(proxy.browser_url());
        proxy.apply_firefox_prefs(&mut prepared.firefox_user_prefs);
        options.prepared_override = Some(prepared);
    } else {
        options.geoip = None;
        options.geolocation_override = Some(geo.as_geolocation()?);
        let mut prepared = prepare(&options).await?;
        // Authentication belongs to the general proxy setting. A pinned
        // fingerprint must not retain an old proxy password.
        prepared.proxy = Some(proxy.browser_url());
        persona
            .metadata
            .insert("pinned_launch".into(), serde_json::to_value(&prepared)?);
        persona.metadata.insert(
            "pinned_browser_version".into(),
            json!(installation.version.full_string()),
        );
        storage::open_store(paths.root())?.save(persona).await?;
        options.prepared_override = Some(prepared);
        println!(
            "Pinned launch identity for {id} in {} / {}",
            geo.country_code, geo.timezone
        );
    }
    Ok(options)
}

async fn checked_location(
    persona: &PersonaRecord,
    id: &str,
    proxy: &ProxySettings,
) -> Result<(ProfileGeo, ProfileGeo)> {
    checked_location_with_progress(persona, id, proxy, || Ok(())).await
}

async fn checked_location_with_progress(
    persona: &PersonaRecord,
    id: &str,
    proxy: &ProxySettings,
    report_verification: impl FnOnce() -> Result<()>,
) -> Result<(ProfileGeo, ProfileGeo)> {
    let saved: ProfileGeo =
        serde_json::from_value(persona.metadata.get("geo").cloned().with_context(|| {
            format!("profile {id} has no saved location; run `browserctl-rs refresh {id}`")
        })?)?;
    let current = ProfileGeo::lookup(proxy).await?;
    report_verification()?;
    let changes = saved.changes_from(&current);
    if !changes.is_empty() {
        bail!(
            "profile {id} location mismatch; browser was not started. Saved: {} / {} / {}. Current proxy exit {}: {} / {} / {}. Changed: {}. Run `browserctl-rs refresh {id}` if intentional",
            saved.country_code,
            saved
                .region_code
                .as_deref()
                .or(saved.region.as_deref())
                .unwrap_or("-"),
            saved.timezone,
            current.ip,
            current.country_code,
            current
                .region_code
                .as_deref()
                .or(current.region.as_deref())
                .unwrap_or("-"),
            current.timezone,
            changes.join("; ")
        );
    }
    println!(
        "Current GeoIP matches {id}: {} | {} | {} | {} | {}",
        current.ip,
        current.country_code,
        current.region.as_deref().unwrap_or("-"),
        current.timezone,
        current.locale
    );
    Ok((saved, current))
}

fn update_launch_geo(paths: &Paths, prepared: &mut PreparedLaunch, geo: &ProfileGeo) -> Result<()> {
    for key in [
        "geolocation:latitude",
        "geolocation:longitude",
        "geolocation:accuracy",
        "timezone",
        "locale:region",
        "locale:language",
        "locale:script",
        "locale:all",
    ] {
        prepared.config.remove(key);
    }
    for (key, value) in geo.as_geolocation()?.as_config()? {
        prepared.config.insert(key, value);
    }
    let user_agent = prepared
        .config
        .get("navigator.userAgent")
        .and_then(Value::as_str)
        .context("saved launch has no user agent")?;
    let target_os = determine_ua_os(user_agent)?;
    prepared
        .env
        .retain(|key, _| !key.starts_with("CAMOU_CONFIG_"));
    prepared.env.extend(get_env_vars(
        &prepared.config,
        target_os,
        Some(&paths.browser()),
    )?);
    Ok(())
}

async fn refresh(paths: &Paths, id: &str, global_proxy: &ProxySettings) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    let mut persona = storage::open_store(paths.root())?.require(id).await?;
    let proxy = profile_proxy(paths.root(), &persona, global_proxy)?;
    proxy.check().await?;
    let geo = ProfileGeo::lookup(&proxy).await?;
    if let Some(value) = persona.metadata.get("pinned_launch") {
        let mut prepared: PreparedLaunch = serde_json::from_value(value.clone())?;
        update_launch_geo(paths, &mut prepared, &geo)?;
        prepared.proxy = Some(proxy.browser_url());
        persona
            .metadata
            .insert("pinned_launch".into(), serde_json::to_value(prepared)?);
    }
    persona
        .metadata
        .insert("geo".into(), serde_json::to_value(&geo)?);
    persona.metadata.remove("pinned_proxy_ip");
    persona.metadata.remove("browserscan_baseline");
    storage::open_store(paths.root())?.save(&persona).await?;
    println!(
        "Refreshed {id}: {} | {} | {} | {}",
        geo.ip,
        geo.country_code,
        geo.region.as_deref().unwrap_or("-"),
        geo.timezone
    );
    Ok(())
}

async fn set_tabs(paths: &Paths, id: &str, urls: &[String], clear: bool) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    let store = storage::open_store(paths.root())?;
    let mut persona = store.require(id).await?;
    if clear || !urls.is_empty() {
        let tabs = if clear {
            Vec::new()
        } else {
            validate_tabs(urls)?
        };
        persona.metadata.insert("tabs".into(), json!(tabs));
        store.save(&persona).await?;
    }
    let tabs = startup_tabs(&persona)?;
    if tabs.is_empty() {
        println!("{id}: no startup tabs");
    } else {
        for (index, tab) in tabs.iter().enumerate() {
            println!("{}. {tab}", index + 1);
        }
    }
    Ok(())
}

async fn delete(paths: &Paths, id: &str, yes: bool, global_proxy: &ProxySettings) -> Result<()> {
    validate_id(id)?;
    if !yes {
        print!("Type {id} to permanently delete this profile and its browser data: ");
        io::stdout().flush()?;
        let mut response = String::new();
        io::stdin().read_line(&mut response)?;
        if response.trim() != id {
            bail!("deletion cancelled");
        }
    }
    ProfileService::with_proxy(paths.root(), global_proxy.clone())?
        .delete(id)
        .await?;
    println!("Deleted {id}");
    Ok(())
}

async fn scan(paths: &Paths, id: &str, global_proxy: &ProxySettings) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    let mut persona = storage::open_store(paths.root())?.require(id).await?;
    let proxy = profile_proxy(paths.root(), &persona, global_proxy)?;
    proxy.check().await?;
    let proxy_guard =
        if persona.metadata.get("proxy_mode").and_then(Value::as_str) == Some("custom") {
            prepare_browser_launch_with_progress(
                paths.root(),
                id,
                &proxy,
                &global_proxy.browser_url(),
                |_, _| Ok(()),
            )
            .await?
        } else {
            None
        };
    let (saved_geo, current_geo) = checked_location(&persona, id, &proxy).await?;
    let expected_ip = current_geo.ip;
    let browser_manager = BrowserManager::with_paths(paths.clone());
    let _browser_guard = browser_manager.runtime_guard()?;
    let installation = browser_manager.prepare_active()?;
    let options =
        pinned_launch_options(paths, id, &mut persona, &saved_geo, &proxy, &installation).await?;
    let runtime_listener = BrowserRuntime::new(paths.root()).bind(id).await?;
    let mut browser = launch_with_juggler(&options).await?;
    drop(proxy_guard);
    let result: Result<()> = tokio::select! {
        result = async {
        let page = browser.new_page().await?;
        page.goto(BROWSERSCAN_URL).await?;
        let artifacts = paths.artifacts();
        fs::create_dir_all(&artifacts)?;
        let scan_files = paths.scan_artifacts(id);
        let screenshot = scan_files.screenshot;
        let report_file = scan_files.report;
        let verification_file = scan_files.verification;
        let fields_file = scan_files.fields;
        let ip_pattern = Regex::new(r"(?m)^IP\s*\n([0-9a-fA-F:.]+)\s*(?:\(|\n)")?;
        let mut report = String::new();
        let mut observed_ip = None;
        let mut scan_identity = None;
        let mut matching_samples = 0;
        for _ in 0..30 {
            report = page
                .evaluate("document.body?.innerText ?? ''")
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
            observed_ip = ip_pattern
                .captures(&report)
                .map(|parts| parts[1].to_string());
            let candidate = ScanIdentity::parse(&report).ok();
            if observed_ip.is_some() && candidate.is_some() {
                if candidate == scan_identity {
                    matching_samples += 1;
                } else {
                    matching_samples = 1;
                    scan_identity = candidate;
                }
                if matching_samples >= 3 {
                    break;
                }
            } else {
                matching_samples = 0;
                scan_identity = None;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        page.screenshot(&screenshot).await?;
        fs::write(&report_file, &report)?;
        let verification = verify_fingerprint(&page, &browser.prepared.config).await?;
        fs::write(
            &verification_file,
            serde_json::to_vec_pretty(&verification)?,
        )?;
        println!("Screenshot: {}", screenshot.display());
        println!("BrowserScan text: {}", report_file.display());
        println!("Fingerprint report: {}", verification_file.display());
        println!("{}", verification.render());
        if !verification.passed() {
            bail!("live fingerprint did not match the prepared config");
        }
        let observed_ip =
            observed_ip.context("BrowserScan did not finish reporting a public IP")?;
        if observed_ip != expected_ip {
            bail!("BrowserScan IP {observed_ip} differs from proxy exit {expected_ip}");
        }
        let expected_timezone = browser
            .prepared
            .config
            .get("timezone")
            .and_then(Value::as_str)
            .context("prepared config has no timezone")?;
        for label in ["Time Zone Based on IP", "Time Zone"] {
            if !report.contains(&format!("{label}\n{expected_timezone}")) {
                bail!("BrowserScan did not confirm {label}: {expected_timezone}");
            }
        }
        for label in ["WebRTC", "WebRTC STUN"] {
            if !report.contains(&format!("{label}\n\ndisabled")) {
                bail!("BrowserScan did not confirm {label} is disabled");
            }
        }
        println!("Proxy exit IP verified: {observed_ip}");
        if matching_samples < 3 {
            bail!("BrowserScan fingerprint did not settle after 60 seconds");
        }
        let scan_identity =
            scan_identity.context("BrowserScan did not finish fingerprint reporting")?;
        if !scan_identity.0.get("Languages").is_some_and(|languages| languages.starts_with(&saved_geo.locale)) {
            bail!("BrowserScan language did not match saved locale {}", saved_geo.locale);
        }
        fs::write(&fields_file, serde_json::to_vec_pretty(&scan_identity)?)?;
        println!("BrowserScan fields: {} (diagnostic only)", fields_file.display());
        Ok(())
        } => result,
        signal = tokio::signal::ctrl_c() => {
            signal?;
            bail!("scan interrupted");
        },
        result = runtime_listener.wait_for_close() => {
            result?;
            bail!("scan closed by browser manager");
        }
    };
    close_browser(&mut browser).await?;
    result
}

async fn run(cli: Cli) -> Result<()> {
    let program_root = Paths::program_root(cli.data_dir.as_deref())?;
    let workspace_lock = Arc::new(WorkspaceLock::shared(&program_root)?);
    let paths = Paths::new(Some(program_root))?;
    storage::open_store(paths.root())?;
    let global_proxy = if matches!(&cli.command, Command::Ui) {
        GeneralSettings::load(paths.root())?.proxy()?
    } else {
        match cli.global_proxy {
            Some(value) => ProxySettings::parse(&value)?,
            None => GeneralSettings::load(paths.root())?.proxy()?,
        }
    };
    match cli.command {
        Command::Fetch => {
            let manager = BrowserManager::with_paths(paths.clone());
            let proxy = if BrowserDownloadSettings::load(paths.root())?.use_global_proxy {
                Some(GeneralSettings::load(paths.root())?.proxy()?)
            } else {
                None
            };
            let release = manager.latest(proxy.as_ref()).await?;
            let versions = manager.installed_versions()?;
            if versions
                .iter()
                .any(|item| item.active && item.version == release.version)
            {
                println!(
                    "Camoufox {} is already installed",
                    release.version.full_string()
                );
                return Ok(());
            }
            if versions
                .iter()
                .any(|item| !item.active && item.version == release.version)
            {
                manager.activate(&release.version)?;
            } else {
                manager
                    .download_and_install(
                        &release,
                        proxy.as_ref(),
                        &DownloadControl::default(),
                        |progress| {
                            if progress.received == 0 || progress.stage != DownloadStage::Download {
                                println!("Browser install: {:?}", progress.stage);
                            }
                        },
                    )
                    .await?;
            }
            manager.prepare_active()?;
            println!("Installed Camoufox in {}", manager.active_path().display());
        }
        Command::Create {
            id,
            proxy,
            os,
            url,
            tabs,
        } => {
            create(
                &paths,
                &id,
                os,
                url.as_deref(),
                &tabs,
                proxy.as_deref(),
                &global_proxy,
            )
            .await?
        }
        Command::List { json } => {
            let summaries = storage::open_store(paths.root())?.list().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summaries)?);
            } else if summaries.is_empty() {
                println!("No profiles. Create one with: browserctl-rs create NAME");
            } else {
                for item in summaries {
                    println!("{:<24} {}", item.id, item.user_agent);
                }
            }
        }
        Command::Show { id } => {
            validate_id(&id)?;
            let persona = storage::open_store(paths.root())?.require(&id).await?;
            let proxy = profile_proxy(paths.root(), &persona, &global_proxy)?;
            let summary = json!({
                "id": persona.id,
                "name": persona.name,
                "created_at": persona.created_at,
                "os": metadata_str(&persona, "os")?,
                "startup_tabs": startup_tabs(&persona)?,
                "proxy_mode": persona.metadata.get("proxy_mode").and_then(Value::as_str).unwrap_or("global"),
                "proxy": proxy.browser_url(),
                "saved_geo": persona.metadata.get("geo"),
                "profile_dir": paths.profile(&id),
                "user_agent": persona.fingerprint.fingerprint.navigator.user_agent,
            });
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        Command::Paths { id } => {
            if let Some(ref id) = id {
                validate_id(id)?;
            }
            let details = json!({
                "data_dir": paths.root(),
                "browser_root": paths.browser_root(),
                "profiles_root": paths.profiles_root(),
                "browser_install": paths.browser(),
                "persona_store": paths.database(),
                "browser_data": id.map(|id| paths.profile(&id)),
            });
            println!("{}", serde_json::to_string_pretty(&details)?);
        }
        Command::Tabs { id, urls, clear } => set_tabs(&paths, &id, &urls, clear).await?,
        Command::Open {
            id,
            url,
            progress_file,
        } => open_reported(&paths, &id, url.as_deref(), &global_proxy, progress_file).await?,
        Command::Scan { id } => scan(&paths, &id, &global_proxy).await?,
        Command::Refresh { id } => refresh(&paths, &id, &global_proxy).await?,
        Command::Delete { id, yes } => delete(&paths, &id, yes, &global_proxy).await?,
        Command::Doctor => {
            println!("Data directory: {}", paths.root().display());
            let browser_manager = BrowserManager::with_paths(paths.clone());
            println!(
                "Browser directory: {}",
                browser_manager.active_path().display()
            );
            let proxy_status = global_proxy.check().await;
            match &proxy_status {
                Ok(()) => println!("Proxy {}: reachable", global_proxy.browser_url()),
                Err(error) => println!("Proxy {}: {error:#}", global_proxy.browser_url()),
            }
            if let Some(version) = browser_manager.active_version()? {
                println!("Browser installed: {}", version.full_string());
            } else {
                println!("Browser: not installed; run `browserctl-rs fetch`");
            }
            proxy_status?;
        }
        Command::Ui => ui::run(paths, global_proxy, workspace_lock)?,
        Command::Proxy { action } => {
            let catalog = ProxyCatalog::new(paths.root());
            match action {
                ProxyAction::List => {
                    let records: Vec<_> = catalog
                        .list()?
                        .into_iter()
                        .map(|proxy| {
                            json!({
                                "id": proxy.id,
                                "name": proxy.name,
                                "url": proxy.url,
                                "has_auth": proxy.credentials.is_some(),
                                "policy": proxy.policy,
                                "ip_switch": proxy.ip_switch,
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&records)?)
                }
                ProxyAction::Set {
                    id,
                    url,
                    name,
                    policy,
                    switch_url,
                    switch_method,
                    rotate_on_start,
                    wait_seconds,
                } => {
                    let normalized_url = ProxySettings::parse(&url)?.browser_url();
                    let credentials = catalog
                        .list()?
                        .into_iter()
                        .find(|proxy| proxy.id == id && proxy.url == normalized_url)
                        .and_then(|proxy| proxy.credentials);
                    let ip_switch = switch_url.map(|url| IpSwitch {
                        url,
                        method: switch_method.into(),
                        on_start: rotate_on_start,
                        wait_seconds,
                    });
                    let saved = catalog.upsert(ManagedProxy {
                        name: name.unwrap_or_else(|| id.clone()),
                        id,
                        url,
                        credentials,
                        policy: policy.into(),
                        ip_switch,
                    })?;
                    println!("Managed proxy {}: {}", saved.id, saved.url);
                }
                ProxyAction::Remove { id } => {
                    catalog.remove(&id)?;
                    println!("Removed managed proxy {id}");
                }
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    run(Cli::parse()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_browser::launch_progress::read_events;

    #[tokio::test]
    async fn open_failure_is_reported_to_native_ui() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(Some(dir.path().to_path_buf())).unwrap();
        let progress_file = dir.path().join("launch.jsonl");
        let proxy = ProxySettings::parse(rust_browser::proxy::DEFAULT_PROXY).unwrap();
        assert!(
            open_reported(&paths, "missing", None, &proxy, Some(progress_file.clone()))
                .await
                .is_err()
        );
        let events = read_events(&progress_file).unwrap();
        assert!(matches!(events.last(), Some(LaunchEvent::Failed { .. })));
    }

    #[test]
    fn rejects_unsafe_profile_ids_and_urls() {
        for id in ["", "../outside", "-bad", "space here", "a/b"] {
            assert!(validate_id(id).is_err(), "{id:?}");
        }
        for url in [
            "file:///etc/passwd",
            "https://user:pass@example.com",
            "relative",
        ] {
            assert!(validate_url(url).is_err(), "{url:?}");
        }
    }

    #[test]
    fn browserscan_identity_requires_complete_values() {
        let mut report = String::new();
        for label in SCAN_IDENTITY_FIELDS {
            let value = if [
                "visitor ID",
                "Canvas",
                "WebGL",
                "WebGL Report",
                "Audio",
                "Client Rects",
                "WebGPU Report",
            ]
            .contains(label)
            {
                "1234ABCD"
            } else {
                "stable value"
            };
            report.push_str(&format!("{label}\n{value}\n"));
        }
        assert_eq!(
            ScanIdentity::parse(&report).unwrap().0.len(),
            SCAN_IDENTITY_FIELDS.len()
        );
        assert!(ScanIdentity::parse(&report.replace("Canvas\n1234ABCD", "Canvas\nWebGL")).is_err());
    }
}
