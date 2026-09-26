use std::fs::{self, File, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use camoufox_core::fingerprint::FingerprintRequest;
use camoufox_core::locale::normalize_locale;
use camoufox_core::os::SupportedOs;
use camoufox_core::persona::PersonaRecord;
use camoufox_store::PersonaStore;
use fs2::FileExt;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use url::Url;

use crate::geo::ProfileGeo;
use crate::paths::AppPaths;
use crate::proxy::ProxySettings;
use crate::proxy_management::ProxyCatalog;
use crate::storage;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("invalid profile: {0}")]
    Invalid(String),
    #[error("profile {0} not found")]
    NotFound(String),
    #[error("profile {0} already exists")]
    AlreadyExists(String),
    #[error("profile {0} is in use")]
    Busy(String),
    #[error("proxy or GeoIP lookup failed: {0}")]
    Proxy(#[source] anyhow::Error),
    #[error("profile storage failed: {0}")]
    Storage(#[source] anyhow::Error),
}

pub type ProfileResult<T> = Result<T, ProfileError>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileOs {
    Macos,
    Windows,
    Linux,
}

impl ProfileOs {
    fn supported(self) -> SupportedOs {
        match self {
            Self::Macos => SupportedOs::Macos,
            Self::Windows => SupportedOs::Windows,
            Self::Linux => SupportedOs::Linux,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "mode", content = "url", rename_all = "lowercase")]
pub enum ProxyChoice {
    Global,
    Custom(String),
}

#[derive(Clone, Debug)]
pub struct CreateProfile {
    pub id: String,
    pub name: Option<String>,
    pub os: ProfileOs,
    pub tabs: Vec<String>,
    pub proxy: ProxyChoice,
    /// None uses the proxy's detected GeoIP. Some overrides editable fields
    /// while keeping the observed proxy IP and enforcing exit-region identity.
    pub geo: Option<ProfileGeoInput>,
}

#[derive(Clone, Debug)]
pub struct ProfileGeoInput {
    pub country_code: String,
    pub country: String,
    pub region: Option<String>,
    pub city: Option<String>,
    pub timezone: String,
    pub latitude: f64,
    pub longitude: f64,
    pub locale: String,
}

impl ProfileGeoInput {
    fn apply_to(self, observed: ProfileGeo) -> ProfileResult<ProfileGeo> {
        let code = self.country_code.trim().to_ascii_uppercase();
        if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ProfileError::Invalid(
                "country code must be two ASCII letters".into(),
            ));
        }
        let country = self.country.trim();
        let timezone = self.timezone.trim();
        let locale = self.locale.trim();
        if country.is_empty()
            || country.chars().count() > 100
            || country.chars().any(char::is_control)
            || timezone.is_empty()
            || timezone.chars().count() > 100
            || timezone.chars().any(char::is_control)
            || locale.chars().any(char::is_control)
            || normalize_locale(locale).is_err()
            || !self.latitude.is_finite()
            || !(-90.0..=90.0).contains(&self.latitude)
            || !self.longitude.is_finite()
            || !(-180.0..=180.0).contains(&self.longitude)
        {
            return Err(ProfileError::Invalid(
                "invalid custom geographic fields".into(),
            ));
        }
        let region = self
            .region
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let city = self
            .city
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if region
            .as_ref()
            .is_some_and(|value| value.chars().count() > 100 || value.chars().any(char::is_control))
            || city.as_ref().is_some_and(|value| {
                value.chars().count() > 100 || value.chars().any(char::is_control)
            })
        {
            return Err(ProfileError::Invalid("region or city is too long".into()));
        }
        if !country.eq_ignore_ascii_case(&observed.country) {
            return Err(ProfileError::Invalid(
                "country name must match the proxy exit".into(),
            ));
        }
        if observed
            .region
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            && region.is_none()
        {
            return Err(ProfileError::Invalid(
                "region is required for this proxy exit".into(),
            ));
        }
        let geo = ProfileGeo {
            ip: observed.ip.clone(),
            country_code: code,
            country: country.to_string(),
            region,
            region_code: None,
            city,
            timezone: timezone.to_string(),
            latitude: self.latitude,
            longitude: self.longitude,
            locale: locale.to_string(),
            source: "custom+ipwho.is".into(),
        };
        let changes = geo.changes_from(&observed);
        if !changes.is_empty() {
            return Err(ProfileError::Invalid(format!(
                "custom location must match the proxy exit country, region and timezone: {}",
                changes.join("; ")
            )));
        }
        Ok(geo)
    }
}

#[derive(Clone, Debug, Default)]
pub struct UpdateProfile {
    pub name: Option<String>,
    pub tabs: Option<Vec<String>>,
    pub proxy: Option<ProxyChoice>,
    /// Replaces the profile's full set of labels when present.
    pub tags: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct ProfileQuery {
    pub search: Option<String>,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyModeFilter {
    Global,
    Custom,
}

#[derive(Clone, Debug, Default)]
pub struct ProfileListFilter {
    pub proxy_mode: Option<ProxyModeFilter>,
    /// Profiles matching any of these tags are returned.
    pub tags: Vec<String>,
}

impl Default for ProfileQuery {
    fn default() -> Self {
        Self {
            search: None,
            page: 1,
            page_size: 20,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfilePage {
    pub items: Vec<ProfileView>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub total_pages: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileView {
    pub id: String,
    pub name: Option<String>,
    pub created_at: u64,
    pub os: String,
    pub tabs: Vec<String>,
    pub tags: Vec<String>,
    pub proxy: ProxyChoice,
    pub effective_proxy: String,
    pub saved_geo: ProfileGeo,
    pub browser_data_dir: PathBuf,
    pub user_agent: String,
}

/// All profile metadata is stored in camoufox-store; browser state stays in its own directory.
#[derive(Clone)]
pub struct ProfileService {
    paths: AppPaths,
    global_proxy: ProxySettings,
}

impl ProfileService {
    pub fn new(data_dir: impl AsRef<Path>, global_proxy: &str) -> ProfileResult<Self> {
        let proxy = ProxySettings::parse(global_proxy)
            .map_err(|error| ProfileError::Invalid(error.to_string()))?;
        Self::with_proxy(data_dir, proxy)
    }

    pub fn with_proxy(data_dir: impl AsRef<Path>, proxy: ProxySettings) -> ProfileResult<Self> {
        let paths =
            AppPaths::new(Some(data_dir.as_ref().to_path_buf())).map_err(ProfileError::Storage)?;
        storage::open_store(paths.root()).map_err(ProfileError::Storage)?;
        let profiles_dir = paths.profiles();
        fs::create_dir_all(&profiles_dir).map_err(|error| ProfileError::Storage(error.into()))?;
        #[cfg(unix)]
        fs::set_permissions(&profiles_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| ProfileError::Storage(error.into()))?;
        Ok(Self {
            paths,
            global_proxy: proxy,
        })
    }

    pub fn data_dir(&self) -> &Path {
        self.paths.root()
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.paths.profiles()
    }

    fn store(&self) -> ProfileResult<PersonaStore> {
        storage::open_store(self.paths.root()).map_err(ProfileError::Storage)
    }

    fn browser_data_dir(&self, id: &str) -> PathBuf {
        self.paths.profile(id)
    }

    fn lock(&self, id: &str) -> ProfileResult<File> {
        fs::create_dir_all(self.paths.locks())
            .map_err(|error| ProfileError::Storage(error.into()))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.paths.profile_lock(id))
            .map_err(|error| ProfileError::Storage(error.into()))?;
        file.try_lock_exclusive()
            .map_err(|_| ProfileError::Busy(id.to_string()))?;
        Ok(file)
    }

    fn tag_lock(&self, id: &str) -> ProfileResult<File> {
        fs::create_dir_all(self.paths.locks())
            .map_err(|error| ProfileError::Storage(error.into()))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.paths.profile_tags_lock(id))
            .map_err(|error| ProfileError::Storage(error.into()))?;
        file.try_lock_exclusive()
            .map_err(|_| ProfileError::Busy(id.to_string()))?;
        Ok(file)
    }

    fn resolve_proxy(&self, choice: &ProxyChoice) -> ProfileResult<ProxySettings> {
        match choice {
            ProxyChoice::Global => Ok(self.global_proxy.clone()),
            ProxyChoice::Custom(url) => ProxyCatalog::new(self.paths.root())
                .resolve_url(url)
                .map_err(|error| ProfileError::Invalid(error.to_string())),
        }
    }

    fn choice(record: &PersonaRecord) -> ProfileResult<ProxyChoice> {
        match record.metadata.get("proxy_mode").and_then(Value::as_str) {
            Some("global") | None => Ok(ProxyChoice::Global),
            Some("custom") => Ok(ProxyChoice::Custom(
                record
                    .metadata
                    .get("proxy_url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ProfileError::Storage(anyhow::anyhow!(
                            "profile {} has no custom proxy",
                            record.id
                        ))
                    })?
                    .to_string(),
            )),
            Some(other) => Err(ProfileError::Storage(anyhow::anyhow!(
                "invalid proxy mode: {other}"
            ))),
        }
    }

    fn view(&self, record: &PersonaRecord) -> ProfileResult<ProfileView> {
        let choice = Self::choice(record)?;
        let proxy = match &choice {
            ProxyChoice::Global => self.global_proxy.clone(),
            ProxyChoice::Custom(url) => ProxySettings::parse(url)
                .map_err(|error| ProfileError::Invalid(error.to_string()))?,
        };
        let geo = record.metadata.get("geo").cloned().ok_or_else(|| {
            ProfileError::Storage(anyhow::anyhow!("profile {} has no saved geo", record.id))
        })?;
        let saved_geo =
            serde_json::from_value(geo).map_err(|error| ProfileError::Storage(error.into()))?;
        let tabs = saved_tabs(record)?;
        let tags = saved_tags(record)?;
        Ok(ProfileView {
            id: record.id.clone(),
            name: record.name.clone(),
            created_at: record.created_at,
            os: record
                .metadata
                .get("os")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            tabs,
            tags,
            proxy: choice,
            effective_proxy: proxy.browser_url(),
            saved_geo,
            browser_data_dir: self.browser_data_dir(&record.id),
            user_agent: record.fingerprint.fingerprint.navigator.user_agent.clone(),
        })
    }

    pub async fn create(&self, input: CreateProfile) -> ProfileResult<ProfileView> {
        validate_id(&input.id)?;
        let name = input.name.as_deref().map(validate_name).transpose()?;
        let tabs = validate_tabs(&input.tabs)?;
        let proxy = self.resolve_proxy(&input.proxy)?;
        if self
            .store()?
            .load(&input.id)
            .await
            .map_err(storage)?
            .is_some()
        {
            return Err(ProfileError::AlreadyExists(input.id));
        }
        proxy.check().await.map_err(ProfileError::Proxy)?;
        let observed = ProfileGeo::lookup(&proxy)
            .await
            .map_err(ProfileError::Proxy)?;
        let geo = match input.geo.clone() {
            Some(custom) => custom.apply_to(observed)?,
            None => observed,
        };
        self.create_with_geo(input, name, tabs, geo).await
    }

    async fn create_with_geo(
        &self,
        input: CreateProfile,
        name: Option<String>,
        tabs: Vec<String>,
        geo: ProfileGeo,
    ) -> ProfileResult<ProfileView> {
        let _lock = self.lock(&input.id)?;
        let store = self.store()?;
        if store.load(&input.id).await.map_err(storage)?.is_some() {
            return Err(ProfileError::AlreadyExists(input.id));
        }
        let request = FingerprintRequest {
            operating_systems: Some(vec![input.os.supported()]),
            seed: Some(rand::thread_rng().r#gen::<u64>()),
            ..Default::default()
        };
        let mut record = PersonaRecord::generate(&input.id, &request).map_err(storage)?;
        record.name = Some(name.unwrap_or_else(|| input.id.clone()));
        record
            .metadata
            .insert("os".into(), json!(input.os.supported().as_str()));
        record.metadata.insert("tabs".into(), json!(tabs));
        record.metadata.insert("geo".into(), json!(geo));
        set_choice(
            &mut record,
            &input.proxy,
            &self.resolve_proxy(&input.proxy)?,
        );
        store.save(&record).await.map_err(storage)?;
        self.view(&record)
    }

    pub async fn get(&self, id: &str) -> ProfileResult<ProfileView> {
        validate_id(id)?;
        let record = self
            .store()?
            .load(id)
            .await
            .map_err(storage)?
            .ok_or_else(|| ProfileError::NotFound(id.to_string()))?;
        self.view(&record)
    }

    pub async fn list(&self, query: ProfileQuery) -> ProfileResult<ProfilePage> {
        self.list_filtered(query, ProfileListFilter::default())
            .await
    }

    pub async fn list_filtered(
        &self,
        query: ProfileQuery,
        filter: ProfileListFilter,
    ) -> ProfileResult<ProfilePage> {
        if query.page == 0 || !(1..=100).contains(&query.page_size) {
            return Err(ProfileError::Invalid(
                "page must be >= 1 and page_size must be 1-100".into(),
            ));
        }
        let offset = (query.page - 1)
            .checked_mul(query.page_size)
            .ok_or_else(|| ProfileError::Invalid("page is too large".into()))?;
        let search = query.search.unwrap_or_default().trim().to_lowercase();
        let filter_tags = validate_tags(&filter.tags)?;
        if search.chars().count() > 100 {
            return Err(ProfileError::Invalid("search is too long".into()));
        }
        let store = self.store()?;
        let mut summaries = store.list().await.map_err(storage)?;
        summaries.retain(|item| {
            search.is_empty()
                || item.id.to_lowercase().contains(&search)
                || item
                    .name
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&search)
        });
        if !filter_tags.is_empty() || filter.proxy_mode.is_some() {
            let mut filtered = Vec::new();
            for summary in summaries {
                let Some(record) = store.load(&summary.id).await.map_err(storage)? else {
                    continue;
                };
                let proxy_matches = match filter.proxy_mode {
                    None => true,
                    Some(ProxyModeFilter::Global) => {
                        matches!(Self::choice(&record)?, ProxyChoice::Global)
                    }
                    Some(ProxyModeFilter::Custom) => {
                        matches!(Self::choice(&record)?, ProxyChoice::Custom(_))
                    }
                };
                let tags = saved_tags(&record)?;
                let tag_matches = filter_tags.is_empty()
                    || filter_tags
                        .iter()
                        .any(|selected| tags.iter().any(|tag| tag.eq_ignore_ascii_case(selected)));
                if proxy_matches && tag_matches {
                    filtered.push(summary);
                }
            }
            summaries = filtered;
        }
        summaries.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let total = summaries.len();
        let mut items = Vec::new();
        for item in summaries.iter().skip(offset).take(query.page_size) {
            if let Some(record) = store.load(&item.id).await.map_err(storage)? {
                items.push(self.view(&record)?);
            }
        }
        Ok(ProfilePage {
            items,
            page: query.page,
            page_size: query.page_size,
            total,
            total_pages: total.div_ceil(query.page_size),
        })
    }

    pub async fn list_ids(&self) -> ProfileResult<Vec<String>> {
        Ok(self
            .store()?
            .list()
            .await
            .map_err(storage)?
            .into_iter()
            .map(|summary| summary.id)
            .collect())
    }

    /// Returns all labels currently assigned to any saved profile.
    pub async fn list_tags(&self) -> ProfileResult<Vec<String>> {
        let store = self.store()?;
        let mut tags = Vec::new();
        for summary in store.list().await.map_err(storage)? {
            if let Some(record) = store.load(&summary.id).await.map_err(storage)? {
                for tag in saved_tags(&record)? {
                    if !tags
                        .iter()
                        .any(|existing: &String| existing.eq_ignore_ascii_case(&tag))
                    {
                        tags.push(tag);
                    }
                }
            }
        }
        tags.sort_by_key(|tag| tag.to_lowercase());
        Ok(tags)
    }

    pub async fn update(&self, id: &str, input: UpdateProfile) -> ProfileResult<ProfileView> {
        validate_id(id)?;
        if input.name.is_none()
            && input.tabs.is_none()
            && input.proxy.is_none()
            && input.tags.is_none()
        {
            return Err(ProfileError::Invalid("update has no changes".into()));
        }
        let name = input.name.as_deref().map(validate_name).transpose()?;
        let tabs = input
            .tabs
            .as_ref()
            .map(|tabs| validate_tabs(tabs))
            .transpose()?;
        let proxy = input
            .proxy
            .as_ref()
            .map(|choice| self.resolve_proxy(choice))
            .transpose()?;
        let tags = input
            .tags
            .as_ref()
            .map(|tags| validate_tags(tags))
            .transpose()?;
        let tags_only = input.tags.is_some()
            && input.name.is_none()
            && input.tabs.is_none()
            && input.proxy.is_none();
        // The browser holds its main lock for its lifetime. A tag-only edit is
        // safe once its control socket is live: the startup persona save has
        // already finished, and other tag edits use this separate lock.
        let _tag_lock = if tags_only {
            Some(self.tag_lock(id)?)
        } else {
            None
        };
        let _lock = match self.lock(id) {
            Ok(lock) => Some(lock),
            Err(ProfileError::Busy(_)) if tags_only => {
                #[cfg(unix)]
                {
                    if !crate::runtime::BrowserRuntime::new(self.paths.root())
                        .is_running(id)
                        .await
                    {
                        return Err(ProfileError::Busy(id.to_string()));
                    }
                    None
                }
                #[cfg(not(unix))]
                {
                    return Err(ProfileError::Busy(id.to_string()));
                }
            }
            Err(error) => return Err(error),
        };
        let store = self.store()?;
        let mut record = store
            .load(id)
            .await
            .map_err(storage)?
            .ok_or_else(|| ProfileError::NotFound(id.to_string()))?;
        if let Some(name) = name {
            record.name = Some(name);
        }
        if let Some(tabs) = tabs {
            record.metadata.insert("tabs".into(), json!(tabs));
        }
        if let Some(tags) = tags {
            record.metadata.insert("tags".into(), json!(tags));
        }
        if let (Some(choice), Some(proxy)) = (input.proxy.as_ref(), proxy.as_ref()) {
            set_choice(&mut record, choice, proxy);
        }
        store.save(&record).await.map_err(storage)?;
        self.view(&record)
    }

    pub async fn delete(&self, id: &str) -> ProfileResult<()> {
        validate_id(id)?;
        let _lock = self.lock(id)?;
        let store = self.store()?;
        if store.load(id).await.map_err(storage)?.is_none() {
            return Err(ProfileError::NotFound(id.to_string()));
        }
        let browser_data = self.browser_data_dir(id);
        let quarantined = if browser_data.exists() {
            let trash = self.paths.trash();
            fs::create_dir_all(&trash).map_err(|error| ProfileError::Storage(error.into()))?;
            let destination = self
                .paths
                .trashed_profile(id, rand::thread_rng().r#gen::<u64>());
            fs::rename(&browser_data, &destination)
                .map_err(|error| ProfileError::Storage(error.into()))?;
            Some(destination)
        } else {
            None
        };
        if let Err(error) = store.delete(id).await {
            if let Some(destination) = quarantined.as_ref() {
                fs::rename(destination, &browser_data)
                    .map_err(|restore| ProfileError::Storage(restore.into()))?;
            }
            return Err(storage(error));
        }
        if let Some(destination) = quarantined {
            fs::remove_dir_all(destination).map_err(|error| ProfileError::Storage(error.into()))?;
        }
        for artifact in self.paths.scan_artifacts(id).all() {
            match fs::remove_file(artifact) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ProfileError::Storage(error.into())),
            }
        }
        Ok(())
    }
}

fn set_choice(record: &mut PersonaRecord, choice: &ProxyChoice, proxy: &ProxySettings) {
    match choice {
        ProxyChoice::Global => {
            record.metadata.insert("proxy_mode".into(), json!("global"));
            record.metadata.remove("proxy_url");
        }
        ProxyChoice::Custom(_) => {
            record.metadata.insert("proxy_mode".into(), json!("custom"));
            record
                .metadata
                .insert("proxy_url".into(), json!(proxy.browser_url()));
        }
    }
}

fn validate_id(id: &str) -> ProfileResult<()> {
    if id.is_empty()
        || id.len() > 64
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ProfileError::Invalid(
            "id must be 1-64 ASCII letters, digits, _ or -, starting with a letter or digit".into(),
        ));
    }
    Ok(())
}

fn validate_name(name: &str) -> ProfileResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(ProfileError::Invalid(
            "name must contain 1-100 printable characters".into(),
        ));
    }
    Ok(name.to_string())
}

fn validate_url(input: &str) -> ProfileResult<String> {
    let parsed = Url::parse(input)
        .map_err(|_| ProfileError::Invalid("URL must be absolute http(s)".into()))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(ProfileError::Invalid(
            "URL must be absolute http(s) without credentials".into(),
        ));
    }
    Ok(parsed.to_string())
}

fn validate_tabs(tabs: &[String]) -> ProfileResult<Vec<String>> {
    let mut clean = Vec::new();
    for tab in tabs {
        let tab = validate_url(tab)?;
        if !clean.contains(&tab) {
            clean.push(tab);
        }
    }
    Ok(clean)
}

fn saved_tabs(record: &PersonaRecord) -> ProfileResult<Vec<String>> {
    if !record.metadata.contains_key("tabs") {
        return match record.metadata.get("url").and_then(Value::as_str) {
            Some(url) => Ok(vec![validate_url(url)?]),
            None => Ok(Vec::new()),
        };
    }
    let tabs: Vec<String> = serde_json::from_value(
        record
            .metadata
            .get("tabs")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .map_err(|error| ProfileError::Storage(error.into()))?;
    validate_tabs(&tabs)
}

fn saved_tags(record: &PersonaRecord) -> ProfileResult<Vec<String>> {
    let tags: Vec<String> = serde_json::from_value(
        record
            .metadata
            .get("tags")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .map_err(|error| ProfileError::Storage(error.into()))?;
    validate_tags(&tags)
}

fn validate_tags(tags: &[String]) -> ProfileResult<Vec<String>> {
    if tags.len() > 20 {
        return Err(ProfileError::Invalid(
            "a profile may have at most 20 tags".into(),
        ));
    }
    let mut clean: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() || tag.chars().count() > 24 || tag.chars().any(char::is_control) {
            return Err(ProfileError::Invalid(
                "tag must contain 1-24 printable characters".into(),
            ));
        }
        if !clean
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tag))
        {
            clean.push(tag.to_string());
        }
    }
    Ok(clean)
}

fn storage(error: impl Into<anyhow::Error>) -> ProfileError {
    ProfileError::Storage(error.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy_management::{ManagedProxy, ProxyCredentials, ProxyPolicy};

    fn geo() -> ProfileGeo {
        ProfileGeo {
            ip: "203.0.113.10".into(),
            country_code: "FR".into(),
            country: "France".into(),
            region: Some("Île-de-France".into()),
            region_code: Some("IDF".into()),
            city: Some("Paris".into()),
            timezone: "Europe/Paris".into(),
            latitude: 48.85,
            longitude: 2.35,
            locale: "fr-FR".into(),
            source: "ipwho.is".into(),
        }
    }

    fn input(id: &str, name: &str, proxy: ProxyChoice) -> CreateProfile {
        CreateProfile {
            id: id.into(),
            name: Some(name.into()),
            os: ProfileOs::Windows,
            tabs: vec!["https://www.vinted.fr/".into()],
            proxy,
            geo: None,
        }
    }

    #[test]
    fn empty_profile_directory_is_ready_for_settings_to_open() {
        let root = tempfile::tempdir().unwrap();
        let service = ProfileService::new(root.path(), "socks5://127.0.0.1:12334").unwrap();
        assert!(service.data_dir().join("profiles").is_dir());
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(service.profiles_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(service.data_dir().join("browser.sqlite").is_file());
        assert_eq!(
            service.browser_data_dir("alpha"),
            service.data_dir().join("profiles/alpha")
        );
    }

    #[test]
    fn managed_auth_applies_to_custom_choice_only() {
        let dir = tempfile::tempdir().unwrap();
        let service = ProfileService::new(dir.path(), "socks5://proxy.example:1080").unwrap();
        ProxyCatalog::new(dir.path())
            .upsert(ManagedProxy {
                id: "managed".into(),
                name: "Managed".into(),
                url: "socks5://proxy.example:1080".into(),
                credentials: Some(ProxyCredentials {
                    username: "alice".into(),
                    password: "secret".into(),
                }),
                policy: ProxyPolicy::ClosePrevious,
                ip_switch: None,
            })
            .unwrap();
        let custom = service
            .resolve_proxy(&ProxyChoice::Custom("socks5://proxy.example:1080".into()))
            .unwrap();
        assert_eq!(custom.credentials(), Some(("alice", "secret")));
        let global = service.resolve_proxy(&ProxyChoice::Global).unwrap();
        assert_eq!(global.credentials(), None);
    }

    #[test]
    fn custom_geo_keeps_proxy_ip_and_rejects_region_drift() {
        let observed = geo();
        let custom = ProfileGeoInput {
            country_code: "FR".into(),
            country: "France".into(),
            region: Some("Île-de-France".into()),
            city: Some("Saint-Denis".into()),
            timezone: "Europe/Paris".into(),
            latitude: 48.94,
            longitude: 2.36,
            locale: "fr-FR".into(),
        };
        let saved = custom.clone().apply_to(observed.clone()).unwrap();
        assert_eq!(saved.ip, observed.ip);
        assert_eq!(saved.city.as_deref(), Some("Saint-Denis"));
        assert_eq!(saved.source, "custom+ipwho.is");
        assert!(matches!(
            ProfileGeoInput {
                region: Some("Bavaria".into()),
                ..custom.clone()
            }
            .apply_to(observed.clone()),
            Err(ProfileError::Invalid(_))
        ));
        assert!(matches!(
            ProfileGeoInput {
                region: None,
                ..custom.clone()
            }
            .apply_to(observed.clone()),
            Err(ProfileError::Invalid(_))
        ));
        assert!(matches!(
            ProfileGeoInput {
                country: "Germany".into(),
                ..custom
            }
            .apply_to(observed),
            Err(ProfileError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn crud_search_paging_and_proxy_modes_preserve_identity() {
        let dir = tempfile::tempdir().unwrap();
        let service = ProfileService::new(dir.path(), "socks5://localhost:23456").unwrap();
        for (id, name, proxy) in [
            ("alpha", "Vinted France", ProxyChoice::Global),
            (
                "beta",
                "Second",
                ProxyChoice::Custom("socks5://localhost:34567".into()),
            ),
            ("gamma", "Vinted spare", ProxyChoice::Global),
        ] {
            let create = input(id, name, proxy);
            service
                .create_with_geo(
                    create,
                    Some(name.into()),
                    vec!["https://www.vinted.fr/".into()],
                    geo(),
                )
                .await
                .unwrap();
        }
        let first = service.get("alpha").await.unwrap();
        assert_eq!(first.effective_proxy, "socks5://localhost:23456");
        let beta = service.get("beta").await.unwrap();
        assert_eq!(beta.effective_proxy, "socks5://localhost:34567");
        let original = service
            .store()
            .unwrap()
            .load("alpha")
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            service
                .create_with_geo(
                    input("alpha", "Duplicate", ProxyChoice::Global),
                    None,
                    vec![],
                    geo()
                )
                .await,
            Err(ProfileError::AlreadyExists(_))
        ));

        let page = service
            .list(ProfileQuery {
                search: Some("vinted".into()),
                page: 1,
                page_size: 1,
            })
            .await
            .unwrap();
        assert_eq!((page.total, page.total_pages), (2, 2));
        let next = service
            .list(ProfileQuery {
                search: Some("VINTED".into()),
                page: 2,
                page_size: 1,
            })
            .await
            .unwrap();
        let mut ids = [page.items[0].id.as_str(), next.items[0].id.as_str()];
        ids.sort();
        assert_eq!(ids, ["alpha", "gamma"]);
        let empty = service
            .list(ProfileQuery {
                search: None,
                page: 8,
                page_size: 2,
            })
            .await
            .unwrap();
        assert!(empty.items.is_empty());

        let updated = service
            .update(
                "alpha",
                UpdateProfile {
                    name: Some("Main France".into()),
                    tabs: Some(vec!["https://example.com".into()]),
                    proxy: Some(ProxyChoice::Custom("socks5://localhost:45678".into())),
                    tags: Some(vec!["法国".into(), "主账号".into(), "法国".into()]),
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name.as_deref(), Some("Main France"));
        assert_eq!(updated.tabs, ["https://example.com/"]);
        assert_eq!(updated.effective_proxy, "socks5://localhost:45678");
        assert_eq!(updated.tags, ["法国", "主账号"]);
        assert_eq!(service.list_tags().await.unwrap(), ["主账号", "法国"]);
        let after = service
            .store()
            .unwrap()
            .load("alpha")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original.seed, after.seed);
        assert_eq!(
            original.fingerprint.fingerprint.navigator.user_agent,
            after.fingerprint.fingerprint.navigator.user_agent
        );
        assert_eq!(original.metadata.get("geo"), after.metadata.get("geo"));
        let reset = service
            .update(
                "alpha",
                UpdateProfile {
                    proxy: Some(ProxyChoice::Global),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(reset.effective_proxy, "socks5://localhost:23456");

        let browser_data = service.browser_data_dir("alpha");
        fs::create_dir_all(&browser_data).unwrap();
        fs::write(browser_data.join("cookies.sqlite"), b"saved").unwrap();
        service.delete("alpha").await.unwrap();
        assert!(!browser_data.exists());
        assert!(matches!(
            service.get("alpha").await,
            Err(ProfileError::NotFound(_))
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn labels_can_change_while_browser_is_running() {
        let dir = tempfile::tempdir().unwrap();
        let service = ProfileService::new(dir.path(), "socks5://127.0.0.1:12334").unwrap();
        service
            .create_with_geo(
                input("live", "Live", ProxyChoice::Global),
                Some("Live".into()),
                vec![],
                geo(),
            )
            .await
            .unwrap();
        let _browser_lock = service.lock("live").unwrap();
        let runtime = crate::runtime::BrowserRuntime::new(service.data_dir());
        let listener = runtime.bind("live").await.unwrap();
        let task = tokio::spawn(async move { listener.wait_for_close().await.unwrap() });
        let changed = service
            .update(
                "live",
                UpdateProfile {
                    tags: Some(vec!["active".into()]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(changed.tags, ["active"]);
        assert!(matches!(
            service
                .update(
                    "live",
                    UpdateProfile {
                        name: Some("Changed".into()),
                        ..Default::default()
                    }
                )
                .await,
            Err(ProfileError::Busy(_))
        ));
        runtime.close("live").await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn invalid_create_and_update_do_not_access_proxy_or_modify_profile() {
        let dir = tempfile::tempdir().unwrap();
        let service = ProfileService::new(dir.path(), "socks5://127.0.0.1:1").unwrap();
        assert!(matches!(
            service
                .create(input("../bad", "Bad", ProxyChoice::Global))
                .await,
            Err(ProfileError::Invalid(_))
        ));
        assert!(matches!(
            service
                .create(input(
                    "valid",
                    "Bad",
                    ProxyChoice::Custom("http://localhost:1234".into())
                ))
                .await,
            Err(ProfileError::Invalid(_))
        ));
        assert!(matches!(
            service
                .list(ProfileQuery {
                    page: 0,
                    ..Default::default()
                })
                .await,
            Err(ProfileError::Invalid(_))
        ));
        assert!(matches!(
            service
                .update(
                    "missing",
                    UpdateProfile {
                        name: Some("New".into()),
                        ..Default::default()
                    }
                )
                .await,
            Err(ProfileError::NotFound(_))
        ));
    }
}
