use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use camoufox::builder::{HeadlessMode, LaunchOptions, PreparedLaunch, ProxyConfig, prepare};
use camoufox_core::fingerprint::FingerprintRequest;
use camoufox_core::os::SupportedOs;
use camoufox_core::persona::PersonaRecord;
use camoufox_geoip::public_ip;
use camoufox_juggler::{JugglerBrowser, launch_with_juggler, verify_fingerprint};
use camoufox_pkgman::{
    CamoufoxFetcher, install_dir, installed_ver_str, launch_path, set_install_dir,
};
use camoufox_store::{FileStore, PersonaStore};
use clap::{Parser, Subcommand, ValueEnum};
use fs2::FileExt;
use rand::Rng;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use url::Url;

const DEFAULT_PROXY: &str = "socks5://127.0.0.1:12334";
const DEFAULT_URL: &str = "https://www.browserscan.net/";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

    fn compare(&self, expected: &Self) -> Result<()> {
        let changes: Vec<String> = expected
            .0
            .iter()
            .filter_map(|(label, old)| {
                let current = self.0.get(label);
                (current != Some(old)).then(|| format!("{label}: {old:?} -> {current:?}"))
            })
            .collect();
        if !changes.is_empty() {
            bail!("BrowserScan fingerprint changed:\n{}", changes.join("\n"));
        }
        Ok(())
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
        #[arg(long, value_enum, default_value_t = BrowserOs::Macos)]
        os: BrowserOs,
        #[arg(long, default_value = DEFAULT_URL)]
        url: String,
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
    /// Open a persistent, visible browser through local SOCKS5.
    Open {
        id: String,
        #[arg(long)]
        url: Option<String>,
    },
    /// Capture BrowserScan plus camoufox-rust's fingerprint verification report.
    Scan { id: String },
    /// Explicitly rotate the pinned launch identity to the current proxy exit.
    Refresh { id: String },
    /// Permanently delete a profile and its browser data.
    Delete {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Check local proxy and browser installation without launching it.
    Doctor,
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

#[derive(Clone)]
struct Paths {
    root: PathBuf,
}

impl Paths {
    fn new(override_dir: Option<PathBuf>) -> Result<Self> {
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

    fn browser(&self) -> PathBuf {
        self.root.join("browser")
    }
    fn personas(&self) -> PathBuf {
        self.root.join("personas")
    }
    fn profile(&self, id: &str) -> PathBuf {
        self.root.join("profiles").join(id)
    }
    fn lock(&self, id: &str) -> PathBuf {
        self.root.join("locks").join(format!("{id}.lock"))
    }
    fn store(&self) -> PersonaStore {
        PersonaStore::new(Box::new(FileStore::new(self.personas())))
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
    fs::create_dir_all(paths.root.join("locks"))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.lock(id))?;
    lock.try_lock_exclusive()
        .with_context(|| format!("profile {id} is already open"))?;
    Ok(lock)
}

fn prepare_browser_files(paths: &Paths) -> Result<()> {
    let root = paths.browser();
    if !root.join("version.json").exists() {
        bail!(
            "Camoufox is not installed in {}; run `browserctl-rs fetch`",
            root.display()
        );
    }
    // camoufox-rust 0.9.1 validates properties at install_root/properties.json,
    // while the macOS archive places them inside the app bundle Resources.
    #[cfg(target_os = "macos")]
    {
        let source = root.join("Camoufox.app/Contents/Resources/properties.json");
        if !source.exists() {
            bail!("missing Camoufox properties at {}", source.display());
        }
        for target in [
            root.join("properties.json"),
            root.join("Camoufox.app/Contents/MacOS/properties.json"),
        ] {
            if !target.exists() || fs::read(&source)? != fs::read(&target)? {
                fs::copy(&source, &target)?;
            }
        }
    }
    Ok(())
}

async fn check_proxy() -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(3),
        TcpStream::connect("127.0.0.1:12334"),
    )
    .await
    .context("timed out connecting to local SOCKS5 proxy at 127.0.0.1:12334")?
    .context("local SOCKS5 proxy at 127.0.0.1:12334 is unavailable")?;
    Ok(())
}

async fn create(paths: &Paths, id: &str, os: BrowserOs, url: &str, tabs: &[String]) -> Result<()> {
    validate_id(id)?;
    let url = validate_url(url)?;
    let tabs = if tabs.is_empty() {
        vec![url]
    } else {
        validate_tabs(tabs)?
    };
    let _lock = profile_lock(paths, id)?;
    let store = paths.store();
    if store.load(id).await?.is_some() {
        bail!("profile {id} already exists");
    }
    let request = FingerprintRequest {
        operating_systems: Some(vec![os.supported()]),
        seed: Some(rand::thread_rng().r#gen::<u64>()),
        ..Default::default()
    };
    let mut persona = PersonaRecord::generate(id, &request)?;
    persona.name = Some(id.to_string());
    persona.metadata.insert("os".into(), json!(os.as_str()));
    persona.metadata.insert("tabs".into(), json!(tabs));
    store.save(&persona).await?;
    println!("Created {id} ({})", os.as_str());
    println!("Browser data: {}", paths.profile(id).display());
    Ok(())
}

async fn open(paths: &Paths, id: &str, url: Option<&str>) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    let mut persona = paths.store().require(id).await?;
    let targets = match url {
        Some(value) => vec![validate_url(value)?],
        None => startup_tabs(&persona)?,
    };
    let targets = if targets.is_empty() {
        vec![DEFAULT_URL.to_string()]
    } else {
        targets
    };
    // A missing proxy must never turn this launch into direct traffic.
    check_proxy().await?;
    let proxy_ip = public_ip(Some(DEFAULT_PROXY)).await?;
    prepare_browser_files(paths)?;
    println!("Opening {id} via {DEFAULT_PROXY}: {}", targets.join(", "));
    let options = pinned_launch_options(paths, id, &mut persona, &proxy_ip).await?;
    let mut browser = launch_with_juggler(&options).await?;
    let run = tokio::select! {
        result = async {
            for target in &targets {
                let page = browser.new_page().await?;
                page.goto(target).await?;
            }
            println!("Browser PID: {:?}. Close the browser to exit.", browser.child.id());
            let status = browser.child.wait().await?;
            println!("Browser exited: {status}");
            Ok::<_, anyhow::Error>(())
        } => result,
        signal = tokio::signal::ctrl_c() => {
            signal?;
            println!("Closing browser");
            Ok(())
        }
    };
    if run.is_err() || browser.child.try_wait()?.is_none() {
        close_browser(&mut browser).await?;
    }
    run
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

async fn launch_options(paths: &Paths, id: &str, persona: PersonaRecord) -> Result<LaunchOptions> {
    let mut options = LaunchOptions {
        os: vec![persona_os(&persona)?],
        persona: Some(persona),
        i_know_what_im_doing: true,
        persistent_profile: Some(paths.profile(id)),
        proxy: Some(ProxyConfig {
            server: DEFAULT_PROXY.into(),
            ..Default::default()
        }),
        geoip: Some(None),
        block_webrtc: true,
        enable_cache: true,
        headless: HeadlessMode::Off,
        // macOS's Resources/../MacOS path must be normalized for XPCOM loading.
        executable_path: Some(launch_path().await?.canonicalize()?),
        ..Default::default()
    };
    // Apply proxy prefs before Firefox's startup networking; Juggler then
    // configures the same proxy through its native protocol for page traffic.
    options
        .firefox_user_prefs
        .insert("network.proxy.type".into(), json!(1));
    options
        .firefox_user_prefs
        .insert("network.proxy.socks".into(), json!("127.0.0.1"));
    options
        .firefox_user_prefs
        .insert("network.proxy.socks_port".into(), json!(12334));
    options
        .firefox_user_prefs
        .insert("network.proxy.socks_version".into(), json!(5));
    options
        .firefox_user_prefs
        .insert("network.proxy.socks_remote_dns".into(), json!(true));
    options
        .firefox_user_prefs
        .insert("network.proxy.no_proxies_on".into(), json!(""));
    options.firefox_user_prefs.insert(
        "network.proxy.allow_hijacking_localhost".into(),
        json!(true),
    );
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
    proxy_ip: &str,
) -> Result<LaunchOptions> {
    let mut options = launch_options(paths, id, persona.clone()).await?;
    if let Some(value) = persona.metadata.get("pinned_launch") {
        let pinned_ip = metadata_str(persona, "pinned_proxy_ip")?;
        if pinned_ip != proxy_ip {
            bail!(
                "proxy exit changed from {pinned_ip} to {proxy_ip}; run `browserctl-rs refresh {id}` to rotate this identity"
            );
        }
        let pinned_version = metadata_str(persona, "pinned_browser_version")?;
        if pinned_version != installed_ver_str()? {
            bail!(
                "Camoufox version changed; run `browserctl-rs refresh {id}` to rotate this identity"
            );
        }
        let prepared: PreparedLaunch =
            serde_json::from_value(value.clone()).context("saved launch identity is invalid")?;
        if prepared.executable_path != *options.executable_path.as_ref().unwrap() {
            bail!(
                "browser installation moved; run `browserctl-rs refresh {id}` to rebuild this identity"
            );
        }
        options.prepared_override = Some(prepared);
    } else {
        options.geoip = Some(Some(proxy_ip.to_string()));
        let prepared = prepare(&options).await?;
        persona
            .metadata
            .insert("pinned_launch".into(), serde_json::to_value(&prepared)?);
        persona
            .metadata
            .insert("pinned_proxy_ip".into(), json!(proxy_ip));
        persona
            .metadata
            .insert("pinned_browser_version".into(), json!(installed_ver_str()?));
        paths.store().save(persona).await?;
        options.prepared_override = Some(prepared);
        println!("Pinned launch identity for {id} at proxy exit {proxy_ip}");
    }
    Ok(options)
}

async fn refresh(paths: &Paths, id: &str) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    check_proxy().await?;
    prepare_browser_files(paths)?;
    let proxy_ip = public_ip(Some(DEFAULT_PROXY)).await?;
    let mut persona = paths.store().require(id).await?;
    for key in [
        "pinned_launch",
        "pinned_proxy_ip",
        "pinned_browser_version",
        "browserscan_baseline",
    ] {
        persona.metadata.remove(key);
    }
    pinned_launch_options(paths, id, &mut persona, &proxy_ip).await?;
    println!(
        "Refreshed {id}; run `browserctl-rs scan {id}` to establish a new BrowserScan baseline"
    );
    Ok(())
}

async fn set_tabs(paths: &Paths, id: &str, urls: &[String], clear: bool) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    let store = paths.store();
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

async fn delete(paths: &Paths, id: &str, yes: bool) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    let store = paths.store();
    store.require(id).await?;
    if !yes {
        print!("Type {id} to permanently delete this profile and its browser data: ");
        io::stdout().flush()?;
        let mut response = String::new();
        io::stdin().read_line(&mut response)?;
        if response.trim() != id {
            bail!("deletion cancelled");
        }
    }
    let profile = paths.profile(id);
    let quarantined = if profile.exists() {
        let trash = paths.root.join("trash");
        fs::create_dir_all(&trash)?;
        let destination = trash.join(format!("{id}-{}", rand::thread_rng().r#gen::<u64>()));
        fs::rename(&profile, &destination)?;
        Some(destination)
    } else {
        None
    };
    if let Err(error) = store.delete(id).await {
        if let Some(destination) = quarantined.as_ref() {
            fs::rename(destination, &profile).with_context(|| {
                format!(
                    "could not restore {} after store error: {error}",
                    profile.display()
                )
            })?;
        }
        return Err(error.into());
    }
    if let Some(destination) = quarantined {
        fs::remove_dir_all(&destination).with_context(|| {
            format!(
                "profile deleted, but could not remove quarantined browser data at {}",
                destination.display()
            )
        })?;
    }
    for suffix in ["browserscan.png", "browserscan.txt", "fingerprint.json"] {
        let artifact = paths.root.join("artifacts").join(format!("{id}-{suffix}"));
        match fs::remove_file(artifact) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    println!("Deleted {id}");
    Ok(())
}

async fn scan(paths: &Paths, id: &str) -> Result<()> {
    validate_id(id)?;
    let _lock = profile_lock(paths, id)?;
    let mut persona = paths.store().require(id).await?;
    check_proxy().await?;
    let expected_ip = public_ip(Some(DEFAULT_PROXY)).await?;
    prepare_browser_files(paths)?;
    let options = pinned_launch_options(paths, id, &mut persona, &expected_ip).await?;
    let mut browser = launch_with_juggler(&options).await?;
    let result: Result<()> = tokio::select! {
        result = async {
        let page = browser.new_page().await?;
        page.goto(DEFAULT_URL).await?;
        let artifacts = paths.root.join("artifacts");
        fs::create_dir_all(&artifacts)?;
        let screenshot = artifacts.join(format!("{id}-browserscan.png"));
        let report_file = artifacts.join(format!("{id}-browserscan.txt"));
        let verification_file = artifacts.join(format!("{id}-fingerprint.json"));
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
        if let Some(saved) = persona.metadata.get("browserscan_baseline") {
            let baseline: ScanIdentity = serde_json::from_value(saved.clone())?;
            scan_identity.compare(&baseline)?;
            println!(
                "BrowserScan identity matches the saved baseline ({} fields)",
                SCAN_IDENTITY_FIELDS.len()
            );
        } else {
            persona.metadata.insert(
                "browserscan_baseline".into(),
                serde_json::to_value(&scan_identity)?,
            );
            paths.store().save(&persona).await?;
            println!(
                "Saved BrowserScan identity baseline ({} fields)",
                SCAN_IDENTITY_FIELDS.len()
            );
        }
        Ok(())
        } => result,
        signal = tokio::signal::ctrl_c() => {
            signal?;
            bail!("scan interrupted");
        }
    };
    close_browser(&mut browser).await?;
    result
}

async fn run(cli: Cli) -> Result<()> {
    let paths = Paths::new(cli.data_dir)?;
    // Separate this installation from the Python Camoufox environment.
    set_install_dir(Some(paths.browser()));
    match cli.command {
        Command::Fetch => {
            CamoufoxFetcher::new().install().await?;
            prepare_browser_files(&paths)?;
            println!("Installed Camoufox in {}", install_dir().display());
        }
        Command::Create { id, os, url, tabs } => create(&paths, &id, os, &url, &tabs).await?,
        Command::List { json } => {
            let summaries = paths.store().list().await?;
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
            let persona = paths.store().require(&id).await?;
            let summary = json!({
                "id": persona.id,
                "name": persona.name,
                "created_at": persona.created_at,
                "os": metadata_str(&persona, "os")?,
                "startup_tabs": startup_tabs(&persona)?,
                "proxy": DEFAULT_PROXY,
                "pinned_proxy_ip": persona.metadata.get("pinned_proxy_ip"),
                "browserscan_baseline": persona.metadata.contains_key("browserscan_baseline"),
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
                "data_dir": paths.root,
                "browser_install": paths.browser(),
                "persona_store": paths.personas(),
                "browser_data": id.map(|id| paths.profile(&id)),
            });
            println!("{}", serde_json::to_string_pretty(&details)?);
        }
        Command::Tabs { id, urls, clear } => set_tabs(&paths, &id, &urls, clear).await?,
        Command::Open { id, url } => open(&paths, &id, url.as_deref()).await?,
        Command::Scan { id } => scan(&paths, &id).await?,
        Command::Refresh { id } => refresh(&paths, &id).await?,
        Command::Delete { id, yes } => delete(&paths, &id, yes).await?,
        Command::Doctor => {
            println!("Data directory: {}", paths.root.display());
            println!("Browser directory: {}", install_dir().display());
            let proxy_status = check_proxy().await;
            match &proxy_status {
                Ok(()) => println!("Proxy {DEFAULT_PROXY}: reachable"),
                Err(error) => println!("Proxy {DEFAULT_PROXY}: {error:#}"),
            }
            if paths.browser().join("version.json").exists() {
                println!("Browser installed: {}", installed_ver_str()?);
            } else {
                println!("Browser: not installed; run `browserctl-rs fetch`");
            }
            proxy_status?;
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
    fn browserscan_identity_requires_complete_values_and_detects_drift() {
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
        let baseline = ScanIdentity::parse(&report).unwrap();
        let changed =
            ScanIdentity::parse(&report.replace("Canvas\n1234ABCD", "Canvas\nDEADBEEF")).unwrap();
        assert!(changed.compare(&baseline).is_err());
        assert!(ScanIdentity::parse(&report.replace("Canvas\n1234ABCD", "Canvas\nWebGL")).is_err());
    }

    #[tokio::test]
    async fn saved_persona_is_stable_and_duplicate_creation_fails() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(Some(dir.path().to_path_buf())).unwrap();
        create(
            &paths,
            "account-a",
            BrowserOs::Windows,
            "https://example.com",
            &[],
        )
        .await
        .unwrap();
        let first = paths.store().require("account-a").await.unwrap();
        assert_eq!(metadata_str(&first, "os").unwrap(), "windows");
        assert_eq!(startup_tabs(&first).unwrap(), ["https://example.com/"]);
        assert!(
            create(
                &paths,
                "account-a",
                BrowserOs::Linux,
                "https://example.org",
                &[]
            )
            .await
            .is_err()
        );
        let second = paths.store().require("account-a").await.unwrap();
        assert_eq!(first.seed, second.seed);
        assert_eq!(
            first.fingerprint.fingerprint.navigator.user_agent,
            second.fingerprint.fingerprint.navigator.user_agent
        );
        set_tabs(
            &paths,
            "account-a",
            &["https://www.vinted.fr".into()],
            false,
        )
        .await
        .unwrap();
        let third = paths.store().require("account-a").await.unwrap();
        assert_eq!(startup_tabs(&third).unwrap(), ["https://www.vinted.fr/"]);
        fs::create_dir_all(paths.profile("account-a")).unwrap();
        fs::write(paths.profile("account-a").join("cookies.sqlite"), b"saved").unwrap();
        fs::create_dir_all(paths.root.join("artifacts")).unwrap();
        let artifact = paths.root.join("artifacts/account-a-browserscan.txt");
        fs::write(&artifact, b"scan").unwrap();
        delete(&paths, "account-a", true).await.unwrap();
        assert!(paths.store().load("account-a").await.unwrap().is_none());
        assert!(!paths.profile("account-a").exists());
        assert!(!artifact.exists());
    }
}
