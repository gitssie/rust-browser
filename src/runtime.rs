//! Local control socket for live CLI browser sessions.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::paths::AppPaths;

#[derive(Clone)]
pub struct BrowserRuntime {
    paths: AppPaths,
}

pub struct BrowserListener {
    listener: UnixListener,
    path: PathBuf,
}

impl Drop for BrowserListener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl BrowserRuntime {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            paths: AppPaths::for_root(data_dir),
        }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.paths.runtime_socket(id)
    }

    pub async fn bind(&self, id: &str) -> Result<BrowserListener> {
        let directory = self.paths.runtime_sockets();
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let path = self.path(id);
        if path.exists() {
            if self.is_running(id).await {
                bail!("browser {id} is already running");
            }
            fs::remove_file(&path)
                .with_context(|| format!("remove stale runtime socket for {id}"))?;
        }
        let listener =
            UnixListener::bind(&path).with_context(|| format!("bind runtime socket for {id}"))?;
        Ok(BrowserListener { listener, path })
    }

    pub async fn is_running(&self, id: &str) -> bool {
        self.request(id, b"PING")
            .await
            .is_ok_and(|reply| reply == b"PONG")
    }

    pub async fn close(&self, id: &str) -> Result<()> {
        let reply = self.request(id, b"CLOSE").await?;
        if reply != b"OK" {
            bail!("unexpected close response for {id}");
        }
        Ok(())
    }

    async fn request(&self, id: &str, command: &[u8]) -> Result<Vec<u8>> {
        let path = self.path(id);
        let mut stream =
            tokio::time::timeout(Duration::from_millis(250), UnixStream::connect(&path)).await??;
        stream.write_all(command).await?;
        let mut reply = [0u8; 8];
        let length =
            tokio::time::timeout(Duration::from_millis(250), stream.read(&mut reply)).await??;
        Ok(reply[..length].to_vec())
    }
}

impl BrowserListener {
    pub async fn wait_for_close(&self) -> Result<()> {
        loop {
            let (mut stream, _) = self.listener.accept().await?;
            let mut command = [0u8; 16];
            let length =
                tokio::time::timeout(Duration::from_secs(2), stream.read(&mut command)).await??;
            match &command[..length] {
                b"PING" => stream.write_all(b"PONG").await?,
                b"CLOSE" => {
                    stream.write_all(b"OK").await?;
                    return Ok(());
                }
                _ => {
                    stream.write_all(b"ERROR").await?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn socket_tracks_and_closes_one_browser() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = BrowserRuntime::new(dir.path());
        let listener = runtime.bind("alpha").await.unwrap();
        let task = tokio::spawn(async move { listener.wait_for_close().await.unwrap() });
        assert!(runtime.is_running("alpha").await);
        assert!(!runtime.is_running("beta").await);
        runtime.close("alpha").await.unwrap();
        task.await.unwrap();
        assert!(!runtime.is_running("alpha").await);
    }
}
