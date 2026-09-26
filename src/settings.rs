//! Local application settings stored in the same SQLite database as profiles.

use std::path::Path;

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::proxy::{DEFAULT_PROXY, ProxySettings};
use crate::storage;

const GENERAL_KEY: &str = "general_proxy";
const BROWSER_DOWNLOAD_KEY: &str = "browser_download";

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserDownloadSettings {
    pub use_global_proxy: bool,
}

impl BrowserDownloadSettings {
    pub fn load(root: &Path) -> Result<Self> {
        let conn = storage::connection(root)?;
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [BROWSER_DOWNLOAD_KEY],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .unwrap_or_else(|| Ok(Self::default()))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let conn = storage::connection(root)?;
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![BROWSER_DOWNLOAD_KEY, serde_json::to_string(self)?],
        )?;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub remote_dns: bool,
    pub timeout_seconds: u64,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        let proxy = ProxySettings::parse(DEFAULT_PROXY).expect("valid built-in proxy default");
        Self {
            host: proxy.host().to_string(),
            port: proxy.port(),
            username: String::new(),
            password: String::new(),
            remote_dns: true,
            timeout_seconds: 10,
        }
    }
}

impl GeneralSettings {
    pub fn proxy(&self) -> Result<ProxySettings> {
        ProxySettings::configured(
            &self.host,
            self.port,
            &self.username,
            &self.password,
            self.remote_dns,
            self.timeout_seconds,
        )
    }

    pub fn load(root: &Path) -> Result<Self> {
        let conn = storage::connection(root)?;
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [GENERAL_KEY],
                |row| row.get(0),
            )
            .optional()?;
        let settings = match value {
            Some(value) => serde_json::from_str(&value)?,
            None => Self::default(),
        };
        settings.proxy()?;
        Ok(settings)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        self.proxy()?;
        let conn = storage::connection(root)?;
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![GENERAL_KEY, serde_json::to_string(self)?],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_proxy_roundtrips_without_touching_other_tables() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = GeneralSettings::load(dir.path()).unwrap();
        settings.host = "proxy.example".into();
        settings.port = 1080;
        settings.username = "alice".into();
        settings.password = "secret".into();
        settings.remote_dns = false;
        settings.save(dir.path()).unwrap();
        assert!(GeneralSettings::load(dir.path()).unwrap() == settings);
        assert_eq!(
            settings.proxy().unwrap().browser_url(),
            "socks5://proxy.example:1080"
        );
    }

    #[test]
    fn browser_download_proxy_choice_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            !BrowserDownloadSettings::load(dir.path())
                .unwrap()
                .use_global_proxy
        );
        BrowserDownloadSettings {
            use_global_proxy: true,
        }
        .save(dir.path())
        .unwrap();
        assert!(
            BrowserDownloadSettings::load(dir.path())
                .unwrap()
                .use_global_proxy
        );
    }
}
