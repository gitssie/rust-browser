//! Native profile management API for the CLI and a future Rust UI.

pub mod browser_manager;
pub mod directory_management;
pub mod geo;
pub mod launch_progress;
pub mod paths;
pub mod profiles;
pub mod proxy;
pub mod proxy_management;
#[cfg(unix)]
pub mod runtime;
#[cfg(windows)]
#[path = "runtime_windows.rs"]
pub mod runtime;
pub mod settings;
pub mod storage;
pub mod tags;
pub mod workspace_lock;
