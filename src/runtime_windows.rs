//! Local named-pipe control for live browser sessions on Windows.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_BUSY};

use crate::paths::AppPaths;

#[derive(Clone)]
pub struct BrowserRuntime {
    paths: AppPaths,
}

pub struct BrowserListener {
    server: NamedPipeServer,
    name: String,
}

impl BrowserRuntime {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            paths: AppPaths::for_root(data_dir),
        }
    }

    pub async fn bind(&self, id: &str) -> Result<BrowserListener> {
        let name = self.paths.runtime_pipe(id);
        // A named pipe disappears when its owning process exits. The first
        // instance flag rejects a second browser session with the same ID.
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .with_context(|| format!("bind runtime pipe for {id}"))?;
        Ok(BrowserListener { server, name })
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
        let name = self.paths.runtime_pipe(id);
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut stream = loop {
            match ClientOptions::new().open(&name) {
                Ok(stream) => break stream,
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                        && Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error).with_context(|| format!("connect to {id}")),
            }
        };
        stream.write_all(command).await?;
        let mut reply = [0u8; 8];
        let length =
            tokio::time::timeout(Duration::from_millis(250), stream.read(&mut reply)).await??;
        Ok(reply[..length].to_vec())
    }
}

impl BrowserListener {
    pub async fn wait_for_close(self) -> Result<()> {
        let BrowserListener { mut server, name } = self;
        loop {
            if let Err(error) = server.connect().await {
                if !client_disconnected(&error) {
                    return Err(error).with_context(|| format!("accept runtime pipe for {name}"));
                }
                server = ServerOptions::new().create(&name)?;
                continue;
            }
            let mut connected = server;
            // Keep an unconnected instance available while this client is
            // being served, so other browser-manager requests can connect.
            server = ServerOptions::new().create(&name)?;
            let mut command = [0u8; 16];
            let length =
                match tokio::time::timeout(Duration::from_secs(2), connected.read(&mut command))
                    .await
                {
                    Ok(Ok(0)) | Err(_) => continue,
                    Ok(Ok(length)) => length,
                    Ok(Err(error)) if client_disconnected(&error) => continue,
                    Ok(Err(error)) => return Err(error).context("read runtime pipe"),
                };
            let closing = &command[..length] == b"CLOSE";
            let reply: &[u8] = match &command[..length] {
                b"PING" => b"PONG",
                b"CLOSE" => b"OK",
                _ => b"ERROR",
            };
            if let Err(error) = connected.write_all(reply).await {
                if client_disconnected(&error) {
                    continue;
                }
                return Err(error).context("write runtime pipe");
            }
            if let Err(error) = connected.flush().await {
                if client_disconnected(&error) {
                    continue;
                }
                return Err(error).context("flush runtime pipe");
            }
            if closing {
                return Ok(());
            }
        }
    }
}

fn client_disconnected(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::BrokenPipe
        || matches!(
            error.raw_os_error(),
            Some(code) if code == ERROR_NO_DATA as i32 || code == ERROR_BROKEN_PIPE as i32
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pipe_tracks_and_closes_one_browser() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = BrowserRuntime::new(dir.path());
        let listener = runtime.bind("alpha").await.unwrap();
        assert!(runtime.bind("alpha").await.is_err());
        let task = tokio::spawn(async move { listener.wait_for_close().await.unwrap() });
        assert!(runtime.is_running("alpha").await);
        assert!(!runtime.is_running("beta").await);
        runtime.close("alpha").await.unwrap();
        task.await.unwrap();
        assert!(!runtime.is_running("alpha").await);
    }

    #[tokio::test]
    async fn disconnected_probe_does_not_close_browser_listener() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = BrowserRuntime::new(dir.path());
        let listener = runtime.bind("alpha").await.unwrap();
        let probe = ClientOptions::new()
            .open(runtime.paths.runtime_pipe("alpha"))
            .unwrap();
        drop(probe);
        let task = tokio::spawn(async move { listener.wait_for_close().await });
        assert!(runtime.is_running("alpha").await);
        runtime.close("alpha").await.unwrap();
        task.await.unwrap().unwrap();
    }
}
