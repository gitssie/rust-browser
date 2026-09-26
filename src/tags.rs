//! Reusable label catalogue. Profile label snapshots remain independent.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use rusqlite::params;

use crate::storage;

#[derive(Clone)]
pub struct TagCatalog {
    root: PathBuf,
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

    pub fn add(&self, name: &str) -> Result<()> {
        let name = validate(name)?;
        let conn = storage::connection(&self.root)?;
        conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])?;
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
        bail!("tag must contain 1-24 printable characters");
    }
    Ok(name)
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
