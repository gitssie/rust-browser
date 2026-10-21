//! Named startup-tab presets. Profiles save URLs, so later preset edits do not
//! change an existing browser's tabs.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::storage;

const SETTINGS_KEY: &str = "browser_tab_presets";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTabPreset {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTabSettings {
    pub presets: Vec<BrowserTabPreset>,
}

impl Default for BrowserTabSettings {
    fn default() -> Self {
        Self {
            presets: vec![BrowserTabPreset {
                name: "bacon".into(),
                url: "https://www.vinted.fr/".into(),
            }],
        }
    }
}

impl BrowserTabSettings {
    pub fn normalized(&self) -> Result<Self> {
        let mut names = HashSet::new();
        let mut presets = Vec::with_capacity(self.presets.len());
        for preset in &self.presets {
            let name = preset.name.trim();
            if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
                bail!("常用标签页名称须为 1–80 个字符，且不能包含控制字符");
            }
            if !names.insert(name.to_lowercase()) {
                bail!("常用标签页名称重复：{name}");
            }
            let value = preset.url.trim();
            let url = match Url::parse(value) {
                Ok(url) => url,
                Err(_) => Url::parse(&format!("https://{value}"))?,
            };
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                bail!("标签页网址必须是有效的 HTTP 或 HTTPS 地址");
            }
            if !url.username().is_empty() || url.password().is_some() {
                bail!("标签页网址不能包含用户名或密码");
            }
            presets.push(BrowserTabPreset {
                name: name.into(),
                url: url.to_string(),
            });
        }
        Ok(Self { presets })
    }

    pub fn load(root: &Path) -> Result<Self> {
        let conn = storage::connection(root)?;
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        match value {
            Some(value) => serde_json::from_str::<Self>(&value)?.normalized(),
            None => Ok(Self::default()),
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let settings = self.normalized()?;
        let conn = storage::connection(root)?;
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SETTINGS_KEY, serde_json::to_string(&settings)?],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_bacon_and_bare_domains_become_https() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            BrowserTabSettings::load(dir.path()).unwrap(),
            BrowserTabSettings::default()
        );
        let settings = BrowserTabSettings {
            presets: vec![BrowserTabPreset {
                name: " Vinted ".into(),
                url: " www.vinted.fr ".into(),
            }],
        };
        settings.save(dir.path()).unwrap();
        let saved = BrowserTabSettings::load(dir.path()).unwrap();
        assert_eq!(saved.presets[0].name, "Vinted");
        assert_eq!(saved.presets[0].url, "https://www.vinted.fr/");
    }

    #[test]
    fn editing_and_removing_all_presets_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = BrowserTabSettings::default();
        settings.presets.push(BrowserTabPreset {
            name: "Other".into(),
            url: "https://example.com/".into(),
        });
        settings.save(dir.path()).unwrap();
        settings.presets[0].name = "Vinted France".into();
        settings.presets[0].url = "https://www.vinted.fr/catalog".into();
        settings.save(dir.path()).unwrap();
        assert_eq!(BrowserTabSettings::load(dir.path()).unwrap(), settings);
        settings.presets.clear();
        settings.save(dir.path()).unwrap();
        assert!(
            BrowserTabSettings::load(dir.path())
                .unwrap()
                .presets
                .is_empty()
        );
    }

    #[test]
    fn invalid_changes_do_not_overwrite_saved_presets() {
        let dir = tempfile::tempdir().unwrap();
        let original = BrowserTabSettings::default();
        original.save(dir.path()).unwrap();
        for (name, url) in [
            ("", "https://example.com/"),
            ("bad", "javascript:alert(1)"),
            ("bad", "file:///tmp/test"),
            ("bad", "https://user:pass@example.com/"),
        ] {
            let bad = BrowserTabSettings {
                presets: vec![BrowserTabPreset {
                    name: name.into(),
                    url: url.into(),
                }],
            };
            assert!(bad.save(dir.path()).is_err());
            assert_eq!(BrowserTabSettings::load(dir.path()).unwrap(), original);
        }
        let mut duplicates = original.clone();
        duplicates.presets.push(BrowserTabPreset {
            name: "BACON".into(),
            url: "https://example.com/".into(),
        });
        assert!(duplicates.save(dir.path()).is_err());
    }
}
