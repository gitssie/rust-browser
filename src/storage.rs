//! Shared SQLite database for camoufox personas and managed proxy rules.

use std::fs::{self, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use camoufox_store::{PersonaStore, SqliteStore};
use rusqlite::Connection;

pub fn database_path(root: &Path) -> PathBuf {
    root.join("browser.sqlite")
}

pub fn open_store(root: &Path) -> Result<PersonaStore> {
    initialize(root)?;
    Ok(PersonaStore::new(Box::new(SqliteStore::open(
        database_path(root),
    )?)))
}

pub fn connection(root: &Path) -> Result<Connection> {
    initialize(root)?;
    let conn = Connection::open(database_path(root))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

fn initialize(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    let path = database_path(root);
    #[cfg(unix)]
    {
        // General proxy credentials are kept in this database.
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    // Upstream camoufox-store owns the personas, sessions and profile_files tables.
    SqliteStore::open(&path)?;
    let conn = Connection::open(&path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS managed_proxies (
            id TEXT PRIMARY KEY,
            url TEXT NOT NULL UNIQUE,
            data TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tags (
            name TEXT PRIMARY KEY COLLATE NOCASE
        );",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy_management::{ManagedProxy, ProxyCatalog, ProxyPolicy};
    use camoufox_core::fingerprint::FingerprintRequest;
    use camoufox_core::persona::PersonaRecord;

    #[tokio::test]
    async fn personas_and_proxies_persist_in_the_same_database() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let persona = PersonaRecord::generate("alice", &FingerprintRequest::default()).unwrap();
        open_store(root).unwrap().save(&persona).await.unwrap();
        let proxy = ManagedProxy {
            id: "local".into(),
            name: "Local".into(),
            url: "socks5://localhost:1234".into(),
            credentials: None,
            policy: ProxyPolicy::AllowParallel,
            ip_switch: None,
        };
        ProxyCatalog::new(root).upsert(proxy).unwrap();
        assert!(database_path(root).is_file());
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(database_path(root))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!root.join("personas").exists());
        assert!(!root.join("managed-proxies.json").exists());
        assert!(
            open_store(root)
                .unwrap()
                .load("alice")
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(ProxyCatalog::new(root).list().unwrap().len(), 1);
        let conn = connection(root).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM personas", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
