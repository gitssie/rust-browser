//! Native profile home. Layout and interaction patterns follow rv's GPUI app.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    Disableable, Icon, IconName, Root, Sizable, StyledExt, TitleBar,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use rust_browser::browser_manager::{
    BrowserManager, DownloadControl, DownloadProgress, DownloadStage, InstalledVersion, Release,
};
use rust_browser::directory_management::{copy_storage, remove_source_storage};
use rust_browser::geo::ProfileGeo;
use rust_browser::launch_progress::{LaunchEvent, LaunchStage, read_events};
use rust_browser::paths::AppPaths;
use rust_browser::profiles::{
    CreateProfile, ProfileGeoInput, ProfileListFilter, ProfileOs, ProfileQuery, ProfileService,
    ProfileView, ProxyChoice, ProxyModeFilter, UpdateProfile,
};
use rust_browser::proxy::ProxySettings;
use rust_browser::proxy_management::{
    IpSwitch, ManagedProxy, ProxyCatalog, ProxyCredentials, ProxyPolicy, SwitchMethod,
};
use rust_browser::runtime::BrowserRuntime;
use rust_browser::settings::{BrowserDownloadSettings, DataDirectories, GeneralSettings};
use rust_browser::tags::TagCatalog;
use rust_browser::workspace_lock::WorkspaceLock;

use crate::Paths;

const GREEN: u32 = 0x008c68;
const GREEN_PALE: u32 = 0xe6f7ef;
const INK: u32 = 0x172a3e;
const MUTED: u32 = 0x718197;
const LINE: u32 = 0xe1e8ef;
const ROW_ALT: u32 = 0xf8fbfd;
const SELECTED: u32 = 0xeafaf5;
const RED: u32 = 0xd92d20;

struct LaunchUi {
    id: String,
    name: String,
    location: String,
    progress_file: PathBuf,
    events_seen: usize,
    stage: Option<LaunchStage>,
    close_count: Option<usize>,
    switch_ip: bool,
    error: Option<String>,
    visible: bool,
}

impl LaunchUi {
    fn steps(&self) -> Vec<LaunchStage> {
        let mut steps = vec![LaunchStage::CheckProxy];
        if self.close_count.is_some() {
            steps.push(LaunchStage::ClosePeers);
        }
        if self.switch_ip {
            steps.push(LaunchStage::SwitchIp);
        }
        steps.extend([
            LaunchStage::GeoIp,
            LaunchStage::VerifyGeo,
            LaunchStage::StartBrowser,
        ]);
        steps
    }

    fn apply(&mut self, event: &LaunchEvent) -> bool {
        match event {
            LaunchEvent::Stage { stage, detail } => {
                self.stage = Some(*stage);
                if *stage == LaunchStage::ClosePeers {
                    self.close_count = detail.as_deref().and_then(|value| value.parse().ok());
                }
                if *stage == LaunchStage::SwitchIp {
                    self.switch_ip = true;
                }
                false
            }
            LaunchEvent::Ready => true,
            LaunchEvent::Failed { message } => {
                self.error = Some(message.clone());
                self.visible = true;
                true
            }
        }
    }
}

#[derive(Clone)]
enum Dialog {
    None,
    Create,
    Edit(String),
    Delete(String),
    CloseAll,
}

#[derive(Clone)]
enum TagPopup {
    Filter,
    Profile(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CreateMode {
    Smart,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Proxies,
    Tags,
    Data,
}

#[derive(Clone, Copy)]
enum StorageKind {
    Browser,
    Profiles,
}

impl StorageKind {
    fn names(self) -> &'static [&'static str] {
        match self {
            Self::Browser => &["browser", "browser-versions", "browser-switch-previous"],
            Self::Profiles => &["profiles", "trash"],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Browser => "浏览器文件工作目录",
            Self::Profiles => "用户数据工作目录",
        }
    }
}

struct InitialSettings {
    general: GeneralSettings,
    browser_download: BrowserDownloadSettings,
    workspace_lock: Arc<WorkspaceLock>,
}

pub fn run(
    paths: Paths,
    global_proxy: ProxySettings,
    workspace_lock: Arc<WorkspaceLock>,
) -> Result<()> {
    let service = ProfileService::with_proxy(paths.root(), global_proxy.clone())?;
    let runtime = BrowserRuntime::new(paths.root());
    let tokio = tokio::runtime::Handle::current();
    let initial_settings = InitialSettings {
        general: GeneralSettings::load(paths.root())?,
        browser_download: BrowserDownloadSettings::load(paths.root())?,
        workspace_lock,
    };
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx| {
        gpui_component::init(cx);
        let bounds = Bounds::centered(None, size(px(1460.), px(900.)), cx);
        let mut options = TitleBar::window_options();
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
        options.window_min_size = Some(size(px(1050.), px(600.)));
        options.app_id = Some("app.cazer.browser".into());
        cx.spawn(async move |cx| {
            cx.open_window(options, move |window, cx| {
                let home = cx.new(|cx| {
                    BrowserHome::new(
                        service,
                        runtime,
                        tokio,
                        initial_settings,
                        global_proxy,
                        window,
                        cx,
                    )
                });
                cx.new(|cx| Root::new(home, window, cx))
            })
            .expect("open browser manager");
        })
        .detach();
    });
    Ok(())
}

struct BrowserHome {
    paths: AppPaths,
    service: ProfileService,
    runtime: BrowserRuntime,
    tokio: tokio::runtime::Handle,
    global_proxy: ProxySettings,
    workspace_lock: Arc<WorkspaceLock>,
    settings_tab: Option<SettingsTab>,
    general: GeneralSettings,
    general_host: Entity<InputState>,
    general_port: Entity<InputState>,
    general_username: Entity<InputState>,
    general_password: Entity<InputState>,
    general_timeout: Entity<InputState>,
    general_generation: u64,
    general_status: String,
    general_test: String,
    general_testing: bool,
    general_test_generation: u64,
    storage_busy: bool,
    storage_status: String,
    browser_download_settings: BrowserDownloadSettings,
    browser_versions: Vec<InstalledVersion>,
    browser_latest: Option<Release>,
    browser_checking: bool,
    browser_busy: bool,
    browser_status: String,
    browser_progress: Option<DownloadProgress>,
    browser_progress_shared: Option<Arc<Mutex<Option<DownloadProgress>>>>,
    browser_control: Option<DownloadControl>,
    browser_versions_expanded: bool,
    browser_delete_pending: Option<String>,
    managed: Vec<ManagedProxy>,
    managed_selected: Option<String>,
    managed_id: Entity<InputState>,
    managed_name: Entity<InputState>,
    managed_url: Entity<InputState>,
    managed_username: Entity<InputState>,
    managed_password: Entity<InputState>,
    managed_switch_url: Entity<InputState>,
    managed_wait: Entity<InputState>,
    managed_policy: ProxyPolicy,
    managed_method: SwitchMethod,
    managed_on_start: bool,
    managed_status: String,
    managed_tests: HashMap<String, String>,
    managed_testing: HashSet<String>,
    managed_generation: u64,
    managed_delete_pending: bool,
    search: Entity<InputState>,
    form_id: Entity<InputState>,
    form_name: Entity<InputState>,
    form_url: Entity<InputState>,
    form_proxy: Entity<InputState>,
    form_country_code: Entity<InputState>,
    form_country: Entity<InputState>,
    form_region: Entity<InputState>,
    form_city: Entity<InputState>,
    form_timezone: Entity<InputState>,
    form_locale: Entity<InputState>,
    form_latitude: Entity<InputState>,
    form_longitude: Entity<InputState>,
    tag_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    rows: Vec<ProfileView>,
    page: usize,
    total: usize,
    loading: bool,
    has_more: bool,
    generation: u64,
    proxy_filter: Option<ProxyModeFilter>,
    filter_tags: Vec<String>,
    all_tags: Vec<String>,
    selected_tag: Option<String>,
    tag_status: String,
    running: HashSet<String>,
    selected: HashSet<String>,
    popup: Option<TagPopup>,
    popup_position: Point<Pixels>,
    dialog: Dialog,
    form_custom_proxy: bool,
    form_os: ProfileOs,
    create_mode: CreateMode,
    geo_preview: Option<ProfileGeo>,
    geo_loading: bool,
    geo_generation: u64,
    geo_error: String,
    busy: bool,
    status: String,
    spawned: HashMap<String, Child>,
    launch: Option<LaunchUi>,
}

impl BrowserHome {
    fn browser_manager(&self) -> BrowserManager {
        BrowserManager::with_paths(self.paths.clone())
    }

    fn choose_storage_root(&mut self, kind: StorageKind, cx: &mut Context<Self>) {
        if self.storage_busy {
            return;
        }
        if self.busy || self.browser_busy || self.browser_checking || self.launch.is_some() {
            self.storage_status = "请等待当前操作结束后再更改目录".into();
            cx.notify();
            return;
        }
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(format!("选择{}", kind.label()).into()),
        });
        cx.spawn(async move |this, cx| match selected.await {
            Ok(Ok(Some(mut paths))) => {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| this.change_storage_root(kind, path, cx));
                }
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                let _ = this.update(cx, |this, cx| {
                    this.storage_status = format!("选择目录失败：{error}");
                    cx.notify();
                });
            }
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    this.storage_status = format!("选择目录失败：{error}");
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn change_storage_root(&mut self, kind: StorageKind, target: PathBuf, cx: &mut Context<Self>) {
        let source = match kind {
            StorageKind::Browser => self.paths.browser_root().to_path_buf(),
            StorageKind::Profiles => self.paths.profiles_root().to_path_buf(),
        };
        if source == target {
            return;
        }
        self.storage_busy = true;
        self.storage_status = format!("正在搬迁{}…", kind.label());
        cx.notify();
        let root = self.paths.root().to_path_buf();
        let service = self.service.clone();
        let runtime = self.runtime.clone();
        let tokio = self.tokio.clone();
        let proxy = self.global_proxy.clone();
        let workspace_lock = self.workspace_lock.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    workspace_lock.with_exclusive(|| {
                        let ids = tokio.block_on(service.list_ids())?;
                        for id in ids {
                            if tokio.block_on(runtime.is_running(&id)) {
                                anyhow::bail!("请先关闭所有浏览器再搬迁数据目录");
                            }
                        }
                        let target = target.canonicalize()?;
                        let mut changed = DataDirectories::load(&root)?;
                        match kind {
                            StorageKind::Browser => {
                                changed.browser_root = (target != root).then(|| target.clone())
                            }
                            StorageKind::Profiles => {
                                changed.profiles_root = (target != root).then(|| target.clone())
                            }
                        }
                        let next_paths = AppPaths::with_directories(root.clone(), changed.clone())?;
                        copy_storage(&source, &target, kind.names())?;
                        let next = match ProfileService::with_paths(next_paths, proxy) {
                            Ok(next) => next,
                            Err(error) => {
                                if let Err(cleanup) =
                                    remove_source_storage(&target, &source, kind.names())
                                {
                                    anyhow::bail!(
                                        "目录校验失败：{error}；目标目录清理失败：{cleanup}"
                                    );
                                }
                                return Err(error.into());
                            }
                        };
                        if let Err(error) = changed.save(&root) {
                            if let Err(cleanup) =
                                remove_source_storage(&target, &source, kind.names())
                            {
                                anyhow::bail!(
                                    "保存目录设置失败：{error}；目标目录清理失败：{cleanup}"
                                );
                            }
                            return Err(error);
                        }
                        let paths = next.paths().clone();
                        let cleanup = remove_source_storage(&source, &target, kind.names());
                        Ok((paths, next, cleanup))
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.storage_busy = false;
                match result {
                    Ok((paths, service, cleanup)) => {
                        this.paths = paths;
                        this.service = service;
                        this.refresh_browser_versions();
                        this.reload(cx);
                        this.storage_status = match cleanup {
                            Ok(()) => format!("{}已搬迁", kind.label()),
                            Err(error) => format!("新目录已启用，但旧目录清理失败：{error}"),
                        };
                    }
                    Err(error) => {
                        this.storage_status = format!("目录搬迁失败：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_browser_versions(&mut self) {
        match self.browser_manager().installed_versions() {
            Ok(versions) => self.browser_versions = versions,
            Err(error) => self.browser_status = format!("读取已安装版本失败：{error}"),
        }
    }

    fn browser_download_proxy(&self) -> Result<Option<ProxySettings>> {
        if self.browser_download_settings.use_global_proxy {
            Ok(Some(
                GeneralSettings::load(self.service.data_dir())?.proxy()?,
            ))
        } else {
            Ok(None)
        }
    }

    fn check_browser_update(&mut self, cx: &mut Context<Self>) {
        if self.browser_checking || self.browser_busy {
            return;
        }
        let proxy = match self.browser_download_proxy() {
            Ok(proxy) => proxy,
            Err(error) => {
                self.browser_status = format!("代理配置无效：{error}");
                cx.notify();
                return;
            }
        };
        self.browser_checking = true;
        self.browser_status = "正在检查 Camoufox 版本…".into();
        let manager = self.browser_manager();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { tokio.block_on(async { manager.latest(proxy.as_ref()).await }) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.browser_checking = false;
                match result {
                    Ok(release) => {
                        let current = this.browser_versions.iter().find(|version| version.active);
                        this.browser_status =
                            if current.is_some_and(|current| current.version == release.version) {
                                "当前已是最新支持版本".into()
                            } else {
                                format!("发现版本 {}", release.version.full_string())
                            };
                        this.browser_latest = Some(release);
                    }
                    Err(error) => this.browser_status = format!("检查更新失败：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_browser_download(&mut self, cx: &mut Context<Self>) {
        if self.browser_busy || self.browser_checking {
            return;
        }
        if !self.running.is_empty()
            || !self.spawned.is_empty()
            || self.busy
            || self.launch.is_some()
        {
            self.browser_status = "请先关闭正在运行的浏览器，再安装版本".into();
            cx.notify();
            return;
        }
        let Some(release) = self.browser_latest.clone() else {
            self.browser_status = "请先检查更新".into();
            cx.notify();
            return;
        };
        if self
            .browser_versions
            .iter()
            .any(|item| item.active && item.version == release.version)
        {
            self.browser_status = "当前已安装此版本".into();
            cx.notify();
            return;
        }
        if self
            .browser_versions
            .iter()
            .any(|item| !item.active && item.version == release.version)
        {
            self.activate_browser_version(release.version, cx);
            return;
        }
        let proxy = match self.browser_download_proxy() {
            Ok(proxy) => proxy,
            Err(error) => {
                self.browser_status = format!("代理配置无效：{error}");
                cx.notify();
                return;
            }
        };
        let control = DownloadControl::default();
        let shared = Arc::new(Mutex::new(None));
        self.browser_busy = true;
        self.browser_progress = Some(DownloadProgress {
            stage: DownloadStage::Download,
            received: 0,
            total: None,
        });
        self.browser_progress_shared = Some(shared.clone());
        self.browser_control = Some(control.clone());
        self.browser_status = "正在下载浏览器…".into();
        self.poll_browser_progress(cx);
        let manager = self.browser_manager();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        manager
                            .download_and_install(&release, proxy.as_ref(), &control, |progress| {
                                if let Ok(mut current) = shared.lock() {
                                    *current = Some(progress);
                                }
                            })
                            .await
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.browser_busy = false;
                this.browser_control = None;
                this.browser_progress_shared = None;
                this.browser_progress = None;
                this.browser_status = match result {
                    Ok(()) => "Camoufox 安装完成".into(),
                    Err(error) => format!("浏览器安装失败：{error}"),
                };
                this.refresh_browser_versions();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn poll_browser_progress(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.browser_busy {
                    return;
                }
                if let Some(shared) = &this.browser_progress_shared
                    && let Ok(value) = shared.lock()
                {
                    this.browser_progress = value.clone();
                }
                this.poll_browser_progress(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn activate_browser_version(
        &mut self,
        version: camoufox_pkgman::version::CamoufoxVersion,
        cx: &mut Context<Self>,
    ) {
        if self.browser_busy || !self.running.is_empty() || self.busy || self.launch.is_some() {
            self.browser_status = "请先关闭正在运行的浏览器，再切换版本".into();
        } else {
            self.browser_status = match self.browser_manager().activate(&version) {
                Ok(()) => format!("已切换到 {}", version.full_string()),
                Err(error) => format!("切换失败：{error}"),
            };
            self.refresh_browser_versions();
        }
        cx.notify();
    }

    fn delete_browser_version(
        &mut self,
        version: camoufox_pkgman::version::CamoufoxVersion,
        cx: &mut Context<Self>,
    ) {
        if self.browser_busy {
            self.browser_status = "请等待浏览器安装结束".into();
        } else {
            self.browser_status = match self.browser_manager().delete_archived(&version) {
                Ok(()) => format!("已删除 {}", version.full_string()),
                Err(error) => format!("删除失败：{error}"),
            };
            self.refresh_browser_versions();
        }
        cx.notify();
    }

    fn general_from_inputs(&self, cx: &App) -> Result<GeneralSettings> {
        let settings = GeneralSettings {
            host: self.general_host.read(cx).value().trim().to_string(),
            port: self.general_port.read(cx).value().trim().parse()?,
            username: self.general_username.read(cx).value().to_string(),
            password: self.general_password.read(cx).value().to_string(),
            remote_dns: self.general.remote_dns,
            timeout_seconds: self.general_timeout.read(cx).value().trim().parse()?,
        };
        settings.proxy()?;
        Ok(settings)
    }

    fn schedule_general_save(&mut self, cx: &mut Context<Self>) {
        self.general_generation += 1;
        self.general_test_generation += 1;
        self.general_testing = false;
        let generation = self.general_generation;
        self.general_status = "等待保存…".into();
        self.general_test.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.general_generation {
                    return;
                }
                let result = this.general_from_inputs(cx).and_then(|settings| {
                    settings.save(this.service.data_dir())?;
                    let proxy = settings.proxy()?;
                    Ok((settings, proxy))
                });
                match result {
                    Ok((settings, proxy)) => {
                        this.general = settings;
                        this.global_proxy = proxy.clone();
                        this.browser_latest = None;
                        match ProfileService::with_proxy(this.service.data_dir(), proxy) {
                            Ok(service) => this.service = service,
                            Err(error) => {
                                this.general_status = format!("更新代理失败：{error}");
                                cx.notify();
                                return;
                            }
                        }
                        this.general_status = "已自动保存".into();
                    }
                    Err(error) => this.general_status = format!("尚未保存：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn test_general_proxy(&mut self, cx: &mut Context<Self>) {
        let proxy = match self
            .general_from_inputs(cx)
            .and_then(|settings| settings.proxy())
        {
            Ok(proxy) => proxy,
            Err(error) => {
                self.general_test = format!("配置无效：{error}");
                cx.notify();
                return;
            }
        };
        self.general_testing = true;
        self.general_test_generation += 1;
        self.general_test = "正在检测出口 IP…".into();
        let generation = self.general_generation;
        let test_generation = self.general_test_generation;
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { tokio.block_on(async { ProfileGeo::lookup(&proxy).await }) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation == this.general_generation
                    && test_generation == this.general_test_generation
                {
                    this.general_testing = false;
                    this.general_test = match result {
                        Ok(geo) => format!("连接正常 · 出口 IP {}", geo.ip),
                        Err(error) => format!("连接失败：{error}"),
                    };
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_managed(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let selected = id
            .as_ref()
            .and_then(|id| self.managed.iter().find(|proxy| &proxy.id == id))
            .cloned();
        self.managed_selected = id;
        self.managed_delete_pending = false;
        let (proxy_id, name, url, username, password, switch_url, wait) =
            if let Some(proxy) = selected {
                self.managed_policy = proxy.policy;
                self.managed_method = proxy
                    .ip_switch
                    .as_ref()
                    .map(|switch| switch.method)
                    .unwrap_or_default();
                self.managed_on_start = proxy
                    .ip_switch
                    .as_ref()
                    .map(|switch| switch.on_start)
                    .unwrap_or(false);
                let switch_url = proxy
                    .ip_switch
                    .as_ref()
                    .map(|switch| switch.url.clone())
                    .unwrap_or_default();
                let wait = proxy
                    .ip_switch
                    .as_ref()
                    .map(|switch| switch.wait_seconds)
                    .unwrap_or(3);
                let credentials = proxy.credentials.unwrap_or(ProxyCredentials {
                    username: String::new(),
                    password: String::new(),
                });
                (
                    proxy.id,
                    proxy.name,
                    proxy.url,
                    credentials.username,
                    credentials.password,
                    switch_url,
                    wait,
                )
            } else {
                self.managed_policy = ProxyPolicy::AllowParallel;
                self.managed_method = SwitchMethod::Get;
                self.managed_on_start = false;
                (
                    format!("proxy-{:08x}", rand::random::<u32>()),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    3,
                )
            };
        for (input, value) in [
            (&self.managed_id, proxy_id),
            (&self.managed_name, name),
            (&self.managed_url, url),
            (&self.managed_username, username),
            (&self.managed_password, password),
            (&self.managed_switch_url, switch_url),
            (&self.managed_wait, wait.to_string()),
        ] {
            input.update(cx, |input, cx| input.set_value(&value, window, cx));
        }
        self.managed_status.clear();
        cx.notify();
    }

    fn test_managed(&mut self, id: String, cx: &mut Context<Self>) {
        if self.managed_testing.contains(&id) {
            return;
        }
        let Some(record) = self.managed.iter().find(|record| record.id == id) else {
            return;
        };
        let proxy = match ProxyCatalog::new(self.service.data_dir()).resolve_url(&record.url) {
            Ok(proxy) => proxy,
            Err(_) => {
                self.managed_tests.insert(id, "代理配置无效".into());
                cx.notify();
                return;
            }
        };
        self.managed_testing.insert(id.clone());
        self.managed_tests.insert(id.clone(), "检测中…".into());
        let generation = self.managed_generation;
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { tokio.block_on(async { ProfileGeo::lookup(&proxy).await }) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.managed_testing.remove(&id);
                if generation == this.managed_generation
                    && this.managed.iter().any(|record| record.id == id)
                {
                    this.managed_tests.insert(
                        id,
                        match result {
                            Ok(geo) => format!("{} · {}", geo.country_code, geo.ip),
                            Err(_) => "检测失败，请检查地址和认证".into(),
                        },
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_managed(&mut self, cx: &mut Context<Self>) {
        let id = self.managed_id.read(cx).value().trim().to_string();
        if self
            .managed_selected
            .as_deref()
            .is_some_and(|selected| selected != id)
        {
            self.managed_status = "现有代理的 ID 不可修改".into();
            cx.notify();
            return;
        }
        let switch_url = self.managed_switch_url.read(cx).value().trim().to_string();
        let username = self.managed_username.read(cx).value().to_string();
        let password = self.managed_password.read(cx).value().to_string();
        let result = (|| -> Result<ManagedProxy> {
            if self.managed_on_start && switch_url.is_empty() {
                return Err(anyhow!("开启启动时切换 IP 后，请填写切换地址"));
            }
            let ip_switch = if switch_url.is_empty() {
                None
            } else {
                Some(IpSwitch {
                    url: switch_url,
                    method: self.managed_method,
                    on_start: self.managed_on_start,
                    wait_seconds: self.managed_wait.read(cx).value().trim().parse()?,
                })
            };
            ProxyCatalog::new(self.service.data_dir()).upsert(ManagedProxy {
                id,
                name: self.managed_name.read(cx).value().trim().to_string(),
                url: self.managed_url.read(cx).value().trim().to_string(),
                credentials: if username.is_empty() && password.is_empty() {
                    None
                } else {
                    Some(ProxyCredentials { username, password })
                },
                policy: self.managed_policy,
                ip_switch,
            })
        })();
        match result {
            Ok(proxy) => {
                self.managed_generation += 1;
                self.managed_tests.remove(&proxy.id);
                self.managed_selected = Some(proxy.id);
                self.managed_delete_pending = false;
                match ProxyCatalog::new(self.service.data_dir()).list() {
                    Ok(managed) => {
                        self.managed = managed;
                        self.managed_status = "代理已保存".into();
                    }
                    Err(error) => self.managed_status = format!("读取代理失败：{error}"),
                }
            }
            Err(error) => self.managed_status = format!("保存失败：{error}"),
        }
        cx.notify();
    }

    fn remove_managed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.managed_selected.clone() else {
            return;
        };
        if !self.managed_delete_pending {
            self.managed_delete_pending = true;
            self.managed_status = "再次点击“确认删除”将移除这条代理规则".into();
            cx.notify();
            return;
        }
        match ProxyCatalog::new(self.service.data_dir()).remove(&id) {
            Ok(()) => {
                self.managed_generation += 1;
                self.managed.retain(|proxy| proxy.id != id);
                self.managed_tests.remove(&id);
                self.select_managed(None, window, cx);
                self.managed_status = "代理已删除".into();
            }
            Err(error) => self.managed_status = format!("删除失败：{error}"),
        }
        cx.notify();
    }

    fn change_tag(&mut self, remove: bool, cx: &mut Context<Self>) {
        let Some(old) = self.selected_tag.clone() else {
            return;
        };
        let replacement = self.tag_input.read(cx).value().trim().to_string();
        if !remove && replacement.is_empty() {
            self.tag_status = "请输入新标签名称".into();
            cx.notify();
            return;
        }
        if !remove
            && self.all_tags.iter().any(|tag| {
                tag.eq_ignore_ascii_case(&replacement) && !tag.eq_ignore_ascii_case(&old)
            })
        {
            self.tag_status = "新标签名称已存在".into();
            cx.notify();
            return;
        }
        let catalog = TagCatalog::new(self.service.data_dir());
        let result = if remove {
            catalog.remove(&old)
        } else {
            catalog.rename(&old, &replacement)
        };
        match result {
            Ok(()) => {
                self.selected_tag = None;
                self.tag_status = if remove {
                    "已从标签目录删除；浏览器原有标签保留".into()
                } else {
                    "标签目录已重命名；浏览器原有标签保留".into()
                };
                self.load_tags(cx);
            }
            Err(error) => self.tag_status = format!("更新失败：{error}"),
        }
        cx.notify();
    }

    fn create_tag(&mut self, cx: &mut Context<Self>) {
        let name = self.tag_input.read(cx).value().trim().to_string();
        match TagCatalog::new(self.service.data_dir()).add(&name) {
            Ok(()) => {
                self.selected_tag = Some(name);
                self.tag_status = "标签已加入目录".into();
                self.load_tags(cx);
            }
            Err(error) => self.tag_status = format!("新增失败：{error}"),
        }
        cx.notify();
    }

    fn new(
        service: ProfileService,
        runtime: BrowserRuntime,
        tokio: tokio::runtime::Handle,
        initial_settings: InitialSettings,
        global_proxy: ProxySettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let InitialSettings {
            general,
            browser_download: browser_download_settings,
            workspace_lock,
        } = initial_settings;
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索名称或 ID"));
        let form_id = cx.new(|cx| InputState::new(window, cx).placeholder("例如 vinted-fr-01"));
        let form_name = cx.new(|cx| InputState::new(window, cx).placeholder("显示名称"));
        let form_url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://www.vinted.fr/"));
        let proxy_placeholder = format!("例如 {}", global_proxy.browser_url());
        let form_proxy = cx.new(|cx| InputState::new(window, cx).placeholder(proxy_placeholder));
        let form_country_code = cx.new(|cx| InputState::new(window, cx).placeholder("FR"));
        let form_country = cx.new(|cx| InputState::new(window, cx).placeholder("France"));
        let form_region = cx.new(|cx| InputState::new(window, cx).placeholder("Île-de-France"));
        let form_city = cx.new(|cx| InputState::new(window, cx).placeholder("Paris"));
        let form_timezone = cx.new(|cx| InputState::new(window, cx).placeholder("Europe/Paris"));
        let form_locale = cx.new(|cx| InputState::new(window, cx).placeholder("fr-FR"));
        let form_latitude = cx.new(|cx| InputState::new(window, cx).placeholder("48.86"));
        let form_longitude = cx.new(|cx| InputState::new(window, cx).placeholder("2.35"));
        let tag_input = cx.new(|cx| InputState::new(window, cx).placeholder("输入标签名称"));
        let general_host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("代理服务器")
                .default_value(&general.host)
        });
        let general_port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("端口")
                .default_value(general.port.to_string())
        });
        let general_username = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选")
                .default_value(&general.username)
        });
        let general_password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选")
                .default_value(&general.password)
                .masked(true)
        });
        let general_timeout = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("秒")
                .default_value(general.timeout_seconds.to_string())
        });
        let managed_id = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(format!("proxy-{:08x}", rand::random::<u32>()))
        });
        let managed_name = cx.new(|cx| InputState::new(window, cx).placeholder("代理名称"));
        let managed_url =
            cx.new(|cx| InputState::new(window, cx).placeholder("socks5://host:port"));
        let managed_username = cx.new(|cx| InputState::new(window, cx).placeholder("可选"));
        let managed_password =
            cx.new(|cx| InputState::new(window, cx).placeholder("可选").masked(true));
        let managed_switch_url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://..."));
        let managed_wait = cx.new(|cx| InputState::new(window, cx).default_value("3"));
        let subscription =
            cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.generation += 1;
                    let generation = this.generation;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(180))
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if this.generation == generation {
                                this.reload(cx);
                            }
                        });
                    })
                    .detach();
                }
            });
        let proxy_subscription =
            cx.subscribe_in(&form_proxy, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change)
                    && matches!(this.dialog, Dialog::Create)
                    && this.form_custom_proxy
                {
                    this.geo_generation += 1;
                    this.geo_preview = None;
                    this.geo_loading = false;
                    this.geo_error = "代理已改变，请重新检测 GEO IP".into();
                    cx.notify();
                }
            });
        let mut general_subscriptions = Vec::new();
        for input in [
            &general_host,
            &general_port,
            &general_username,
            &general_password,
            &general_timeout,
        ] {
            general_subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.schedule_general_save(cx);
                    }
                },
            ));
        }
        let managed = ProxyCatalog::new(service.data_dir())
            .list()
            .unwrap_or_default();
        let paths = service.paths().clone();
        let browser_manager = BrowserManager::with_paths(paths.clone());
        let (browser_versions, browser_status) = match browser_manager.installed_versions() {
            Ok(versions) => (versions, String::new()),
            Err(error) => (Vec::new(), format!("读取已安装版本失败：{error}")),
        };
        let mut this = Self {
            paths,
            service,
            runtime,
            tokio,
            global_proxy,
            workspace_lock,
            settings_tab: None,
            general,
            general_host,
            general_port,
            general_username,
            general_password,
            general_timeout,
            general_generation: 0,
            general_status: "已自动保存".into(),
            general_test: String::new(),
            general_testing: false,
            general_test_generation: 0,
            storage_busy: false,
            storage_status: String::new(),
            browser_download_settings,
            browser_versions,
            browser_latest: None,
            browser_checking: false,
            browser_busy: false,
            browser_status,
            browser_progress: None,
            browser_progress_shared: None,
            browser_control: None,
            browser_versions_expanded: true,
            browser_delete_pending: None,
            managed,
            managed_selected: None,
            managed_id,
            managed_name,
            managed_url,
            managed_username,
            managed_password,
            managed_switch_url,
            managed_wait,
            managed_policy: ProxyPolicy::AllowParallel,
            managed_method: SwitchMethod::Get,
            managed_on_start: false,
            managed_status: String::new(),
            managed_tests: HashMap::new(),
            managed_testing: HashSet::new(),
            managed_generation: 0,
            managed_delete_pending: false,
            search,
            form_id,
            form_name,
            form_url,
            form_proxy,
            form_country_code,
            form_country,
            form_region,
            form_city,
            form_timezone,
            form_locale,
            form_latitude,
            form_longitude,
            tag_input,
            _subscriptions: {
                general_subscriptions.push(subscription);
                general_subscriptions.push(proxy_subscription);
                general_subscriptions
            },
            rows: Vec::new(),
            page: 1,
            total: 0,
            loading: false,
            has_more: true,
            generation: 0,
            proxy_filter: None,
            filter_tags: Vec::new(),
            all_tags: Vec::new(),
            selected_tag: None,
            tag_status: String::new(),
            running: HashSet::new(),
            selected: HashSet::new(),
            popup: None,
            popup_position: point(px(0.), px(0.)),
            dialog: Dialog::None,
            form_custom_proxy: false,
            form_os: ProfileOs::Windows,
            create_mode: CreateMode::Smart,
            geo_preview: None,
            geo_loading: false,
            geo_generation: 0,
            geo_error: String::new(),
            busy: false,
            status: String::new(),
            spawned: HashMap::new(),
            launch: None,
        };
        if let Some(id) = this.managed.first().map(|proxy| proxy.id.clone()) {
            this.select_managed(Some(id), window, cx);
        }
        this.reload(cx);
        this.load_tags(cx);
        this.refresh_running(cx);
        this.start_polling(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.rows.clear();
        self.page = 1;
        self.total = 0;
        self.loading = false;
        self.has_more = true;
        self.selected.clear();
        self.request_page(cx);
    }

    fn request_page(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.has_more {
            return;
        }
        self.loading = true;
        let generation = self.generation;
        let page = self.page;
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        let query = ProfileQuery {
            search: Some(self.search.read(cx).value().to_string()),
            page,
            page_size: 100,
        };
        let filter = ProfileListFilter {
            proxy_mode: self.proxy_filter,
            tags: self.filter_tags.clone(),
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { tokio.block_on(service.list_filtered(query, filter)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(page_result) => {
                        this.total = page_result.total;
                        this.rows.extend(page_result.items);
                        this.page += 1;
                        this.has_more = this.rows.len() < this.total;
                    }
                    Err(error) => {
                        this.has_more = false;
                        this.status = error.to_string();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_tags(&mut self, cx: &mut Context<Self>) {
        match TagCatalog::new(self.service.data_dir()).list() {
            Ok(tags) => self.all_tags = tags,
            Err(error) => self.status = format!("读取标签失败：{error}"),
        }
        cx.notify();
    }

    fn refresh_running(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let runtime = self.runtime.clone();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        let ids = service.list_ids().await?;
                        let mut running = HashSet::new();
                        for id in ids {
                            if runtime.is_running(&id).await {
                                running.insert(id);
                            }
                        }
                        Ok::<_, rust_browser::profiles::ProfileError>(running)
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let launching = this.launch.as_ref().map(|launch| launch.id.as_str());
                this.spawned.retain(|id, child| {
                    Some(id.as_str()) == launching || child.try_wait().ok().flatten().is_none()
                });
                if let Ok(running) = result
                    && this.running != running
                {
                    this.running = running;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                if this
                    .update(cx, |this, cx| this.refresh_running(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn open_browser(&mut self, id: String, cx: &mut Context<Self>) {
        if self.browser_busy {
            self.status = "浏览器版本正在安装，请稍后打开".into();
            cx.notify();
            return;
        }
        if self.running.contains(&id) || self.spawned.contains_key(&id) || self.launch.is_some() {
            return;
        }
        let Some(profile) = self.rows.iter().find(|profile| profile.id == id) else {
            return;
        };
        let name = profile.name.clone().unwrap_or_else(|| profile.id.clone());
        let location = format!(
            "{} · {}",
            profile.saved_geo.country,
            profile.saved_geo.city.as_deref().unwrap_or("-")
        );
        let result = (|| -> Result<(Child, PathBuf)> {
            let exe = std::env::current_exe()?;
            let log_dir = self.paths.logs();
            fs::create_dir_all(&log_dir)?;
            let progress_file = self.paths.launch_progress(&id, rand::random::<u64>());
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&progress_file)?;
            let log = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.paths.profile_log(&id))?;
            let mut command = ProcessCommand::new(exe);
            command
                .arg("--data-dir")
                .arg(self.service.data_dir())
                .arg("open")
                .arg(&id)
                .arg("--progress-file")
                .arg(&progress_file)
                .stdin(Stdio::null())
                .stdout(Stdio::from(log.try_clone()?))
                .stderr(Stdio::from(log));
            // The native manager uses the saved general proxy, including its
            // credentials, even if the manager inherited a CLI proxy override.
            command.env_remove("RUST_BROWSER_PROXY");
            #[cfg(unix)]
            command.process_group(0);
            let child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = fs::remove_file(&progress_file);
                    return Err(error).context("start browser process");
                }
            };
            Ok((child, progress_file))
        })();
        match result {
            Ok((child, progress_file)) => {
                self.spawned.insert(id.clone(), child);
                self.launch = Some(LaunchUi {
                    id: id.clone(),
                    name,
                    location,
                    progress_file,
                    events_seen: 0,
                    stage: None,
                    close_count: None,
                    switch_ip: false,
                    error: None,
                    visible: true,
                });
                self.dialog = Dialog::None;
                self.popup = None;
                self.status = format!("正在打开 {id}…");
                self.start_launch_poll(cx);
            }
            Err(error) => self.status = error.to_string(),
        }
        cx.notify();
    }

    fn start_launch_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let Ok(keep_polling) = this.update(cx, |this, cx| this.poll_launch(cx)) else {
                    break;
                };
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll_launch(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(launch) = self.launch.as_mut() else {
            return false;
        };
        let events = match read_events(&launch.progress_file) {
            Ok(events) => events,
            Err(error) => {
                launch.error = Some(format!("读取启动进度失败：{error}"));
                launch.visible = true;
                cx.notify();
                return false;
            }
        };
        let mut terminal = false;
        let mut ready = false;
        for event in events.iter().skip(launch.events_seen) {
            terminal = launch.apply(event);
            ready = matches!(event, LaunchEvent::Ready);
            if terminal {
                break;
            }
        }
        launch.events_seen = events.len();
        let id = launch.id.clone();
        let path = launch.progress_file.clone();
        if ready {
            self.launch = None;
            self.status = format!("已打开 {id}");
            let _ = fs::remove_file(path);
            self.refresh_running(cx);
            cx.notify();
            return false;
        }
        if terminal {
            self.status = format!("打开 {id} 失败");
            let _ = fs::remove_file(path);
            cx.notify();
            return false;
        }
        if let Some(child) = self.spawned.get_mut(&id)
            && let Ok(Some(status)) = child.try_wait()
        {
            launch.error = Some(format!(
                "启动进程已退出（{status}），请查看 {}",
                self.paths.profile_log(&id).display()
            ));
            launch.visible = true;
            self.status = format!("打开 {id} 失败");
            let _ = fs::remove_file(path);
            cx.notify();
            return false;
        }
        cx.notify();
        true
    }

    fn cancel_launch(&mut self, cx: &mut Context<Self>) {
        let Some(launch) = self.launch.take() else {
            return;
        };
        if let Some(mut child) = self.spawned.remove(&launch.id) {
            if child.try_wait().ok().flatten().is_none() {
                #[cfg(unix)]
                unsafe {
                    libc::killpg(child.id() as i32, libc::SIGKILL);
                }
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        let _ = fs::remove_file(launch.progress_file);
        self.status = if launch.error.is_some() {
            format!("打开 {} 失败", launch.id)
        } else {
            format!("已取消打开 {}", launch.id)
        };
        self.refresh_running(cx);
        cx.notify();
    }

    fn close_browsers(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let runtime = self.runtime.clone();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        let mut errors = Vec::new();
                        for id in ids {
                            if let Err(error) = runtime.close(&id).await {
                                errors.push(format!("{id}: {error}"));
                            }
                        }
                        errors
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.status = if result.is_empty() {
                    "已请求关闭浏览器".into()
                } else {
                    result.join("; ")
                };
                this.refresh_running(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn show_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Dialog::Create;
        self.form_custom_proxy = false;
        self.form_os = ProfileOs::Windows;
        self.create_mode = CreateMode::Smart;
        self.geo_preview = None;
        self.geo_error.clear();
        self.geo_generation += 1;
        for input in [
            &self.form_id,
            &self.form_name,
            &self.form_url,
            &self.form_proxy,
            &self.form_country_code,
            &self.form_country,
            &self.form_region,
            &self.form_city,
            &self.form_timezone,
            &self.form_locale,
            &self.form_latitude,
            &self.form_longitude,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.detect_geo(cx);
        cx.notify();
    }

    fn show_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.geo_generation += 1;
        let Some(profile) = self.rows.iter().find(|row| row.id == id) else {
            return;
        };
        self.form_id
            .update(cx, |input, cx| input.set_value(&profile.id, window, cx));
        self.form_name.update(cx, |input, cx| {
            input.set_value(profile.name.as_deref().unwrap_or(""), window, cx)
        });
        self.form_url.update(cx, |input, cx| {
            input.set_value(
                profile.tabs.first().map(String::as_str).unwrap_or(""),
                window,
                cx,
            )
        });
        self.form_custom_proxy = matches!(profile.proxy, ProxyChoice::Custom(_));
        self.form_proxy.update(cx, |input, cx| {
            input.set_value(&profile.effective_proxy, window, cx)
        });
        self.dialog = Dialog::Edit(id);
        cx.notify();
    }

    fn detect_geo(&mut self, cx: &mut Context<Self>) {
        let proxy = if self.form_custom_proxy {
            ProxyCatalog::new(self.service.data_dir())
                .resolve_url(self.form_proxy.read(cx).value().trim())
        } else {
            Ok(self.global_proxy.clone())
        };
        let proxy = match proxy {
            Ok(proxy) => proxy,
            Err(error) => {
                self.geo_loading = false;
                self.geo_preview = None;
                self.geo_error = format!("请填写有效的 SOCKS5 代理：{error}");
                cx.notify();
                return;
            }
        };
        self.geo_generation += 1;
        let generation = self.geo_generation;
        self.geo_loading = true;
        self.geo_preview = None;
        self.geo_error.clear();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        proxy.check().await?;
                        ProfileGeo::lookup(&proxy).await
                    })
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if generation != this.geo_generation || !matches!(this.dialog, Dialog::Create) {
                    return;
                }
                this.geo_loading = false;
                match result {
                    Ok(geo) => {
                        this.fill_geo_fields(&geo, window, cx);
                        this.geo_preview = Some(geo);
                    }
                    Err(error) => this.geo_error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn fill_geo_fields(&mut self, geo: &ProfileGeo, window: &mut Window, cx: &mut Context<Self>) {
        for (input, value) in [
            (&self.form_country_code, geo.country_code.clone()),
            (&self.form_country, geo.country.clone()),
            (&self.form_region, geo.region.clone().unwrap_or_default()),
            (&self.form_city, geo.city.clone().unwrap_or_default()),
            (&self.form_timezone, geo.timezone.clone()),
            (&self.form_locale, geo.locale.clone()),
            (&self.form_latitude, geo.latitude.to_string()),
            (&self.form_longitude, geo.longitude.to_string()),
        ] {
            input.update(cx, |input, cx| input.set_value(&value, window, cx));
        }
    }

    fn custom_geo(&self, cx: &Context<Self>) -> Result<ProfileGeoInput> {
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        Ok(ProfileGeoInput {
            country_code: value(&self.form_country_code),
            country: value(&self.form_country),
            region: Some(value(&self.form_region)),
            city: Some(value(&self.form_city)),
            timezone: value(&self.form_timezone),
            locale: value(&self.form_locale),
            latitude: value(&self.form_latitude)
                .parse()
                .map_err(|_| anyhow!("纬度必须是数字"))?,
            longitude: value(&self.form_longitude)
                .parse()
                .map_err(|_| anyhow!("经度必须是数字"))?,
        })
    }

    fn submit_form(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let id = self.form_id.read(cx).value().trim().to_string();
        let name = self.form_name.read(cx).value().trim().to_string();
        let url = self.form_url.read(cx).value().trim().to_string();
        let proxy = self.form_proxy.read(cx).value().trim().to_string();
        let choice = if self.form_custom_proxy {
            ProxyChoice::Custom(proxy)
        } else {
            ProxyChoice::Global
        };
        let tabs = if url.is_empty() {
            Vec::new()
        } else {
            vec![url]
        };
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        let dialog = self.dialog.clone();
        let is_create = matches!(dialog, Dialog::Create);
        let os = self.form_os;
        let geo = if matches!(dialog, Dialog::Create) && self.create_mode == CreateMode::Custom {
            match self.custom_geo(cx) {
                Ok(geo) => Some(geo),
                Err(error) => {
                    self.geo_error = error.to_string();
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        self.geo_error.clear();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        match dialog {
                            Dialog::Create => {
                                service
                                    .create(CreateProfile {
                                        id,
                                        name: if name.is_empty() { None } else { Some(name) },
                                        os,
                                        tabs,
                                        proxy: choice,
                                        geo,
                                    })
                                    .await
                            }
                            Dialog::Edit(id) => {
                                service
                                    .update(
                                        &id,
                                        UpdateProfile {
                                            name: Some(name),
                                            tabs: Some(tabs),
                                            proxy: Some(choice),
                                            tags: None,
                                        },
                                    )
                                    .await
                            }
                            _ => unreachable!(),
                        }
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(profile) => {
                        this.status = format!("已保存 {}", profile.id);
                        this.dialog = Dialog::None;
                        this.reload(cx);
                        this.load_tags(cx);
                    }
                    Err(error) if is_create => this.geo_error = error.to_string(),
                    Err(error) => this.status = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_profile(&mut self, id: String, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { tokio.block_on(service.delete(&id)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.dialog = Dialog::None;
                match result {
                    Ok(()) => {
                        this.status = "配置已删除".into();
                        this.reload(cx);
                        this.load_tags(cx);
                    }
                    Err(error) => this.status = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_profile_tags(&mut self, id: String, tags: Vec<String>, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        let update_id = id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(service.update(
                        &update_id,
                        UpdateProfile {
                            tags: Some(tags),
                            ..Default::default()
                        },
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(profile) => {
                        if this.filter_tags.is_empty() {
                            if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                                *row = profile;
                            }
                        } else {
                            this.popup = None;
                            this.reload(cx);
                        }
                        this.load_tags(cx);
                    }
                    Err(error) => this.status = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_tag(&mut self, id: String, tag: String, cx: &mut Context<Self>) {
        let Some(profile) = self.rows.iter().find(|profile| profile.id == id) else {
            return;
        };
        let mut tags = profile.tags.clone();
        if let Some(index) = tags
            .iter()
            .position(|existing| existing.eq_ignore_ascii_case(&tag))
        {
            tags.remove(index);
        } else {
            tags.push(tag);
        }
        self.set_profile_tags(id, tags, cx);
    }

    fn add_tag(&mut self, cx: &mut Context<Self>) {
        let tag = self.tag_input.read(cx).value().trim().to_string();
        if tag.is_empty() {
            return;
        }
        if let Err(error) = TagCatalog::new(self.service.data_dir()).add(&tag) {
            self.status = format!("添加标签失败：{error}");
            cx.notify();
            return;
        }
        self.load_tags(cx);
        if let Some(TagPopup::Profile(id)) = self.popup.clone() {
            self.toggle_tag(id, tag, cx);
        }
    }

    fn open_tag_popup(
        &mut self,
        popup: TagPopup,
        click: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let size = window.bounds().size;
        let x: f32 = click.x.into();
        let y: f32 = click.y.into();
        let width: f32 = size.width.into();
        let height: f32 = size.height.into();
        self.popup_position = point(
            px(x.min((width - 258.).max(8.)).max(8.)),
            px(y.min((height - 428.).max(8.)).max(8.)),
        );
        self.popup = Some(popup);
        cx.notify();
    }

    fn render_header(&self, _cx: &mut Context<Self>) -> AnyElement {
        TitleBar::new()
            .child(
                h_flex()
                    .w_full()
                    .px_4()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(22.)).rounded_full().bg(rgb(GREEN)))
                    .child(
                        div()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("Cazer Browser"),
                    )
                    .child(div().flex_1()),
            )
            .into_any_element()
    }

    fn render_settings_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.settings_tab.unwrap_or(SettingsTab::General);
        let mut nav = v_flex()
            .w(px(218.))
            .h_full()
            .p_4()
            .gap_2()
            .border_r_1()
            .border_color(rgb(LINE))
            .child(
                Button::new("back-profiles")
                    .ghost()
                    .label("← 返回浏览器列表")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_tab = None;
                        this.reload(cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .pt_4()
                    .pb_3()
                    .text_xl()
                    .font_semibold()
                    .text_color(rgb(INK))
                    .child("全局设置"),
            );
        for (tab, label) in [
            (SettingsTab::General, "常规"),
            (SettingsTab::Proxies, "代理管理"),
            (SettingsTab::Tags, "标签管理"),
            (SettingsTab::Data, "数据与浏览器"),
        ] {
            nav = nav.child(
                Button::new(format!("settings-{label}"))
                    .w_full()
                    .when(active == tab, |button| button.primary())
                    .when(active != tab, |button| button.ghost())
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_tab = Some(tab);
                        if tab == SettingsTab::Tags {
                            this.load_tags(cx);
                        }
                        cx.notify();
                    })),
            );
        }
        nav.into_any_element()
    }

    fn render_general_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .text_2xl()
                    .font_semibold()
                    .text_color(rgb(INK))
                    .child("常规"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("修改有效配置后自动保存"),
            )
            .child(
                v_flex()
                    .w_full()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .child(
                        h_flex()
                            .p_4()
                            .justify_between()
                            .border_b_1()
                            .border_color(rgb(LINE))
                            .child(
                                div()
                                    .text_lg()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("SOCKS5 网络代理"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(GREEN))
                                    .child(self.general_status.clone()),
                            ),
                    )
                    .child(
                        v_flex()
                            .p_5()
                            .gap_4()
                            .child(
                                h_flex()
                                    .gap_5()
                                    .child(settings_field("服务器", &self.general_host))
                                    .child(settings_field("端口", &self.general_port)),
                            )
                            .child(
                                h_flex()
                                    .gap_5()
                                    .child(settings_field("用户名", &self.general_username))
                                    .child(settings_field("密码", &self.general_password)),
                            )
                            .child(
                                h_flex()
                                    .gap_5()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .w(px(90.))
                                                    .text_sm()
                                                    .text_color(rgb(INK))
                                                    .child("DNS 解析"),
                                            )
                                            .child(
                                                Button::new("general-dns")
                                                    .outline()
                                                    .label(if self.general.remote_dns {
                                                        "通过代理解析"
                                                    } else {
                                                        "本地解析"
                                                    })
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.general.remote_dns =
                                                            !this.general.remote_dns;
                                                        this.schedule_general_save(cx);
                                                    })),
                                            ),
                                    )
                                    .child(settings_field("连接超时（秒）", &self.general_timeout)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child("用户名和密码留空时使用无认证连接。"),
                            )
                            .child(div().h(px(1.)).bg(rgb(LINE)))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_4()
                                    .child(
                                        Button::new("test-general-proxy")
                                            .outline()
                                            .label(if self.general_testing {
                                                "检测中…"
                                            } else {
                                                "检测连接"
                                            })
                                            .disabled(self.general_testing)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.test_general_proxy(cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(
                                                if self.general_test.starts_with("连接失败") {
                                                    RED
                                                } else {
                                                    GREEN
                                                },
                                            ))
                                            .child(self.general_test.clone()),
                                    ),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .p_5()
                    .gap_4()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("数据目录"),
                    )
                    .child(storage_directory_row(
                        "程序目录（SQLite 与设置）",
                        self.paths.root().to_path_buf(),
                        None,
                        self.storage_busy,
                        cx,
                    ))
                    .child(storage_directory_row(
                        "浏览器文件工作目录",
                        self.paths.browser_root().to_path_buf(),
                        Some(StorageKind::Browser),
                        self.storage_busy,
                        cx,
                    ))
                    .child(storage_directory_row(
                        "用户数据工作目录",
                        self.paths.profiles_root().to_path_buf(),
                        Some(StorageKind::Profiles),
                        self.storage_busy,
                        cx,
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("所选位置分别存放 browser/、browser-versions/ 和 profiles/。"),
                    )
                    .when(!self.storage_status.is_empty(), |card| {
                        card.child(
                            div()
                                .text_sm()
                                .text_color(rgb(if self.storage_status.contains("失败") {
                                    RED
                                } else {
                                    GREEN
                                }))
                                .child(self.storage_status.clone()),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_managed_settings(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex();
        for (index, proxy) in self.managed.iter().enumerate() {
            let id = proxy.id.clone();
            let selected = self.managed_selected.as_deref() == Some(id.as_str());
            let policy = match proxy.policy {
                ProxyPolicy::AllowParallel => "允许并发",
                ProxyPolicy::RejectNew => "拒绝新开",
                ProxyPolicy::ClosePrevious => "关闭已有",
            };
            let rotation = if proxy.ip_switch.as_ref().is_some_and(|rule| rule.on_start) {
                "开启"
            } else {
                "关闭"
            };
            let test = self
                .managed_tests
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "未检测".into());
            let testing = self.managed_testing.contains(&id);
            let test_id = id.clone();
            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("managed-row-{id}")))
                    .h(px(66.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(if selected {
                        SELECTED
                    } else if index % 2 == 0 {
                        0xffffff
                    } else {
                        ROW_ALT
                    }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_managed(Some(id.clone()), window, cx)
                    }))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(165.))
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child(proxy.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .text_ellipsis()
                                    .child(proxy.url.clone()),
                            ),
                    )
                    .child(
                        div()
                            .w(px(86.))
                            .text_xs()
                            .text_color(rgb(INK))
                            .child(policy),
                    )
                    .child(
                        div()
                            .w(px(50.))
                            .text_xs()
                            .text_color(rgb(if rotation == "开启" { GREEN } else { MUTED }))
                            .child(rotation),
                    )
                    .child(
                        div()
                            .w(px(105.))
                            .text_xs()
                            .text_ellipsis()
                            .text_color(rgb(if test.starts_with("检测失败") {
                                RED
                            } else {
                                MUTED
                            }))
                            .child(test),
                    )
                    .child(
                        Button::new(format!("test-managed-{test_id}"))
                            .outline()
                            .xsmall()
                            .label(if testing { "检测中" } else { "检测" })
                            .disabled(testing)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.test_managed(test_id.clone(), cx)
                            })),
                    ),
            );
        }
        let table = v_flex()
            .flex_1()
            .min_w(px(535.))
            .border_1()
            .border_color(rgb(LINE))
            .rounded_md()
            .child(
                div()
                    .p_4()
                    .font_semibold()
                    .text_color(rgb(INK))
                    .child("代理列表"),
            )
            .child(
                h_flex()
                    .h(px(39.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .bg(rgb(ROW_ALT))
                    .border_y_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(165.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("名称 / SOCKS5 地址"),
                    )
                    .child(
                        div()
                            .w(px(86.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("并发策略"),
                    )
                    .child(
                        div()
                            .w(px(50.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("切换 IP"),
                    )
                    .child(
                        div()
                            .w(px(105.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("出口检测"),
                    )
                    .child(
                        div()
                            .w(px(44.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("操作"),
                    ),
            )
            .child(if self.managed.is_empty() {
                div()
                    .h(px(120.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("尚无托管代理，点击右上角添加")
                    .into_any_element()
            } else {
                rows.into_any_element()
            })
            .into_any_element();

        let mut policy_buttons = h_flex().gap_2();
        for (policy, label) in [
            (ProxyPolicy::AllowParallel, "允许并发"),
            (ProxyPolicy::RejectNew, "拒绝新开"),
            (ProxyPolicy::ClosePrevious, "关闭已有"),
        ] {
            policy_buttons = policy_buttons.child(
                Button::new(format!("policy-{label}"))
                    .when(self.managed_policy == policy, |button| button.primary())
                    .when(self.managed_policy != policy, |button| button.outline())
                    .small()
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.managed_policy = policy;
                        cx.notify();
                    })),
            );
        }
        let mut method_buttons = h_flex().gap_2();
        for (method, label) in [(SwitchMethod::Get, "GET"), (SwitchMethod::Post, "POST")] {
            method_buttons = method_buttons.child(
                Button::new(format!("switch-method-{label}"))
                    .when(self.managed_method == method, |button| button.primary())
                    .when(self.managed_method != method, |button| button.outline())
                    .small()
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.managed_method = method;
                        cx.notify();
                    })),
            );
        }
        let editor = v_flex()
            .w(px(if narrow { 760. } else { 460. }))
            .border_1()
            .border_color(rgb(LINE))
            .rounded_md()
            .child(
                h_flex()
                    .h(px(53.))
                    .px_4()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(div().font_semibold().text_color(rgb(INK)).child(
                        if self.managed_selected.is_some() {
                            "编辑代理"
                        } else {
                            "添加代理"
                        },
                    ))
                    .when(self.managed_selected.is_some(), |row| {
                        row.child(
                            Button::new("delete-managed")
                                .ghost()
                                .small()
                                .label(if self.managed_delete_pending {
                                    "确认删除"
                                } else {
                                    "删除"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.remove_managed(window, cx)
                                })),
                        )
                    }),
            )
            .child(
                v_flex()
                    .p_4()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("名称"),
                            )
                            .child(Input::new(&self.managed_name)),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("SOCKS5 地址"),
                            )
                            .child(Input::new(&self.managed_url))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("格式：socks5://host:port"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_semibold()
                                            .text_color(rgb(INK))
                                            .child("用户名（可选）"),
                                    )
                                    .child(Input::new(&self.managed_username)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_semibold()
                                            .text_color(rgb(INK))
                                            .child("密码（可选）"),
                                    )
                                    .child(Input::new(&self.managed_password)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("无认证时两项都留空；账号和密码保存在本地 SQLite。"),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("并发策略"),
                            )
                            .child(policy_buttons),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("启动时切换 IP"),
                            )
                            .child(
                                Button::new("managed-on-start")
                                    .when(self.managed_on_start, |button| button.primary())
                                    .when(!self.managed_on_start, |button| button.outline())
                                    .small()
                                    .label(if self.managed_on_start {
                                        "已开启"
                                    } else {
                                        "已关闭"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.managed_on_start = !this.managed_on_start;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.managed_on_start, |form| {
                        form.child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().text_color(rgb(INK)).child("切换地址"))
                                .child(Input::new(&self.managed_switch_url))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(MUTED))
                                        .child("HTTP 或 HTTPS；在启动浏览器时调用"),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_2()
                                .child(div().text_sm().text_color(rgb(INK)).child("HTTP 方法"))
                                .child(method_buttons),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(INK))
                                        .child("切换后等待（秒）"),
                                )
                                .child(Input::new(&self.managed_wait))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(MUTED))
                                        .child("等待出口 IP 生效；失败最多重试 1 次"),
                                ),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .justify_end()
                            .child(
                                Button::new("cancel-managed")
                                    .outline()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.select_managed(
                                            this.managed_selected.clone(),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                Button::new("save-managed")
                                    .primary()
                                    .label("保存代理")
                                    .on_click(cx.listener(|this, _, _, cx| this.save_managed(cx))),
                            ),
                    )
                    .when(!self.managed_status.is_empty(), |form| {
                        form.child(
                            div()
                                .text_sm()
                                .text_color(rgb(if self.managed_status.contains("失败") {
                                    RED
                                } else {
                                    GREEN
                                }))
                                .child(self.managed_status.clone()),
                        )
                    }),
            )
            .into_any_element();
        let panels = if narrow {
            v_flex()
                .gap_4()
                .child(table)
                .child(editor)
                .into_any_element()
        } else {
            h_flex()
                .items_start()
                .gap_4()
                .child(table)
                .child(editor)
                .into_any_element()
        };
        v_flex()
            .w_full()
            .gap_5()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("代理管理"),
                    )
                    .child(
                        Button::new("new-managed-proxy")
                            .primary()
                            .icon(IconName::Plus)
                            .label("添加代理")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_managed(None, window, cx)
                            })),
                    ),
            )
            .child(
                div().text_sm().text_color(rgb(MUTED)).child(
                    "为独立代理设置并发策略和启动时的 IP 切换。常规网络代理在“常规”页设置。",
                ),
            )
            .child(panels)
            .into_any_element()
    }

    fn render_tag_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut tags = v_flex().gap_1();
        for tag in &self.all_tags {
            let name = tag.clone();
            tags = tags.child(
                Button::new(format!("settings-tag-{name}"))
                    .w_full()
                    .when(self.selected_tag.as_deref() == Some(&name), |button| {
                        button.primary()
                    })
                    .when(self.selected_tag.as_deref() != Some(&name), |button| {
                        button.ghost()
                    })
                    .label(tag.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.selected_tag = Some(name.clone());
                        this.tag_input
                            .update(cx, |input, cx| input.set_value(&name, window, cx));
                        this.tag_status.clear();
                        cx.notify();
                    })),
            );
        }
        v_flex()
            .w_full()
            .gap_5()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("标签管理"),
                    )
                    .child(
                        Button::new("new-catalog-tag")
                            .primary()
                            .icon(IconName::Plus)
                            .label("新增标签")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.selected_tag = None;
                                this.tag_input
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                this.tag_status.clear();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("这里管理可选标签目录。修改或删除目录标签不会改变浏览器已保存的标签。"),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_5()
                    .child(
                        v_flex()
                            .w(px(300.))
                            .p_3()
                            .gap_1()
                            .border_1()
                            .border_color(rgb(LINE))
                            .rounded_md()
                            .child(if self.all_tags.is_empty() {
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child("尚无标签")
                                    .into_any_element()
                            } else {
                                tags.into_any_element()
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .p_5()
                            .gap_4()
                            .border_1()
                            .border_color(rgb(LINE))
                            .rounded_md()
                            .child(div().font_semibold().text_color(rgb(INK)).child(
                                if self.selected_tag.is_some() {
                                    "编辑所选标签"
                                } else {
                                    "新增标签"
                                },
                            ))
                            .child(settings_field("名称", &self.tag_input))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .when(self.selected_tag.is_none(), |row| {
                                        row.child(
                                            Button::new("add-catalog-tag")
                                                .primary()
                                                .label("添加标签")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.create_tag(cx)
                                                })),
                                        )
                                    })
                                    .child(
                                        Button::new("rename-tag")
                                            .primary()
                                            .label("重命名")
                                            .disabled(self.selected_tag.is_none())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.change_tag(false, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("remove-tag")
                                            .outline()
                                            .label("删除标签")
                                            .disabled(self.selected_tag.is_none())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.change_tag(true, cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(MUTED))
                                            .child(self.tag_status.clone()),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_data_settings(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let manager = self.browser_manager();
        let current = self.browser_versions.iter().find(|version| version.active);
        let current_label = current
            .map(|version| version.version.full_string())
            .unwrap_or_else(|| "尚未安装".into());
        let latest_label = self
            .browser_latest
            .as_ref()
            .map(|release| release.version.full_string())
            .unwrap_or_else(|| "尚未检查".into());
        let has_update = self.browser_latest.as_ref().is_some_and(|release| {
            current.is_none_or(|current| current.version != release.version)
        });
        let entity = cx.entity();
        let mut version_rows = v_flex().gap_2();
        for installed in &self.browser_versions {
            let version = installed.version.clone();
            let version_for_delete = version.clone();
            let key = version.full_string();
            let active = installed.active;
            let pending = self.browser_delete_pending.as_deref() == Some(key.as_str());
            version_rows = version_rows.child(
                h_flex()
                    .w_full()
                    .p_3()
                    .items_center()
                    .gap_4()
                    .rounded_md()
                    .bg(rgb(ROW_ALT))
                    .child(
                        div()
                            .w(px(160.))
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child(key.clone()),
                    )
                    .child(if active { "当前使用" } else { "已安装" })
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(installed.path.display().to_string()),
                    )
                    .child(
                        Button::new(format!("browser-activate-{key}"))
                            .outline()
                            .small()
                            .label("设为当前")
                            .disabled(active || self.browser_busy || !self.running.is_empty())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_browser_version(version.clone(), cx);
                            })),
                    )
                    .child(
                        Button::new(format!("browser-delete-{key}"))
                            .outline()
                            .small()
                            .label(if pending { "确认删除" } else { "删除" })
                            .disabled(active || self.browser_busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let key = version_for_delete.full_string();
                                if this.browser_delete_pending.as_deref() == Some(key.as_str()) {
                                    this.browser_delete_pending = None;
                                    this.delete_browser_version(version_for_delete.clone(), cx);
                                } else {
                                    this.browser_delete_pending = Some(key);
                                    this.browser_status = "再次点击“确认删除”移除该历史版本".into();
                                    cx.notify();
                                }
                            })),
                    ),
            );
        }
        if self.browser_versions.is_empty() {
            version_rows = version_rows.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("尚无已安装版本"),
            );
        }
        let progress = self.browser_progress.as_ref();
        let fraction = progress.and_then(|progress| {
            progress.total.map(|total| {
                if total == 0 {
                    0.
                } else {
                    (progress.received as f32 / total as f32).clamp(0., 1.)
                }
            })
        });
        let stage = progress
            .map(|progress| progress.stage)
            .unwrap_or(DownloadStage::Download);
        let progress_title = match stage {
            DownloadStage::Download => "正在下载",
            DownloadStage::Verify => "正在校验",
            DownloadStage::Install => "正在安装",
        };
        let progress_text = progress
            .map(|progress| {
                let received = progress.received as f64 / 1_048_576.;
                match progress.total {
                    Some(total) => format!("{received:.1} / {:.1} MB", total as f64 / 1_048_576.),
                    None => format!("已下载 {received:.1} MB"),
                }
            })
            .unwrap_or_default();
        v_flex()
            .w_full()
            .gap_4()
            .child(
                div()
                    .text_2xl()
                    .font_semibold()
                    .text_color(rgb(INK))
                    .child("数据与浏览器"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("管理 Camoufox 安装与本地数据"),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .items_stretch()
                    .when(narrow, |row| row.flex_col())
                    .child(
                        v_flex()
                            .flex_1()
                            .p_5()
                            .gap_3()
                            .border_1()
                            .border_color(rgb(LINE))
                            .rounded_md()
                            .child(
                                div()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("Camoufox 浏览器"),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_semibold()
                                            .text_color(rgb(INK))
                                            .child(current_label),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(if current.is_some() {
                                                GREEN
                                            } else {
                                                MUTED
                                            }))
                                            .child(if current.is_some() {
                                                "● 已安装"
                                            } else {
                                                "未安装"
                                            }),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("browser-check-update")
                                            .outline()
                                            .small()
                                            .label(if self.browser_checking {
                                                "检查中…"
                                            } else {
                                                "检查更新"
                                            })
                                            .disabled(self.browser_checking || self.browser_busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.check_browser_update(cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("browser-manage-versions")
                                            .outline()
                                            .small()
                                            .label(if self.browser_versions_expanded {
                                                "收起版本"
                                            } else {
                                                "管理版本"
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.browser_versions_expanded =
                                                    !this.browser_versions_expanded;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                div().text_xs().text_color(rgb(MUTED)).child(format!(
                                    "安装路径  {}",
                                    manager.active_path().display()
                                )),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .p_5()
                            .gap_3()
                            .border_1()
                            .border_color(rgb(LINE))
                            .rounded_md()
                            .child(div().font_semibold().text_color(rgb(INK)).child("可用更新"))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_semibold()
                                            .text_color(rgb(INK))
                                            .child(latest_label),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("browser-download-install")
                                            .primary()
                                            .small()
                                            .label(if self.browser_busy {
                                                "安装中…"
                                            } else {
                                                "下载并安装"
                                            })
                                            .disabled(
                                                !has_update
                                                    || self.browser_busy
                                                    || self.browser_checking,
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.start_browser_download(cx)
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("来源：Camoufox 官方发布 · 适配当前系统"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .py_2()
                    .child(
                        Checkbox::new("browser-download-global-proxy")
                            .checked(self.browser_download_settings.use_global_proxy)
                            .disabled(self.browser_busy || self.browser_checking)
                            .accessibility_label("使用全局代理下载与更新浏览器")
                            .on_click(move |checked, _, cx| {
                                entity.update(cx, |this, cx| {
                                    let settings = BrowserDownloadSettings {
                                        use_global_proxy: *checked,
                                    };
                                    match settings.save(this.service.data_dir()) {
                                        Ok(()) => {
                                            this.browser_download_settings = settings;
                                            this.browser_latest = None;
                                            this.browser_status =
                                                "下载网络设置已自动保存，请重新检查更新".into();
                                        }
                                        Err(error) => {
                                            this.browser_status = format!("保存失败：{error}")
                                        }
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("使用全局代理下载与更新浏览器"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("勾选后，版本检查、下载与更新使用常规设置中的全局代理。"),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .p_4()
                    .gap_3()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .child(
                        div()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("已安装版本"),
                    )
                    .when(self.browser_versions_expanded, |container| {
                        container.child(version_rows)
                    }),
            )
            .when(self.browser_busy, |container| {
                container.child(
                    v_flex()
                        .w_full()
                        .p_4()
                        .gap_3()
                        .border_1()
                        .border_color(rgb(LINE))
                        .rounded_md()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .font_semibold()
                                        .text_color(rgb(INK))
                                        .child(progress_title),
                                )
                                .child(div().flex_1())
                                .child(div().text_sm().text_color(rgb(MUTED)).child(progress_text))
                                .child(
                                    Button::new("browser-pause-download")
                                        .outline()
                                        .small()
                                        .label(
                                            if self
                                                .browser_control
                                                .as_ref()
                                                .is_some_and(DownloadControl::is_paused)
                                            {
                                                "继续"
                                            } else {
                                                "暂停"
                                            },
                                        )
                                        .disabled(stage != DownloadStage::Download)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(control) = &this.browser_control {
                                                control.pause(!control.is_paused());
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("browser-cancel-download")
                                        .outline()
                                        .small()
                                        .label("取消")
                                        .disabled(stage != DownloadStage::Download)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(control) = &this.browser_control {
                                                control.cancel();
                                                this.browser_status = "正在取消下载…".into();
                                            }
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            div().w_full().h(px(9.)).rounded_full().bg(rgb(LINE)).child(
                                div()
                                    .w(relative(fraction.unwrap_or(0.)))
                                    .h_full()
                                    .rounded_full()
                                    .bg(rgb(GREEN)),
                            ),
                        )
                        .child(
                            div().text_xs().text_color(rgb(MUTED)).child(format!(
                                "下载  →  校验  →  安装     当前：{progress_title}"
                            )),
                        ),
                )
            })
            .when(!self.browser_status.is_empty(), |container| {
                container.child(
                    div()
                        .text_sm()
                        .text_color(rgb(if self.browser_status.contains("失败") {
                            RED
                        } else {
                            GREEN
                        }))
                        .child(self.browser_status.clone()),
                )
            })
            .child(
                v_flex()
                    .p_5()
                    .gap_3()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .child(div().font_semibold().text_color(rgb(INK)).child("本地数据"))
                    .child(data_location(
                        "SQLite 数据库",
                        self.paths.database(),
                        true,
                        cx,
                    ))
                    .child(data_location(
                        "浏览器用户数据",
                        self.paths.profiles(),
                        false,
                        cx,
                    ))
                    .child(data_location(
                        "浏览器安装目录",
                        manager.active_path(),
                        false,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_settings(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let content = match self.settings_tab.unwrap_or(SettingsTab::General) {
            SettingsTab::General => self.render_general_settings(cx),
            SettingsTab::Proxies => {
                let width: f32 = window.bounds().size.width.into();
                self.render_managed_settings(width < 1300., cx)
            }
            SettingsTab::Tags => self.render_tag_settings(cx),
            SettingsTab::Data => {
                let width: f32 = window.bounds().size.width.into();
                self.render_data_settings(width < 1320., cx)
            }
        };
        h_flex()
            .flex_1()
            .min_h_0()
            .child(self.render_settings_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .overflow_y_scrollbar()
                    .p_8()
                    .child(content),
            )
            .into_any_element()
    }

    fn render_search_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let proxy_label = match self.proxy_filter {
            None => "全部代理",
            Some(ProxyModeFilter::Global) => "跟随全局",
            Some(ProxyModeFilter::Custom) => "独立配置",
        };
        h_flex()
            .h(px(60.))
            .px_4()
            .items_center()
            .gap_3()
            .bg(rgb(0xffffff))
            .child(div().flex_1())
            .child(div().w(px(270.)).child(
                Input::new(&self.search).prefix(Icon::new(IconName::Search).text_color(rgb(MUTED))),
            ))
            .child(
                Button::new("proxy-filter")
                    .outline()
                    .label(proxy_label)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.proxy_filter = match this.proxy_filter {
                            None => Some(ProxyModeFilter::Global),
                            Some(ProxyModeFilter::Global) => Some(ProxyModeFilter::Custom),
                            Some(ProxyModeFilter::Custom) => None,
                        };
                        this.reload(cx);
                    })),
            )
            .child(
                Button::new("new-profile")
                    .primary()
                    .icon(IconName::Plus)
                    .label("新建浏览器")
                    .on_click(cx.listener(|this, _, window, cx| this.show_create(window, cx))),
            )
            .into_any_element()
    }

    fn render_runtime_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected_running: Vec<String> = self
            .selected
            .iter()
            .filter(|id| self.running.contains(*id))
            .cloned()
            .collect();
        let can_close_selected = !selected_running.is_empty();
        let can_close_all = !self.running.is_empty();
        let tag_title = if self.filter_tags.is_empty() {
            "全部".to_string()
        } else {
            format!("已选 {}", self.filter_tags.len())
        };
        h_flex()
            .mx_4()
            .h(px(50.))
            .px_3()
            .items_center()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(0xffffff))
            .child(div().size(px(8.)).rounded_full().bg(rgb(GREEN)))
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(rgb(INK))
                    .child(format!("正在运行 {} 个", self.running.len())),
            )
            .child(div().h(px(20.)).w(px(1.)).bg(rgb(LINE)))
            .when(
                self.launch.as_ref().is_some_and(|launch| !launch.visible),
                |bar| {
                    bar.child(
                        Button::new("show-launch-progress")
                            .outline()
                            .label("查看启动进度")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(launch) = this.launch.as_mut() {
                                    launch.visible = true;
                                    cx.notify();
                                }
                            })),
                    )
                },
            )
            .child(
                Button::new("close-selected")
                    .outline()
                    .label("关闭选中浏览器")
                    .disabled(!can_close_selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_browsers(selected_running.clone(), cx)
                    })),
            )
            .child(
                Button::new("close-all")
                    .outline()
                    .label("关闭全部")
                    .disabled(!can_close_all)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dialog = Dialog::CloseAll;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("open-settings")
                    .outline()
                    .icon(IconName::Settings)
                    .label("全局设置")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_tab = Some(SettingsTab::General);
                        this.popup = None;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(div().text_sm().text_color(rgb(MUTED)).child("标签"))
            .child(
                Button::new("tag-filter")
                    .outline()
                    .label(tag_title)
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        if matches!(this.popup, Some(TagPopup::Filter)) {
                            this.popup = None;
                            cx.notify();
                        } else {
                            this.open_tag_popup(TagPopup::Filter, event.position(), window, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    fn render_table_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let all_loaded_selected = !self.rows.is_empty()
            && self
                .rows
                .iter()
                .all(|profile| self.selected.contains(&profile.id));
        let entity = cx.entity();
        h_flex()
            .mx_4()
            .mt_2()
            .h(px(42.))
            .px_3()
            .items_center()
            .border_1()
            .border_color(rgb(LINE))
            .rounded_t_md()
            .bg(rgb(0xf7fafc))
            .text_xs()
            .font_semibold()
            .text_color(rgb(INK))
            .child(
                div().w(px(38.)).child(
                    Checkbox::new("select-loaded")
                        .checked(all_loaded_selected)
                        .accessibility_label("选择已加载的浏览器")
                        .on_click(move |checked, _, cx| {
                            entity.update(cx, |this, cx| {
                                if *checked {
                                    this.selected
                                        .extend(this.rows.iter().map(|profile| profile.id.clone()));
                                } else {
                                    this.selected.clear();
                                }
                                cx.notify();
                            });
                        }),
                ),
            )
            .child(div().w(px(260.)).child("名称 / ID"))
            .child(div().w(px(100.)).child("状态"))
            .child(div().w(px(180.)).child("保存地区"))
            .child(div().w(px(110.)).child("代理"))
            .child(div().flex_1().min_w(px(130.)).child("标签"))
            .child(div().w(px(155.)).child("创建时间"))
            .child(div().w(px(125.)).child("操作"))
            .into_any_element()
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let profile = self.rows[index].clone();
        let id = profile.id.clone();
        let running = self.running.contains(&id);
        let selected = self.selected.contains(&id);
        let entity = cx.entity();
        let check_id = id.clone();
        let action_id = id.clone();
        let edit_id = id.clone();
        let delete_id = id.clone();
        let tag_id = id.clone();
        let location = format!(
            "{} · {}",
            profile.saved_geo.country,
            profile.saved_geo.city.as_deref().unwrap_or("-")
        );
        let proxy_label = match &profile.proxy {
            ProxyChoice::Global => "跟随全局",
            ProxyChoice::Custom(_) => "独立配置",
        };
        let os = match profile.os.as_str() {
            "macos" => "macOS",
            "windows" => "Windows",
            "linux" => "Linux",
            _ => "未知系统",
        };
        let tags = profile
            .tags
            .iter()
            .take(2)
            .cloned()
            .map(|tag| {
                div()
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .bg(rgb(GREEN_PALE))
                    .text_xs()
                    .text_color(rgb(GREEN))
                    .child(tag)
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        h_flex()
            .id(SharedString::from(format!("profile-row-{id}")))
            .w_full()
            .h(px(54.))
            .px_3()
            .items_center()
            .text_sm()
            .border_b_1()
            .border_color(rgb(LINE))
            .bg(rgb(if selected {
                SELECTED
            } else if index.is_multiple_of(2) {
                0xffffff
            } else {
                ROW_ALT
            }))
            .child(
                div().w(px(38.)).child(
                    Checkbox::new(SharedString::from(format!("profile-check-{id}")))
                        .checked(selected)
                        .accessibility_label(format!("选择 {}", profile.id))
                        .on_click(move |checked, _, cx| {
                            entity.update(cx, |this, cx| {
                                if *checked {
                                    this.selected.insert(check_id.clone());
                                } else {
                                    this.selected.remove(&check_id);
                                }
                                cx.notify();
                            });
                        }),
                ),
            )
            .child(
                v_flex()
                    .w(px(260.))
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div().font_semibold().text_color(rgb(INK)).child(
                                    profile.name.clone().unwrap_or_else(|| profile.id.clone()),
                                ),
                            )
                            .child(div().text_xs().text_color(rgb(MUTED)).child(os)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(profile.id.clone()),
                    ),
            )
            .child(
                div()
                    .w(px(100.))
                    .text_color(rgb(if running { GREEN } else { MUTED }))
                    .child(if running {
                        "● 运行中"
                    } else {
                        "● 未运行"
                    }),
            )
            .child(
                div()
                    .w(px(180.))
                    .text_ellipsis()
                    .text_color(rgb(INK))
                    .child(location),
            )
            .child(
                div()
                    .w(px(110.))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(proxy_label),
            )
            .child(
                h_flex()
                    .id(SharedString::from(format!("tag-cell-{id}")))
                    .w_full()
                    .flex_1()
                    .min_w(px(130.))
                    .gap_1()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.open_tag_popup(
                            TagPopup::Profile(tag_id.clone()),
                            event.position(),
                            window,
                            cx,
                        );
                    }))
                    .children(tags)
                    .child(div().text_color(rgb(GREEN)).child("＋")),
            )
            .child(
                div()
                    .w(px(155.))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(format_date(profile.created_at)),
            )
            .child(
                h_flex()
                    .w(px(125.))
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("run-{id}")))
                            .ghost()
                            .xsmall()
                            .label(if running { "关闭" } else { "打开" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if running {
                                    this.close_browsers(vec![action_id.clone()], cx);
                                } else {
                                    this.open_browser(action_id.clone(), cx);
                                }
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("edit-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings)
                            .tooltip("编辑配置")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_edit(edit_id.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Delete)
                            .tooltip("删除配置")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dialog = Dialog::Delete(delete_id.clone());
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_tag_popup(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(popup) = self.popup.clone() else {
            return div().into_any_element();
        };
        let selected_tags = match &popup {
            TagPopup::Filter => self.filter_tags.clone(),
            TagPopup::Profile(id) => self
                .rows
                .iter()
                .find(|row| &row.id == id)
                .map(|row| row.tags.clone())
                .unwrap_or_default(),
        };
        let mut available_tags = self.all_tags.clone();
        if matches!(popup, TagPopup::Profile(_)) {
            for tag in &selected_tags {
                if !available_tags
                    .iter()
                    .any(|option| option.eq_ignore_ascii_case(tag))
                {
                    available_tags.push(tag.clone());
                }
            }
        }
        let mut options = Vec::new();
        for tag in available_tags {
            let tag_for_click = tag.clone();
            let target = popup.clone();
            let entity = cx.entity();
            options.push(
                h_flex()
                    .h(px(30.))
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new(SharedString::from(format!("tag-choice-{tag}")))
                            .checked(selected_tags.contains(&tag))
                            .accessibility_label(format!("标签 {tag}"))
                            .on_click(move |checked, _, cx| {
                                entity.update(cx, |this, cx| match &target {
                                    TagPopup::Filter => {
                                        if *checked {
                                            if !this.filter_tags.contains(&tag_for_click) {
                                                this.filter_tags.push(tag_for_click.clone());
                                            }
                                        } else {
                                            this.filter_tags
                                                .retain(|selected| selected != &tag_for_click);
                                        }
                                        this.reload(cx);
                                    }
                                    TagPopup::Profile(id) => {
                                        this.toggle_tag(id.clone(), tag_for_click.clone(), cx)
                                    }
                                });
                            }),
                    )
                    .child(div().text_sm().text_color(rgb(INK)).child(tag))
                    .into_any_element(),
            );
        }
        v_flex()
            .absolute()
            .top(self.popup_position.y)
            .left(self.popup_position.x)
            .w(px(250.))
            .max_h(px(420.))
            .overflow_y_scrollbar()
            .p_3()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(0xffffff))
            .shadow_lg()
            .child(
                h_flex()
                    .items_center()
                    .child(div().font_semibold().text_color(rgb(INK)).child(
                        if matches!(popup, TagPopup::Filter) {
                            "筛选标签"
                        } else {
                            "选择标签"
                        },
                    ))
                    .child(div().flex_1())
                    .child(
                        Button::new("close-tags")
                            .ghost()
                            .xsmall()
                            .label("×")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.popup = None;
                                cx.notify();
                            })),
                    ),
            )
            .children(options)
            .child(if matches!(popup, TagPopup::Profile(_)) {
                v_flex()
                    .pt_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(div().text_sm().text_color(rgb(GREEN)).child("＋ 新建标签"))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(div().flex_1().child(Input::new(&self.tag_input).small()))
                            .child(
                                Button::new("add-tag")
                                    .outline()
                                    .small()
                                    .label("添加")
                                    .on_click(cx.listener(|this, _, _, cx| this.add_tag(cx))),
                            ),
                    )
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .into_any_element()
    }

    fn render_create_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let smart = self.create_mode == CreateMode::Smart;
        let proxy_description = if self.form_custom_proxy {
            "使用此 Profile 的独立代理".to_string()
        } else {
            self.global_proxy.browser_url()
        };
        let geo_content: AnyElement = if smart {
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .text_color(rgb(INK))
                                .child("代理 GEO IP"),
                        )
                        .child(
                            Button::new("detect-geo-smart")
                                .outline()
                                .small()
                                .label(if self.geo_loading {
                                    "检测中…"
                                } else {
                                    "检测代理 GEO IP"
                                })
                                .disabled(self.geo_loading)
                                .on_click(cx.listener(|this, _, _, cx| this.detect_geo(cx))),
                        )
                        .child(if self.geo_preview.is_some() {
                            div()
                                .text_sm()
                                .text_color(rgb(GREEN))
                                .child("● 已获取出口位置")
                        } else {
                            div()
                        }),
                )
                .child(if let Some(geo) = &self.geo_preview {
                    v_flex()
                        .gap_2()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(GREEN_PALE))
                        .child(div().font_semibold().text_color(rgb(INK)).child(format!(
                            "{} · {}  /  {}",
                            geo.country,
                            geo.city.as_deref().unwrap_or("-"),
                            geo.ip
                        )))
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .text_color(rgb(GREEN))
                                .child("自动生成的配置"),
                        )
                        .child(geo_summary_line(
                            "国家 / 地区",
                            format!("{} · {}", geo.country_code, geo.country),
                        ))
                        .child(geo_summary_line(
                            "省 / 州",
                            geo.region.clone().unwrap_or_else(|| "-".into()),
                        ))
                        .child(geo_summary_line(
                            "城市",
                            geo.city.clone().unwrap_or_else(|| "-".into()),
                        ))
                        .child(geo_summary_line("时区", geo.timezone.clone()))
                        .child(geo_summary_line("语言", geo.locale.clone()))
                        .child(geo_summary_line(
                            "经纬度",
                            format!("{:.4} / {:.4}", geo.latitude, geo.longitude),
                        ))
                        .into_any_element()
                } else {
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(ROW_ALT))
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(if self.geo_loading {
                            "正在通过代理获取出口位置…"
                        } else {
                            "检测代理后，这里会显示自动填充的地理配置。"
                        })
                        .into_any_element()
                })
                .into_any_element()
        } else {
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .text_color(rgb(INK))
                                .child("地理位置 (GEO)"),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("detect-geo-custom")
                                .outline()
                                .small()
                                .label(if self.geo_loading {
                                    "填充中…"
                                } else {
                                    "从 GEO IP 填充"
                                })
                                .disabled(self.geo_loading)
                                .on_click(cx.listener(|this, _, _, cx| this.detect_geo(cx))),
                        ),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(geo_field("国家代码", &self.form_country_code))
                        .child(geo_field("国家 / 地区", &self.form_country)),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(geo_field("省 / 州", &self.form_region))
                        .child(geo_field("城市", &self.form_city)),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(geo_field("时区", &self.form_timezone))
                        .child(geo_field("语言", &self.form_locale)),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(geo_field("纬度", &self.form_latitude))
                        .child(geo_field("经度", &self.form_longitude)),
                )
                .into_any_element()
        };
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .p_1()
                    .rounded_md()
                    .bg(rgb(ROW_ALT))
                    .child(
                        Button::new("create-mode-smart")
                            .flex_1()
                            .when(smart, |button| button.primary())
                            .when(!smart, |button| button.ghost())
                            .label("智能模式")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_mode = CreateMode::Smart;
                                if let Some(geo) = this.geo_preview.clone() {
                                    this.fill_geo_fields(&geo, window, cx);
                                } else if !this.geo_loading {
                                    this.detect_geo(cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("create-mode-custom")
                            .flex_1()
                            .when(!smart, |button| button.primary())
                            .when(smart, |button| button.ghost())
                            .label("自定义模式")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_mode = CreateMode::Custom;
                                cx.notify();
                            })),
                    ),
            )
            .child(form_field("Profile ID", &self.form_id, cx))
            .child(form_field("名称", &self.form_name, cx))
            .child(form_field("启动页面", &self.form_url, cx))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(100.))
                            .text_sm()
                            .text_color(rgb(INK))
                            .child("代理"),
                    )
                    .child(
                        Button::new("create-proxy-mode")
                            .outline()
                            .small()
                            .label(if self.form_custom_proxy {
                                "独立配置"
                            } else {
                                "跟随全局"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.form_custom_proxy = !this.form_custom_proxy;
                                this.geo_generation += 1;
                                this.geo_preview = None;
                                this.geo_loading = false;
                                this.geo_error.clear();
                                if !this.form_custom_proxy {
                                    this.detect_geo(cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(proxy_description),
                    ),
            )
            .child(if self.form_custom_proxy {
                form_field("SOCKS5", &self.form_proxy, cx)
            } else {
                div().into_any_element()
            })
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(100.))
                            .text_sm()
                            .text_color(rgb(INK))
                            .child("操作系统"),
                    )
                    .child(
                        Button::new("create-os")
                            .outline()
                            .small()
                            .label(match self.form_os {
                                ProfileOs::Windows => "Windows",
                                ProfileOs::Macos => "macOS",
                                ProfileOs::Linux => "Linux",
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.form_os = match this.form_os {
                                    ProfileOs::Windows => ProfileOs::Macos,
                                    ProfileOs::Macos => ProfileOs::Linux,
                                    ProfileOs::Linux => ProfileOs::Windows,
                                };
                                cx.notify();
                            })),
                    ),
            )
            .child(div().h(px(1.)).w_full().bg(rgb(LINE)))
            .child(geo_content)
            .child(if self.geo_error.is_empty() {
                div().into_any_element()
            } else {
                div()
                    .text_sm()
                    .text_color(rgb(RED))
                    .child(self.geo_error.clone())
                    .into_any_element()
            })
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("创建时固定地理身份；国家名称与代码、地区和时区须与代理出口一致。"),
            )
            .into_any_element()
    }

    fn render_dialog(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let dialog = self.dialog.clone();
        if matches!(dialog, Dialog::None) {
            return div().into_any_element();
        }
        let title = match &dialog {
            Dialog::Create => "新建浏览器",
            Dialog::Edit(_) => "编辑浏览器",
            Dialog::Delete(_) => "删除浏览器",
            Dialog::CloseAll => "关闭全部浏览器",
            Dialog::None => unreachable!(),
        };
        let body: AnyElement = match dialog.clone() {
            Dialog::Create => self.render_create_body(cx),
            Dialog::Edit(id) => v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(100.)).text_sm().text_color(rgb(INK)).child("ID"))
                        .child(div().text_sm().text_color(rgb(MUTED)).child(id)),
                )
                .child(form_field("名称", &self.form_name, cx))
                .child(form_field("启动页面", &self.form_url, cx))
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(100.)).text_sm().child("代理"))
                        .child(
                            Button::new("form-proxy-mode")
                                .outline()
                                .small()
                                .label(if self.form_custom_proxy {
                                    "独立配置"
                                } else {
                                    "跟随全局"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.form_custom_proxy = !this.form_custom_proxy;
                                    cx.notify();
                                })),
                        ),
                )
                .child(if self.form_custom_proxy {
                    form_field("SOCKS5", &self.form_proxy, cx)
                } else {
                    div().into_any_element()
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("修改代理后，启动时仍会校验出口地区；已固定的浏览器指纹保持不变。"),
                )
                .into_any_element(),
            Dialog::Delete(id) => div()
                .text_color(rgb(INK))
                .child(format!("永久删除 {id} 及其浏览器数据？"))
                .into_any_element(),
            Dialog::CloseAll => div()
                .text_color(rgb(INK))
                .child(format!("关闭当前运行的 {} 个浏览器？", self.running.len()))
                .into_any_element(),
            Dialog::None => unreachable!(),
        };
        let confirm = match dialog {
            Dialog::Create | Dialog::Edit(_) => "保存",
            Dialog::Delete(_) => "删除",
            Dialog::CloseAll => "关闭全部",
            Dialog::None => unreachable!(),
        };
        let confirm_dialog = dialog.clone();
        let confirm_disabled = self.busy
            || (matches!(dialog, Dialog::Create)
                && self.create_mode == CreateMode::Smart
                && (self.geo_loading || self.geo_preview.is_none()));
        let card = v_flex()
            .w(px(if matches!(dialog, Dialog::Create) {
                640.
            } else {
                520.
            }))
            .max_h(window.bounds().size.height - px(48.))
            .p_5()
            .gap_4()
            .rounded_lg()
            .bg(rgb(0xffffff))
            .shadow_lg()
            .child(
                h_flex()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child(title),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("dialog-close")
                            .ghost()
                            .label("×")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dialog = Dialog::None;
                                cx.notify();
                            })),
                    ),
            )
            .child(body)
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("dialog-cancel")
                            .outline()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dialog = Dialog::None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("dialog-confirm")
                            .primary()
                            .label(confirm)
                            .disabled(confirm_disabled)
                            .on_click(cx.listener(move |this, _, _, cx| match &confirm_dialog {
                                Dialog::Create | Dialog::Edit(_) => this.submit_form(cx),
                                Dialog::Delete(id) => this.delete_profile(id.clone(), cx),
                                Dialog::CloseAll => {
                                    this.dialog = Dialog::None;
                                    this.close_browsers(this.running.iter().cloned().collect(), cx);
                                }
                                Dialog::None => {}
                            })),
                    ),
            )
            .overflow_y_scrollbar();
        div()
            .absolute()
            .inset_0()
            .bg(hsla(0., 0., 0., 0.38))
            .flex()
            .items_center()
            .justify_center()
            .child(card)
            .into_any_element()
    }

    fn render_launch_dialog(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(launch) = self.launch.as_ref().filter(|launch| launch.visible) else {
            return div().into_any_element();
        };
        let steps = launch.steps();
        let active = launch
            .stage
            .and_then(|stage| steps.iter().position(|item| *item == stage))
            .unwrap_or(0);
        let failed = launch.error.is_some();
        let fill = ((active as f32 + 0.5) / steps.len() as f32).clamp(0.08, 0.94);
        let id = launch.id.clone();
        let items = steps
            .iter()
            .enumerate()
            .map(|(index, stage)| {
                let completed = index < active;
                let current = index == active;
                let color = if failed && current {
                    RED
                } else if completed || current {
                    GREEN
                } else {
                    MUTED
                };
                let marker = div()
                    .size(px(25.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(color))
                    .bg(rgb(if completed { GREEN } else { 0xffffff }))
                    .text_xs()
                    .font_semibold()
                    .text_color(rgb(0xffffff))
                    .child(if completed {
                        "✓"
                    } else if failed && current {
                        "×"
                    } else {
                        ""
                    });
                let detail = if current && !failed {
                    match stage {
                        LaunchStage::ClosePeers => launch
                            .close_count
                            .map(|count| format!("正在关闭 {count} 个使用此代理的浏览器…")),
                        LaunchStage::VerifyGeo => {
                            Some("正在确认代理出口与保存的地区一致".to_string())
                        }
                        LaunchStage::SwitchIp => Some("正在切换并验证新的出口 IP".to_string()),
                        _ => None,
                    }
                } else {
                    None
                };
                h_flex()
                    .min_h(px(if detail.is_some() { 49. } else { 38. }))
                    .items_start()
                    .gap_3()
                    .child(marker)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(if current || completed { INK } else { MUTED }))
                                    .child(launch_stage_label(*stage)),
                            )
                            .when_some(detail, |body, detail| {
                                body.child(div().text_xs().text_color(rgb(MUTED)).child(detail))
                            }),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let card = v_flex()
            .w(px(570.))
            .max_h(window.bounds().size.height - px(48.))
            .rounded_lg()
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(0xffffff))
            .shadow_lg()
            .child(
                v_flex()
                    .p_6()
                    .gap_4()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(38.))
                                    .rounded_full()
                                    .border_4()
                                    .border_color(rgb(GREEN))
                                    .bg(rgb(GREEN_PALE)),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div().text_lg().font_semibold().text_color(rgb(INK)).child(
                                            if failed {
                                                "打开浏览器失败"
                                            } else {
                                                "正在打开浏览器"
                                            },
                                        ),
                                    )
                                    .child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                                        "{} · {} · {}",
                                        launch.name, launch.id, launch.location
                                    ))),
                            ),
                    )
                    .child(v_flex().gap_2().children(items))
                    .child(
                        div().w_full().h(px(7.)).rounded_full().bg(rgb(LINE)).child(
                            div()
                                .w(relative(fill))
                                .h_full()
                                .rounded_full()
                                .bg(rgb(if failed { RED } else { GREEN })),
                        ),
                    )
                    .child(if let Some(error) = &launch.error {
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgb(0xfff1f0))
                            .text_sm()
                            .text_color(rgb(RED))
                            .child(error.clone())
                            .into_any_element()
                    } else {
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("请稍候，首次启动可能需要更长时间")
                            .into_any_element()
                    }),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_3()
                    .p_4()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new("launch-cancel")
                            .outline()
                            .label(if failed { "关闭" } else { "取消启动" })
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_launch(cx))),
                    )
                    .child(if failed {
                        Button::new("launch-retry")
                            .primary()
                            .label("重试")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.cancel_launch(cx);
                                this.open_browser(id.clone(), cx);
                            }))
                            .into_any_element()
                    } else {
                        Button::new("launch-background")
                            .ghost()
                            .label("在后台继续")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(launch) = this.launch.as_mut() {
                                    launch.visible = false;
                                    cx.notify();
                                }
                            }))
                            .into_any_element()
                    }),
            )
            .overflow_y_scrollbar();
        div()
            .absolute()
            .inset_0()
            .bg(hsla(0., 0., 0., 0.38))
            .flex()
            .items_center()
            .justify_center()
            .child(card)
            .into_any_element()
    }
}

impl Render for BrowserHome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settings_tab.is_some() {
            return div()
                .size_full()
                .relative()
                .bg(rgb(0xffffff))
                .child(
                    v_flex()
                        .size_full()
                        .child(self.render_header(cx))
                        .child(self.render_settings(window, cx)),
                )
                .when(self.storage_busy, |page| {
                    page.child(
                        div()
                            .absolute()
                            .inset_0()
                            .bg(hsla(0., 0., 0., 0.28))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .p_5()
                                    .rounded_md()
                                    .bg(rgb(0xffffff))
                                    .text_color(rgb(INK))
                                    .child(self.storage_status.clone()),
                            ),
                    )
                })
                .into_any_element();
        }
        let rows = if self.rows.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(MUTED))
                .child(if self.loading {
                    "正在加载浏览器…"
                } else {
                    "没有匹配的浏览器"
                })
                .into_any_element()
        } else {
            uniform_list(
                "profile-list",
                self.rows.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    if range.end >= this.rows.len().saturating_sub(20) {
                        this.request_page(cx);
                    }
                    range
                        .map(|index| this.render_row(index, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .mx_4()
            .flex_1()
            .min_h_0()
            .border_x_1()
            .border_b_1()
            .border_color(rgb(LINE))
            .into_any_element()
        };
        div()
            .size_full()
            .relative()
            .bg(rgb(0xffffff))
            .child(
                v_flex()
                    .size_full()
                    .child(self.render_header(cx))
                    .child(self.render_search_bar(cx))
                    .child(self.render_runtime_bar(cx))
                    .child(self.render_table_header(cx))
                    .child(rows),
            )
            .child(self.render_tag_popup(cx))
            .child(self.render_dialog(window, cx))
            .child(self.render_launch_dialog(window, cx))
            .child(if self.status.is_empty() {
                div().into_any_element()
            } else {
                div()
                    .absolute()
                    .bottom(px(16.))
                    .right(px(22.))
                    .p_2()
                    .rounded_md()
                    .bg(rgb(INK))
                    .text_xs()
                    .text_color(rgb(0xffffff))
                    .child(self.status.clone())
                    .into_any_element()
            })
            .into_any_element()
    }
}

fn settings_field(label: &'static str, input: &Entity<InputState>) -> AnyElement {
    h_flex()
        .flex_1()
        .items_center()
        .gap_2()
        .child(div().w(px(90.)).text_sm().text_color(rgb(INK)).child(label))
        .child(div().flex_1().child(Input::new(input)))
        .into_any_element()
}

fn storage_directory_row(
    label: &'static str,
    root: PathBuf,
    kind: Option<StorageKind>,
    busy: bool,
    cx: &mut Context<BrowserHome>,
) -> AnyElement {
    h_flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(170.))
                .text_sm()
                .text_color(rgb(INK))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(root.display().to_string()),
        )
        .when_some(kind, |row, kind| {
            row.child(
                Button::new(format!("change-directory-{}", kind.label()))
                    .outline()
                    .label("选择目录")
                    .disabled(busy)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.choose_storage_root(kind, cx)),
                    ),
            )
        })
        .into_any_element()
}

fn data_location(
    label: &'static str,
    path: PathBuf,
    open_parent: bool,
    cx: &mut Context<BrowserHome>,
) -> AnyElement {
    let target = if open_parent {
        path.parent().unwrap_or(path.as_path()).to_path_buf()
    } else {
        path.clone()
    };
    h_flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(125.))
                .text_sm()
                .text_color(rgb(MUTED))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .text_sm()
                .text_color(rgb(INK))
                .child(path.display().to_string()),
        )
        .child(
            Button::new(format!("open-data-location-{label}"))
                .outline()
                .small()
                .label("打开目录")
                .disabled(!target.is_dir())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let command = if cfg!(target_os = "macos") {
                        "open"
                    } else if cfg!(target_os = "windows") {
                        "explorer"
                    } else {
                        "xdg-open"
                    };
                    if let Err(error) = ProcessCommand::new(command).arg(&target).spawn() {
                        this.browser_status = format!("无法打开目录：{error}");
                        cx.notify();
                    }
                })),
        )
        .into_any_element()
}

fn form_field(
    label: &'static str,
    input: &Entity<InputState>,
    _: &mut Context<BrowserHome>,
) -> AnyElement {
    h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(100.))
                .text_sm()
                .text_color(rgb(INK))
                .child(label),
        )
        .child(div().flex_1().child(Input::new(input)))
        .into_any_element()
}

fn launch_stage_label(stage: LaunchStage) -> &'static str {
    match stage {
        LaunchStage::CheckProxy => "检查代理连接",
        LaunchStage::ClosePeers => "关闭占用该代理的浏览器",
        LaunchStage::SwitchIp => "切换代理出口 IP",
        LaunchStage::GeoIp => "获取出口 IP 与地区",
        LaunchStage::VerifyGeo => "校验 Profile 地区",
        LaunchStage::StartBrowser => "启动 Camoufox",
    }
}

fn geo_field(label: &'static str, input: &Entity<InputState>) -> AnyElement {
    v_flex()
        .flex_1()
        .gap_1()
        .child(div().text_xs().text_color(rgb(MUTED)).child(label))
        .child(Input::new(input).small())
        .into_any_element()
}

fn geo_summary_line(label: &'static str, value: String) -> AnyElement {
    h_flex()
        .text_sm()
        .child(div().w(px(125.)).text_color(rgb(MUTED)).child(label))
        .child(div().text_color(rgb(INK)).child(value))
        .into_any_element()
}

fn format_date(timestamp: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let when = UNIX_EPOCH + Duration::from_secs(timestamp);
    let datetime: chrono::DateTime<chrono::Local> = when.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

#[cfg(test)]
mod launch_tests {
    use std::path::PathBuf;

    use super::LaunchUi;
    use rust_browser::launch_progress::{LaunchEvent, LaunchStage};

    #[test]
    fn close_step_only_appears_when_reported() {
        let mut launch = LaunchUi {
            id: "sample".into(),
            name: "Sample".into(),
            location: "France · Paris".into(),
            progress_file: PathBuf::new(),
            events_seen: 0,
            stage: None,
            close_count: None,
            switch_ip: false,
            error: None,
            visible: true,
        };
        assert!(!launch.steps().contains(&LaunchStage::ClosePeers));
        launch.apply(&LaunchEvent::Stage {
            stage: LaunchStage::ClosePeers,
            detail: Some("2".into()),
        });
        assert_eq!(launch.close_count, Some(2));
        assert_eq!(launch.steps()[1], LaunchStage::ClosePeers);
        launch.apply(&LaunchEvent::Stage {
            stage: LaunchStage::SwitchIp,
            detail: None,
        });
        assert_eq!(launch.steps()[2], LaunchStage::SwitchIp);
    }
}
