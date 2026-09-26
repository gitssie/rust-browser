use std::net::IpAddr;

use anyhow::{Context, Result, bail};
use camoufox_core::locale::{Geolocation, dominant_locale, normalize_locale};
use serde::{Deserialize, Serialize};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use crate::proxy::ProxySettings;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileGeo {
    pub ip: String,
    pub country_code: String,
    pub country: String,
    pub region: Option<String>,
    pub region_code: Option<String>,
    pub city: Option<String>,
    pub timezone: String,
    pub latitude: f64,
    pub longitude: f64,
    pub locale: String,
    pub source: String,
}

#[derive(Deserialize)]
struct IpWhoResponse {
    success: bool,
    message: Option<String>,
    ip: Option<String>,
    country_code: Option<String>,
    country: Option<String>,
    region: Option<String>,
    region_code: Option<String>,
    city: Option<String>,
    timezone: Option<IpWhoTimezone>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

#[derive(Deserialize)]
struct IpWhoTimezone {
    id: String,
}

impl ProfileGeo {
    pub async fn lookup(proxy: &ProxySettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.request_url())?)
            .timeout(proxy.timeout())
            .build()?;
        let response: IpWhoResponse = client
            .get("https://ipwho.is/")
            .send()
            .await
            .context("ipwho.is could not verify the local proxy exit")?
            .error_for_status()
            .context("ipwho.is returned an HTTP error")?
            .json()
            .await
            .context("invalid ipwho.is response")?;
        if !response.success {
            bail!(
                "ipwho.is lookup failed: {}",
                response.message.unwrap_or_else(|| "unknown error".into())
            );
        }
        let ip = response.ip.context("ipwho.is omitted ip")?;
        ip.parse::<IpAddr>()
            .context("ipwho.is returned an invalid ip")?;
        let country_code = response
            .country_code
            .context("ipwho.is omitted country_code")?
            .to_ascii_uppercase();
        let locale = dominant_locale(&country_code)?.as_string();
        let timezone = response.timezone.context("ipwho.is omitted timezone")?.id;
        if timezone.is_empty() {
            bail!("ipwho.is returned an empty timezone");
        }
        let latitude = response.latitude.context("ipwho.is omitted latitude")?;
        let longitude = response.longitude.context("ipwho.is omitted longitude")?;
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            bail!("ipwho.is returned invalid coordinates");
        }
        Ok(Self {
            ip,
            country_code,
            country: response.country.context("ipwho.is omitted country")?,
            region: response.region,
            region_code: response.region_code,
            city: response.city,
            timezone,
            latitude,
            longitude,
            locale,
            source: "ipwho.is".into(),
        })
    }

    pub fn as_geolocation(&self) -> Result<Geolocation> {
        Ok(Geolocation {
            locale: normalize_locale(&self.locale)?,
            longitude: self.longitude,
            latitude: self.latitude,
            timezone: self.timezone.clone(),
            accuracy: None,
        })
    }

    /// Python's location_changes: IP, city and coordinates may rotate.
    pub fn changes_from(&self, current: &Self) -> Vec<String> {
        let mut changes = Vec::new();
        for (field, saved, observed) in [
            (
                "country_code",
                self.country_code.as_str(),
                current.country_code.as_str(),
            ),
            (
                "timezone",
                self.timezone.as_str(),
                current.timezone.as_str(),
            ),
        ] {
            if normalized(saved) != normalized(observed) {
                changes.push(format!("{field}: {saved:?} -> {observed:?}"));
            }
        }
        let (field, saved, observed) =
            if let Some(code) = self.region_code.as_deref().filter(|s| !s.is_empty()) {
                ("region_code", Some(code), current.region_code.as_deref())
            } else {
                ("region", self.region.as_deref(), current.region.as_deref())
            };
        if let Some(saved) = saved.filter(|s| !s.is_empty())
            && observed.is_none_or(|value| normalized(saved) != normalized(value))
        {
            changes.push(format!("{field}: {saved:?} -> {observed:?}"));
        }
        changes
    }
}

fn normalized(value: &str) -> String {
    value
        .nfkd()
        .filter(|ch| ch.is_alphanumeric() && !is_combining_mark(*ch))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ProfileGeo {
        ProfileGeo {
            ip: "203.0.113.10".into(),
            country_code: "FR".into(),
            country: "France".into(),
            region: Some("Ile-de-France".into()),
            region_code: None,
            city: Some("Paris".into()),
            timezone: "Europe/Paris".into(),
            latitude: 48.8534,
            longitude: 2.3488,
            locale: "fr-FR".into(),
            source: "ipwho.is".into(),
        }
    }

    #[test]
    fn same_region_allows_ip_city_and_coordinates_to_rotate() {
        let saved = sample();
        let current = ProfileGeo {
            ip: "203.0.113.11".into(),
            region: Some("Île de France".into()),
            city: Some("Saint-Denis".into()),
            latitude: 48.9,
            ..saved.clone()
        };
        assert!(saved.changes_from(&current).is_empty());
        assert_eq!(saved.as_geolocation().unwrap().latitude, 48.8534);
    }

    #[test]
    fn rejects_region_country_and_timezone_drift() {
        let saved = sample();
        let current = ProfileGeo {
            country_code: "DE".into(),
            region: Some("Bavaria".into()),
            timezone: "Europe/Berlin".into(),
            ..saved.clone()
        };
        assert_eq!(saved.changes_from(&current).len(), 3);
        let saved = ProfileGeo {
            region_code: Some("IDF".into()),
            ..saved
        };
        let current = ProfileGeo {
            region_code: Some("ARA".into()),
            ..current
        };
        assert!(
            saved
                .changes_from(&current)
                .iter()
                .any(|change| change.contains("region_code"))
        );
    }
}
