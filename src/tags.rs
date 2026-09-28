//! Reusable label catalogue. Profile label snapshots remain independent.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use rusqlite::params;

use crate::storage;

#[derive(Clone)]
pub struct TagCatalog {
    root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEntry {
    pub name: String,
    pub color: Option<String>,
}

impl TagCatalog {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn list(&self) -> Result<Vec<String>> {
        let conn = storage::connection(&self.root)?;
        let mut statement = conn.prepare("SELECT name FROM tags ORDER BY name COLLATE NOCASE")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn list_entries(&self) -> Result<Vec<TagEntry>> {
        let conn = storage::connection(&self.root)?;
        let mut statement =
            conn.prepare("SELECT name, color FROM tags ORDER BY name COLLATE NOCASE")?;
        let rows = statement.query_map([], |row| {
            Ok(TagEntry {
                name: row.get(0)?,
                color: row.get(1)?,
            })
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn add(&self, name: &str) -> Result<()> {
        let name = validate(name)?;
        let conn = storage::connection(&self.root)?;
        conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])?;
        Ok(())
    }

    pub fn add_with_color(&self, name: &str, color: &str) -> Result<()> {
        let name = validate(name)?;
        let color = validate_color(color)?;
        let conn = storage::connection(&self.root)?;
        conn.execute(
            "INSERT INTO tags (name, color) VALUES (?1, ?2)",
            params![name, color],
        )
        .map_err(friendly_write_error)?;
        Ok(())
    }

    pub fn update(&self, old: &str, new: &str, color: &str) -> Result<()> {
        let old = validate(old)?;
        let new = validate(new)?;
        let color = validate_color(color)?;
        let conn = storage::connection(&self.root)?;
        if conn
            .execute(
                "UPDATE tags SET name = ?1, color = ?2 WHERE name = ?3",
                params![new, color, old],
            )
            .map_err(friendly_write_error)?
            == 0
        {
            bail!("tag {old} not found");
        }
        Ok(())
    }

    pub fn rename(&self, old: &str, new: &str) -> Result<()> {
        let old = validate(old)?;
        let new = validate(new)?;
        let conn = storage::connection(&self.root)?;
        if conn.execute(
            "UPDATE tags SET name = ?1 WHERE name = ?2",
            params![new, old],
        )? == 0
        {
            bail!("tag {old} not found");
        }
        Ok(())
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let name = validate(name)?;
        let conn = storage::connection(&self.root)?;
        if conn.execute("DELETE FROM tags WHERE name = ?1", [name])? == 0 {
            bail!("tag {name} not found");
        }
        Ok(())
    }
}

fn validate(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 24 || name.chars().any(char::is_control) {
        bail!("标签名称须为 1–24 个可显示字符");
    }
    Ok(name)
}

fn validate_color(color: &str) -> Result<&str> {
    if !matches!(color.len(), 7 | 9)
        || !color.starts_with('#')
        || !color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("标签颜色格式无效");
    }
    Ok(color)
}

fn friendly_write_error(error: rusqlite::Error) -> anyhow::Error {
    match error {
        rusqlite::Error::SqliteFailure(sqlite_error, _)
            if sqlite_error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            anyhow!("标签名称已存在，请更换名称")
        }
        error => error.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camoufox_core::fingerprint::FingerprintRequest;
    use camoufox_core::persona::PersonaRecord;
    use serde_json::json;

    #[test]
    fn catalogue_crud() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let catalogue = TagCatalog::new(root);
        catalogue.add("法国").unwrap();
        catalogue.add("法国").unwrap();
        assert_eq!(catalogue.list().unwrap(), ["法国"]);
        catalogue.rename("法国", "账号").unwrap();
        assert_eq!(catalogue.list().unwrap(), ["账号"]);
        catalogue.remove("账号").unwrap();
        assert!(catalogue.list().unwrap().is_empty());
    }

    #[test]
    fn custom_color_persists_through_rename() {
        let dir = tempfile::tempdir().unwrap();
        let catalogue = TagCatalog::new(dir.path());
        catalogue.add_with_color("法国", "#008C68").unwrap();
        catalogue.update("法国", "主账号", "#A23C7F").unwrap();
        assert!(
            catalogue
                .add_with_color("主账号", "#2563EB")
                .unwrap_err()
                .to_string()
                .contains("标签名称已存在")
        );
        assert_eq!(
            catalogue.list_entries().unwrap(),
            [TagEntry {
                name: "主账号".into(),
                color: Some("#A23C7F".into()),
            }]
        );
    }

    #[test]
    fn legacy_tags_gain_optional_color_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = storage::database_path(dir.path());
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE tags (name TEXT PRIMARY KEY COLLATE NOCASE);
             INSERT INTO tags (name) VALUES ('旧标签');",
        )
        .unwrap();
        drop(conn);
        let catalogue = TagCatalog::new(dir.path());
        assert_eq!(
            catalogue.list_entries().unwrap(),
            [TagEntry {
                name: "旧标签".into(),
                color: None,
            }]
        );
    }

    #[tokio::test]
    async fn removing_catalogue_tag_preserves_profile_tag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let store = storage::open_store(root).unwrap();
        let mut persona =
            PersonaRecord::generate("profile-a", &FingerprintRequest::default()).unwrap();
        persona.metadata.insert("tags".into(), json!(["法国"]));
        store.save(&persona).await.unwrap();
        let catalogue = TagCatalog::new(root);
        catalogue.add("法国").unwrap();
        catalogue.remove("法国").unwrap();
        let saved = store.load("profile-a").await.unwrap().unwrap();
        assert_eq!(saved.metadata["tags"], json!(["法国"]));
    }
}
