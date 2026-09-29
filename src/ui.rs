//! Native profile home. Layout and interaction patterns follow rv's GPUI app.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
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
    Disableable, Icon, IconName, Root, Sizable, StyledExt, Theme, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants as _},
    checkbox::Checkbox,
    color_picker::{ColorPicker, ColorPickerState},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    popover::Popover,
    scroll::ScrollableElement,
    select::{Select, SelectEvent, SelectItem, SelectState},
    spinner::Spinner,
    switch::Switch,
    theme::{Colorize as _, try_parse_color},
    v_flex,
};
use rand::Rng as _;
use rust_browser::browser_manager::{
    BrowserManager, DownloadControl, DownloadProgress, DownloadStage, InstalledVersion, Release,
};
use rust_browser::directory_management::{copy_storage, remove_source_storage};
use rust_browser::geo::ProfileGeo;
use rust_browser::launch_progress::{LaunchEvent, LaunchStage, read_events};
use rust_browser::paths::AppPaths;
use rust_browser::profiles::{
    CreateProfile, ProfileGeoInput, ProfileListFilter, ProfileOs, ProfileQuery, ProfileScreen,
    ProfileService, ProfileView, ProxyChoice, UpdateProfile,
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

#[path = "ui/assets.rs"]
mod assets;
#[path = "ui/browser_actions.rs"]
mod browser_actions;
#[path = "ui/cards.rs"]
mod cards;
#[path = "ui/dialogs.rs"]
mod dialogs;
#[path = "ui/home.rs"]
mod home;
#[path = "ui/management_theme.rs"]
mod management_theme;
#[path = "ui/notes.rs"]
mod notes;
#[path = "ui/notifications.rs"]
mod notifications;
#[path = "ui/profile_actions.rs"]
mod profile_actions;
#[path = "ui/settings_page.rs"]
mod settings_page;
#[path = "ui/tag_management.rs"]
mod tag_management;
use assets::app_logo;
use management_theme::{management_add_button, management_row_button};
use notes::NoteEditor;

const GREEN: u32 = 0x008c68;
const GREEN_PALE: u32 = 0xd9f8ee;
const INK: u32 = 0x101f58;
const MUTED: u32 = 0x718197;
const LINE: u32 = 0xdce5f3;
const ROW_ALT: u32 = 0xf8fbfd;
const SELECTED: u32 = 0xe0f8f1;
const PENCIL_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.4 2.6a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg>"#;
const TRASH_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2"/><path d="M19 6l-1 14a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1L5 6"/><path d="M10 11v6M14 11v6"/></svg>"#;
const WINDOWS_ICON: &[u8] = include_bytes!("../assets/fontawesome-windows.svg");
const APPLE_ICON: &[u8] = include_bytes!("../assets/fontawesome-apple.svg");
const LINUX_ICON: &[u8] = include_bytes!("../assets/fontawesome-linux.svg");
const POWER_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/></svg>"#;
const PLAY_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="m10 8 6 4-6 4z"/></svg>"#;
const NAV_BROWSER_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="18" rx="2"/><path d="M2 8h20"/></svg>"#;
const NAV_PROXY_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18"/></svg>"#;
const NAV_TAG_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 4h10l8 8-9 9-9-9V4Z"/><circle cx="8" cy="8" r="1"/></svg>"#;

#[cfg(target_os = "macos")]
actions!(cazer_browser, [Quit]);

fn host_profile_os() -> ProfileOs {
    if cfg!(target_os = "macos") {
        ProfileOs::Macos
    } else if cfg!(target_os = "windows") {
        ProfileOs::Windows
    } else {
        ProfileOs::Linux
    }
}

fn profile_screen(window: &Window, cx: &App) -> Option<ProfileScreen> {
    let display = window.display(cx)?;
    let bounds = display.bounds();
    let visible = display.visible_bounds();
    let dimension = |value: Pixels| f32::from(value).round().max(0.) as u32;
    Some(ProfileScreen {
        width: dimension(bounds.size.width),
        height: dimension(bounds.size.height),
        avail_width: dimension(visible.size.width),
        avail_height: dimension(visible.size.height),
        avail_left: dimension(visible.origin.x - bounds.origin.x),
        avail_top: dimension(visible.origin.y - bounds.origin.y),
    })
}

fn proxy_display_name(choice: &ProxyChoice, managed: &[ManagedProxy]) -> String {
    match choice {
        ProxyChoice::Global => "全局".into(),
        ProxyChoice::Direct => "直连".into(),
        ProxyChoice::Custom(url) => managed
            .iter()
            .find(|proxy| proxy.url == *url)
            .map(|proxy| proxy.name.clone())
            .unwrap_or_else(|| "自定义代理".into()),
    }
}

fn tag_colors(tag: &str) -> (u32, u32) {
    const PALETTE: [(u32, u32); 5] = [
        (0xe8f1ff, 0x0069ed),
        (0xffe8ee, 0xf02e65),
        (0xf0e5ff, 0x9227eb),
        (0xffeedf, 0xee6900),
        (GREEN_PALE, GREEN),
    ];
    let hash = tag.as_bytes().iter().fold(2166136261_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(16777619)
    });
    PALETTE[hash as usize % PALETTE.len()]
}

fn random_tag_color(used_colors: &HashMap<String, String>) -> Hsla {
    const COLORS: [u32; 10] = [
        0x2563eb, 0x7c3aed, 0xdb2777, 0xea580c, 0x0d9488, 0x16a34a, 0xca8a04, 0xdc2626, 0x0891b2,
        0x4f46e5,
    ];
    let unused: Vec<_> = COLORS
        .iter()
        .copied()
        .filter(|color| {
            let hex = format!("#{color:06X}");
            !used_colors
                .values()
                .any(|used| used.eq_ignore_ascii_case(&hex))
        })
        .collect();
    let palette = if unused.is_empty() {
        &COLORS[..]
    } else {
        &unused[..]
    };
    rgb(palette[rand::thread_rng().gen_range(0..palette.len())]).into()
}

fn country_flag(code: &str) -> String {
    let code = code.to_ascii_uppercase();
    if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return "🌐".into();
    }
    code.bytes()
        .filter_map(|byte| char::from_u32(0x1f1e6 + u32::from(byte - b'A')))
        .collect()
}

fn mirror_launch_output(mut source: impl Read + Send + 'static, mut log: File, stderr: bool) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            let size = match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(size) => size,
            };
            let _ = log.write_all(&buffer[..size]);
            if stderr {
                let _ = std::io::stderr().write_all(&buffer[..size]);
            } else {
                let _ = std::io::stdout().write_all(&buffer[..size]);
            }
        }
    });
}
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProfileProxySelection {
    Global,
    Direct,
    Managed(String),
    ExistingCustom(String),
}

#[derive(Clone)]
struct ProfileProxyOption {
    label: SharedString,
    value: ProfileProxySelection,
}

impl SelectItem for ProfileProxyOption {
    type Value = ProfileProxySelection;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

fn profile_proxy_options(
    global: &ProxySettings,
    managed: &[ManagedProxy],
    current: Option<&ProxyChoice>,
) -> Vec<ProfileProxyOption> {
    let mut options = vec![
        ProfileProxyOption {
            label: format!("全局代理 · {}", global.browser_url()).into(),
            value: ProfileProxySelection::Global,
        },
        ProfileProxyOption {
            label: "直连（不使用代理）".into(),
            value: ProfileProxySelection::Direct,
        },
    ];
    options.extend(managed.iter().map(|proxy| ProfileProxyOption {
        label: format!("{} · {}", proxy.name, proxy.url).into(),
        value: ProfileProxySelection::Managed(proxy.id.clone()),
    }));
    if let Some(ProxyChoice::Custom(url)) = current
        && !managed.iter().any(|proxy| &proxy.url == url)
    {
        options.push(ProfileProxyOption {
            label: format!("当前独立代理 · {url}").into(),
            value: ProfileProxySelection::ExistingCustom(url.clone()),
        });
    }
    options
}

fn profile_proxy_choice(
    selection: &ProfileProxySelection,
    managed: &[ManagedProxy],
) -> Result<ProxyChoice> {
    match selection {
        ProfileProxySelection::Global => Ok(ProxyChoice::Global),
        ProfileProxySelection::Direct => Ok(ProxyChoice::Direct),
        ProfileProxySelection::Managed(id) => managed
            .iter()
            .find(|proxy| &proxy.id == id)
            .map(|proxy| ProxyChoice::Custom(proxy.url.clone()))
            .ok_or_else(|| anyhow!("所选代理已不存在，请重新选择")),
        ProfileProxySelection::ExistingCustom(url) => Ok(ProxyChoice::Custom(url.clone())),
    }
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
        {
            let theme = Theme::global_mut(cx);
            theme.primary = rgb(GREEN).into();
            theme.primary_hover = rgb(0x007c5c).into();
            theme.primary_active = rgb(0x006b50).into();
            theme.button_primary = rgb(GREEN).into();
            theme.button_primary_hover = rgb(0x007c5c).into();
            theme.button_primary_active = rgb(0x006b50).into();
            theme.tokens.primary = theme.primary.into();
            theme.tokens.primary_hover = theme.primary_hover.into();
            theme.tokens.primary_active = theme.primary_active.into();
            theme.tokens.button_primary = theme.button_primary.into();
            theme.tokens.button_primary_hover = theme.button_primary_hover.into();
            theme.tokens.button_primary_active = theme.button_primary_active.into();
        }
        Theme::sync_base(cx);
        cx.set_quit_mode(QuitMode::LastWindowClosed);
        #[cfg(target_os = "macos")]
        {
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus([
                Menu::new("Cazer Browser").items([MenuItem::action("退出 Cazer Browser", Quit)])
            ]);
            cx.activate(true);
            set_native_app_menu_name();
            set_native_app_icon();
        }
        let bounds = Bounds::centered(None, size(px(1440.), px(840.)), cx);
        let mut options = WindowOptions::default();
        if let Some(titlebar) = options.titlebar.as_mut() {
            titlebar.title = Some("Cazer Browser".into());
        }
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
        options.window_min_size = Some(size(px(1050.), px(600.)));
        options.app_id = Some("app.cazer.browser".into());
        cx.spawn(async move |cx| {
            let window_handle = cx
                .open_window(options, move |window, cx| {
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
                    let root = cx.new(|cx| Root::new(home, window, cx));
                    window.blur(cx);
                    root
                })
                .expect("open browser manager");
            #[cfg(target_os = "macos")]
            {
                let native_window = window_handle
                    .update(cx, |_, window, _| native_window_for(window))
                    .expect("find macOS window");
                if let Some(native_window) = native_window {
                    install_native_toolbar(&native_window);
                }
            }
        })
        .detach();
    });
    Ok(())
}

#[cfg(target_os = "macos")]
fn native_window_for(window: &Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // GPUI's AppKit handle points to its NSView. AppKit owns the view for the
    // window lifetime, and this callback runs on the UI thread.
    let view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
    view.window()
}

#[cfg(target_os = "macos")]
fn install_native_toolbar(native_window: &objc2_app_kit::NSWindow) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSToolbar, NSWindowTitleVisibility, NSWindowToolbarStyle};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let toolbar = NSToolbar::new(mtm);
    toolbar.setVisible(true);
    native_window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
    native_window.setToolbar(Some(&toolbar));
    native_window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
}

#[cfg(target_os = "macos")]
fn set_native_app_menu_name() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else {
        return;
    };
    let Some(app_item) = menu.itemAtIndex(0) else {
        return;
    };
    let title = NSString::from_str("Cazer Browser");
    app_item.setTitle(&title);
    if let Some(submenu) = app_item.submenu() {
        submenu.setTitle(&title);
    }
}

#[cfg(target_os = "macos")]
fn set_native_app_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../assets/cazer-logo.png"));
    let Some(icon) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&icon)) };
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
    search: Entity<InputState>,
    form_name: Entity<InputState>,
    form_url: Entity<InputState>,
    form_extra_tabs: Vec<Entity<InputState>>,
    form_proxy_select: Entity<SelectState<Vec<ProfileProxyOption>>>,
    form_country_code: Entity<InputState>,
    form_country: Entity<InputState>,
    form_region: Entity<InputState>,
    form_city: Entity<InputState>,
    form_timezone: Entity<InputState>,
    form_locale: Entity<InputState>,
    form_latitude: Entity<InputState>,
    form_longitude: Entity<InputState>,
    tag_input: Entity<InputState>,
    tag_color_picker: Entity<ColorPickerState>,
    _subscriptions: Vec<Subscription>,
    rows: Vec<ProfileView>,
    page: usize,
    total: usize,
    loading: bool,
    has_more: bool,
    generation: u64,
    status_filter: Option<bool>,
    proxy_filter: Option<ProxyChoice>,
    filter_tags: Vec<String>,
    filter_tag_options: Vec<String>,
    filter_tags_generation: u64,
    all_tags: Vec<String>,
    tag_custom_colors: HashMap<String, String>,
    selected_tag: Option<String>,
    tag_form_error: String,
    running: HashSet<String>,
    dialog: Dialog,
    form_os: ProfileOs,
    create_mode: CreateMode,
    geo_preview: Option<ProfileGeo>,
    geo_loading: bool,
    geo_generation: u64,
    geo_error: String,
    busy: bool,
    pending_notice: Option<Notification>,
    status_generation: u64,
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
                        let target = dunce::canonicalize(target)?;
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
        self.browser_status = "正在检查浏览器版本…".into();
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
                    Ok(()) => "浏览器安装完成".into(),
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
                    .unwrap_or(10);
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

    fn open_managed_editor(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_edit = id.is_some();
        self.select_managed(id, window, cx);
        let home = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let content_home = home.clone();
            let save_home = home.clone();
            dialog
                .title(if is_edit {
                    "编辑代理"
                } else {
                    "添加代理"
                })
                .w(px(560.))
                .content(move |content, _, cx| {
                    content.child(
                        content_home
                            .read(cx)
                            .render_managed_editor(content_home.clone()),
                    )
                })
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cancel-managed")
                                .outline()
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("save-managed")
                                .primary()
                                .label("保存代理")
                                .on_click(move |_, window, cx| {
                                    if save_home.update(cx, |this, cx| this.save_managed(cx)) {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
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

    fn save_managed(&mut self, cx: &mut Context<Self>) -> bool {
        let id = self.managed_id.read(cx).value().trim().to_string();
        if self
            .managed_selected
            .as_deref()
            .is_some_and(|selected| selected != id)
        {
            self.managed_status = "现有代理的 ID 不可修改".into();
            cx.notify();
            return false;
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
                match ProxyCatalog::new(self.service.data_dir()).list() {
                    Ok(managed) => {
                        self.managed = managed;
                        self.managed_status = "代理已保存".into();
                        cx.notify();
                        return true;
                    }
                    Err(error) => self.managed_status = format!("读取代理失败：{error}"),
                }
            }
            Err(error) => self.managed_status = format!("保存失败：{error}"),
        }
        cx.notify();
        false
    }

    fn remove_managed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.managed_selected.clone() else {
            return;
        };
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
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索"));
        let form_name = cx.new(|cx| InputState::new(window, cx).placeholder("显示名称"));
        let form_url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://www.vinted.fr/"));
        let managed = ProxyCatalog::new(service.data_dir())
            .list()
            .unwrap_or_default();
        let form_proxy_select = cx.new(|cx| {
            SelectState::new(
                profile_proxy_options(&global_proxy, &managed, None),
                Some(Default::default()),
                window,
                cx,
            )
            .searchable(true)
        });
        let form_country_code = cx.new(|cx| InputState::new(window, cx).placeholder("FR"));
        let form_country = cx.new(|cx| InputState::new(window, cx).placeholder("France"));
        let form_region = cx.new(|cx| InputState::new(window, cx).placeholder("Île-de-France"));
        let form_city = cx.new(|cx| InputState::new(window, cx).placeholder("Paris"));
        let form_timezone = cx.new(|cx| InputState::new(window, cx).placeholder("Europe/Paris"));
        let form_locale = cx.new(|cx| InputState::new(window, cx).placeholder("fr-FR"));
        let form_latitude = cx.new(|cx| InputState::new(window, cx).placeholder("48.86"));
        let form_longitude = cx.new(|cx| InputState::new(window, cx).placeholder("2.35"));
        let tag_input = cx.new(|cx| InputState::new(window, cx).placeholder("输入标签名称"));
        let tag_color_picker =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(GREEN)));
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
        let managed_wait = cx.new(|cx| InputState::new(window, cx).default_value("10"));
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
        let proxy_subscription = cx.subscribe_in(
            &form_proxy_select,
            window,
            |this, _, event: &SelectEvent<Vec<ProfileProxyOption>>, _, cx| {
                if matches!(event, SelectEvent::Confirm(_)) && matches!(this.dialog, Dialog::Create)
                {
                    this.detect_geo(cx);
                }
            },
        );
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
            search,
            form_name,
            form_url,
            form_extra_tabs: Vec::new(),
            form_proxy_select,
            form_country_code,
            form_country,
            form_region,
            form_city,
            form_timezone,
            form_locale,
            form_latitude,
            form_longitude,
            tag_input,
            tag_color_picker,
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
            status_filter: None,
            proxy_filter: None,
            filter_tags: Vec::new(),
            filter_tag_options: Vec::new(),
            filter_tags_generation: 0,
            all_tags: Vec::new(),
            tag_custom_colors: HashMap::new(),
            selected_tag: None,
            tag_form_error: String::new(),
            running: HashSet::new(),
            dialog: Dialog::None,
            form_os: host_profile_os(),
            create_mode: CreateMode::Smart,
            geo_preview: None,
            geo_loading: false,
            geo_generation: 0,
            geo_error: String::new(),
            busy: false,
            pending_notice: None,
            status_generation: 0,
            spawned: HashMap::new(),
            launch: None,
        };
        this.reload(cx);
        this.load_tags(cx);
        this.refresh_running(cx);
        this.start_polling(cx);
        this
    }
}

impl Render for BrowserHome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_pending_notice(window, cx);
        if self.settings_tab.is_some() {
            return div()
                .size_full()
                .relative()
                .bg(rgb(0xf7fafc))
                .child(
                    h_flex()
                        .size_full()
                        .child(self.render_navigation(cx))
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
                .children(Root::render_dialog_layer(window, cx))
                .children(Root::render_notification_layer(window, cx))
                .into_any_element();
        }
        let visible_rows = self.visible_row_indices();
        if visible_rows.is_empty() && self.status_filter.is_some() && self.has_more && !self.loading
        {
            self.request_page(cx);
        }
        let content_width = (f32::from(window.bounds().size.width) - 204. - 32.).max(0.);
        let columns = ((content_width + 12.) / 304.).floor().max(1.) as usize;
        let cards = if visible_rows.is_empty() {
            div()
                .absolute()
                .inset_0()
                .pt(px(72.))
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
            let visible_count = visible_rows.len();
            let row_count = visible_count.div_ceil(columns);
            uniform_list(
                SharedString::from(format!("profile-cards-{columns}")),
                row_count,
                cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                    if range.end >= row_count.saturating_sub(3) {
                        this.request_page(cx);
                    }
                    range
                        .map(|row| {
                            let start = row * columns;
                            let end = (start + columns).min(visible_count);
                            h_flex()
                                .w_full()
                                .h(px(162.))
                                .gap_3()
                                .children((start..end).map(|index| {
                                    this.render_profile_card(visible_rows[index], window, cx)
                                }))
                                .children((end..start + columns).map(|_| div().flex_1()))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .absolute()
            .inset_0()
            .p_4()
            .pt(px(72.))
            .into_any_element()
        };
        div()
            .size_full()
            .relative()
            .bg(rgb(0xf7fafc))
            .child(
                h_flex()
                    .size_full()
                    .child(self.render_navigation(cx))
                    .child(
                        div().flex_1().h_full().relative().child(cards).child(
                            div()
                                .absolute()
                                .top(px(16.))
                                .left(px(16.))
                                .right(px(16.))
                                .child(self.render_search_bar(cx)),
                        ),
                    ),
            )
            .child(self.render_dialog(window, cx))
            .child(self.render_launch_dialog(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
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
    kind: StorageKind,
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
        .child(
            Button::new(format!("change-directory-{}", kind.label()))
                .outline()
                .small()
                .label("选择目录")
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| this.choose_storage_root(kind, cx))),
        )
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
        LaunchStage::CheckProxy => "连接网络",
        LaunchStage::ClosePeers => "准备网络",
        LaunchStage::SwitchIp => "切换网络",
        LaunchStage::GeoIp => "获取当前位置",
        LaunchStage::VerifyGeo => "确认地区",
        LaunchStage::StartBrowser => "打开浏览器",
    }
}

fn launch_error_message(error: &str) -> &'static str {
    if error.contains("location mismatch") {
        "当前网络地区与浏览器保存的地区不一致，请更换网络后重试。"
    } else if error.contains("proxy") || error.contains("代理") {
        "网络连接失败，请检查代理设置后重试。"
    } else if error.contains("not installed") || error.contains("未安装") {
        "浏览器组件尚未安装，请先在全局设置中安装。"
    } else {
        "浏览器未能打开，请检查网络和浏览器设置后重试。"
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

    use super::{
        LaunchUi, ProfileProxySelection, profile_proxy_choice, profile_proxy_options,
        proxy_display_name,
    };
    use rust_browser::launch_progress::{LaunchEvent, LaunchStage};
    use rust_browser::profiles::ProxyChoice;
    use rust_browser::proxy::ProxySettings;
    use rust_browser::proxy_management::{ManagedProxy, ProxyPolicy};

    #[test]
    fn proxy_dropdown_combines_global_and_managed_with_existing_fallback() {
        let global = ProxySettings::parse("socks5://127.0.0.1:12334").unwrap();
        let managed = vec![ManagedProxy {
            id: "fr".into(),
            name: "法国代理".into(),
            url: "socks5://proxy.example:1080".into(),
            credentials: None,
            policy: ProxyPolicy::AllowParallel,
            ip_switch: None,
        }];
        let options = profile_proxy_options(&global, &managed, None);
        assert_eq!(options.len(), 3);
        assert_eq!(proxy_display_name(&ProxyChoice::Global, &managed), "全局");
        assert_eq!(proxy_display_name(&ProxyChoice::Direct, &managed), "直连");
        assert_eq!(
            proxy_display_name(&ProxyChoice::Custom(managed[0].url.clone()), &managed),
            "法国代理"
        );
        assert!(matches!(options[0].value, ProfileProxySelection::Global));
        assert_eq!(options[1].label.as_ref(), "直连（不使用代理）");
        assert!(matches!(
            profile_proxy_choice(&options[1].value, &managed).unwrap(),
            ProxyChoice::Direct
        ));
        assert_eq!(
            options[2].value,
            ProfileProxySelection::Managed("fr".into())
        );
        assert!(matches!(
            profile_proxy_choice(&options[2].value, &managed).unwrap(),
            ProxyChoice::Custom(url) if url == "socks5://proxy.example:1080"
        ));
        assert_eq!(
            profile_proxy_options(
                &global,
                &managed,
                Some(&ProxyChoice::Custom("socks5://proxy.example:1080".into()))
            )
            .len(),
            3
        );
        let existing = profile_proxy_options(
            &global,
            &managed,
            Some(&ProxyChoice::Custom("socks5://other.example:1080".into())),
        );
        assert_eq!(existing.len(), 4);
        assert!(matches!(
            profile_proxy_choice(&existing[3].value, &managed).unwrap(),
            ProxyChoice::Custom(url) if url == "socks5://other.example:1080"
        ));
    }

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
