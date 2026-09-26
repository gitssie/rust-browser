use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use url::Url;

pub const DEFAULT_PROXY: &str = "socks5://127.0.0.1:12334";

#[derive(Clone)]
pub struct ProxySettings {
    host: String,
    port: u16,
    username: String,
    password: String,
    remote_dns: bool,
    timeout_seconds: u64,
}

impl ProxySettings {
    pub fn parse(raw: &str) -> Result<Self> {
        let normalized = if let Some(rest) = raw.strip_prefix("socks://") {
            format!("socks5://{rest}")
        } else {
            raw.to_string()
        };
        let url = Url::parse(&normalized).context("proxy must be a socks5:// URL")?;
        if !matches!(url.scheme(), "socks5" | "socks5h")
            || url.host_str().is_none()
            || url.port().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("proxy must be socks5://HOST:PORT without credentials or a path");
        }
        Self::configured(
            url.host_str().unwrap(),
            url.port().unwrap(),
            "",
            "",
            true,
            10,
        )
    }

    pub fn configured(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        remote_dns: bool,
        timeout_seconds: u64,
    ) -> Result<Self> {
        let host = host.trim();
        let url = Url::parse(&format!("socks5://{host}:{port}"))
            .context("invalid SOCKS5 server or port")?;
        if host.is_empty()
            || port == 0
            || url.host_str().is_none()
            || url.port() != Some(port)
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("invalid SOCKS5 server or port");
        }
        if (username.is_empty() != password.is_empty())
            || username.len() > 255
            || password.len() > 255
            || username.chars().any(char::is_control)
            || password.chars().any(char::is_control)
        {
            bail!("proxy authentication requires both username and password (up to 255 bytes)");
        }
        if !(1..=60).contains(&timeout_seconds) {
            bail!("proxy connection timeout must be 1-60 seconds");
        }
        Ok(Self {
            host: url.host_str().unwrap().to_string(),
            port,
            username: username.to_string(),
            password: password.to_string(),
            remote_dns,
            timeout_seconds,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn credentials(&self) -> Option<(&str, &str)> {
        (!self.username.is_empty()).then_some((&self.username, &self.password))
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds)
    }

    fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{host}:{}", self.port)
    }

    pub fn browser_url(&self) -> String {
        format!("socks5://{}", self.authority())
    }

    pub fn request_url(&self) -> String {
        let scheme = if self.remote_dns { "socks5h" } else { "socks5" };
        let mut url = Url::parse(&format!("{scheme}://{}", self.authority()))
            .expect("validated proxy address");
        if let Some((username, password)) = self.credentials() {
            url.set_username(username).expect("valid proxy username");
            url.set_password(Some(password))
                .expect("valid proxy password");
        }
        url.to_string()
    }

    pub async fn check(&self) -> Result<()> {
        tokio::time::timeout(
            Duration::from_secs(self.timeout_seconds),
            TcpStream::connect((self.host.as_str(), self.port)),
        )
        .await
        .with_context(|| format!("timed out connecting to proxy {}", self.browser_url()))?
        .with_context(|| format!("proxy {} is unavailable", self.browser_url()))?;
        Ok(())
    }

    pub fn apply_firefox_prefs(&self, prefs: &mut BTreeMap<String, Value>) {
        prefs.insert("network.proxy.type".into(), json!(1));
        prefs.insert("network.proxy.socks".into(), json!(self.host));
        prefs.insert("network.proxy.socks_port".into(), json!(self.port));
        prefs.insert("network.proxy.socks_version".into(), json!(5));
        prefs.insert(
            "network.proxy.socks_remote_dns".into(),
            json!(self.remote_dns),
        );
        prefs.insert("network.proxy.no_proxies_on".into(), json!(""));
        prefs.insert(
            "network.proxy.allow_hijacking_localhost".into(),
            json!(true),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_proxy_and_remote_dns() {
        let proxy = ProxySettings::parse("socks://127.0.0.1:12334").unwrap();
        assert_eq!(proxy.browser_url(), "socks5://127.0.0.1:12334");
        assert_eq!(proxy.request_url(), "socks5h://127.0.0.1:12334");
        assert!(ProxySettings::parse("socks5://user:pass@127.0.0.1:12334").is_err());
    }

    #[test]
    fn credentials_are_only_in_request_url_and_dns_can_be_local() {
        let proxy =
            ProxySettings::configured("proxy.example", 1080, "user@x", "p:a ss", false, 7).unwrap();
        assert_eq!(proxy.browser_url(), "socks5://proxy.example:1080");
        assert_eq!(
            proxy.request_url(),
            "socks5://user%40x:p%3Aa%20ss@proxy.example:1080"
        );
        assert_eq!(proxy.timeout(), Duration::from_secs(7));
        let mut prefs = BTreeMap::new();
        proxy.apply_firefox_prefs(&mut prefs);
        assert_eq!(prefs["network.proxy.socks_remote_dns"], json!(false));
    }
}
