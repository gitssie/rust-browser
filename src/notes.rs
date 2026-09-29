//! Markdown notes attached to browser profiles. SQLite is the only source of truth.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};

use crate::storage;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileNote {
    pub markdown: String,
    pub revision: i64,
}

#[derive(Clone)]
pub struct NoteRepository {
    root: PathBuf,
}

impl NoteRepository {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn get(&self, profile_id: &str) -> Result<ProfileNote> {
        let conn = storage::connection(&self.root)?;
        Ok(conn
            .query_row(
                "SELECT markdown, revision FROM profile_notes WHERE profile_id = ?1",
                [profile_id],
                |row| {
                    Ok(ProfileNote {
                        markdown: row.get(0)?,
                        revision: row.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Save only if the database still has the version this editor loaded.
    pub fn save(&self, profile_id: &str, markdown: &str, expected_revision: i64) -> Result<i64> {
        let conn = storage::connection(&self.root)?;
        let updated_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        let next_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("备注版本已用尽"))?;
        let changed = conn.execute(
            "INSERT INTO profile_notes (profile_id, markdown, revision, updated_at)
             SELECT ?1, ?2, ?4, ?5 WHERE EXISTS (SELECT 1 FROM personas WHERE id = ?1)
             ON CONFLICT(profile_id) DO UPDATE SET
                 markdown = excluded.markdown,
                 revision = excluded.revision,
                 updated_at = excluded.updated_at
             WHERE profile_notes.revision = ?3",
            params![
                profile_id,
                markdown,
                expected_revision,
                next_revision,
                updated_at
            ],
        )?;
        if changed == 0 {
            bail!("备注已在其他窗口更新，或浏览器已删除；请复制当前内容后重新加载");
        }
        Ok(next_revision)
    }

    pub fn nonempty_ids(&self) -> Result<HashSet<String>> {
        let conn = storage::connection(&self.root)?;
        let mut stmt = conn.prepare("SELECT profile_id FROM profile_notes WHERE markdown <> ''")?;
        let ids = stmt.query_map([], |row| row.get(0))?;
        ids.collect::<rusqlite::Result<HashSet<_>>>()
            .map_err(Into::into)
    }

    /// Substring matching works for Chinese and ordinary Latin text alike.
    pub fn matching_notes(&self, query: &str) -> Result<HashMap<String, String>> {
        let conn = storage::connection(&self.root)?;
        let mut stmt = conn.prepare("SELECT profile_id, markdown FROM profile_notes")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let needle = query.to_lowercase();
        let mut matches = HashMap::new();
        for row in rows {
            let (id, markdown) = row?;
            if markdown.to_lowercase().contains(&needle) {
                matches.insert(id, markdown);
            }
        }
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saves_and_searches_chinese_markdown_without_losing_newer_edits() {
        use camoufox_core::fingerprint::FingerprintRequest;
        use camoufox_core::persona::PersonaRecord;
        let dir = tempfile::tempdir().unwrap();
        let notes = NoteRepository::new(dir.path());
        let store = storage::open_store(dir.path()).unwrap();
        for id in ["browser-a", "browser-b"] {
            let persona = PersonaRecord::generate(id, &FingerprintRequest::default()).unwrap();
            store.save(&persona).await.unwrap();
        }
        let first = notes
            .save("browser-a", "# 客户资料\n法国站点测试", 0)
            .unwrap();
        let second = notes
            .save("browser-a", "# 客户资料\n联系人：王先生", first)
            .unwrap();
        assert!(notes.save("browser-a", "过期内容", first).is_err());
        assert_eq!(second, 2);
        assert_eq!(
            notes.get("browser-a").unwrap().markdown,
            "# 客户资料\n联系人：王先生"
        );
        assert!(
            notes
                .matching_notes("王先生")
                .unwrap()
                .contains_key("browser-a")
        );
        assert!(
            notes
                .matching_notes("客户")
                .unwrap()
                .contains_key("browser-a")
        );
        notes.save("browser-b", "École française", 0).unwrap();
        assert!(
            notes
                .matching_notes("école")
                .unwrap()
                .contains_key("browser-b")
        );
        assert!(notes.nonempty_ids().unwrap().contains("browser-a"));
        store.delete("browser-a").await.unwrap();
        assert!(notes.save("browser-a", "late autosave", second).is_err());
        assert_eq!(notes.get("browser-a").unwrap().markdown, "");
    }
}
