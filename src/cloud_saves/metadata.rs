use crate::domain::GalaxyBuild;
use anyhow::{Result, anyhow, bail};
use flate2::read::ZlibDecoder;
use serde::Deserialize;
use std::io::Read;

const GALAXY_VERSION: &str = "2.0.80";
const MAX_COMPRESSED_REPOSITORY: usize = 16 * 1024 * 1024;
const MAX_EXPANDED_REPOSITORY: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct GameCredentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug)]
pub struct MissingGameCredentials;

impl std::fmt::Display for MissingGameCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("game credentials are absent")
    }
}

impl std::error::Error for MissingGameCredentials {}

impl std::fmt::Debug for GameCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameCredentials")
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConfiguration {
    pub content: RemoteContent,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoteContent {
    pub windows: Option<PlatformConfiguration>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformConfiguration {
    pub cloud_storage: Option<CloudStorageConfiguration>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloudStorageConfiguration {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub locations: Vec<RemoteLocation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteLocation {
    #[serde(default)]
    pub name: Option<String>,
    pub location: String,
}

pub fn select_build<'a>(
    builds: &'a [GalaxyBuild],
    installed: Option<&str>,
) -> Option<&'a GalaxyBuild> {
    builds
        .iter()
        .filter(|build| build.generation == 2)
        .find(|build| installed.is_some() && build.version.as_deref() == installed)
        .or_else(|| {
            builds
                .iter()
                .filter(|build| build.generation == 2 && build.public)
                .max_by_key(|build| build.published_at)
        })
}

pub fn fetch_credentials(client: &reqwest::blocking::Client, url: &str) -> Result<GameCredentials> {
    let mut response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|_| anyhow!("repository download failed"))?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_COMPRESSED_REPOSITORY as u64)
    {
        bail!("compressed repository exceeds the safety limit");
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_COMPRESSED_REPOSITORY as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("repository download failed"))?;
    decode_credentials(&bytes)
}

fn decode_credentials(bytes: &[u8]) -> Result<GameCredentials> {
    decode_credentials_with_limits(bytes, MAX_COMPRESSED_REPOSITORY, MAX_EXPANDED_REPOSITORY)
}

fn decode_credentials_with_limits(
    bytes: &[u8],
    max_compressed: usize,
    max_expanded: u64,
) -> Result<GameCredentials> {
    if bytes.len() > max_compressed {
        bail!("compressed repository exceeds the safety limit");
    }
    let mut expanded = Vec::new();
    ZlibDecoder::new(bytes)
        .take(max_expanded + 1)
        .read_to_end(&mut expanded)
        .map_err(|_| anyhow!("repository compression is invalid"))?;
    if expanded.len() as u64 > max_expanded {
        bail!("expanded repository exceeds the safety limit");
    }
    let value: serde_json::Value =
        serde_json::from_slice(&expanded).map_err(|_| anyhow!("repository JSON is malformed"))?;
    credentials_in(&value).ok_or_else(|| MissingGameCredentials.into())
}

fn credentials_in(value: &serde_json::Value) -> Option<GameCredentials> {
    if let Some(object) = value.as_object() {
        let client_id = object.get("clientId").and_then(|v| v.as_str());
        let client_secret = object.get("clientSecret").and_then(|v| v.as_str());
        if let (Some(client_id), Some(client_secret)) = (client_id, client_secret) {
            return Some(GameCredentials {
                client_id: client_id.into(),
                client_secret: client_secret.into(),
            });
        }
        object.values().find_map(credentials_in)
    } else if let Some(array) = value.as_array() {
        array.iter().find_map(credentials_in)
    } else {
        None
    }
}

pub fn fetch_remote_configuration(
    client: &reqwest::blocking::Client,
    client_id: &str,
) -> Result<RemoteConfiguration> {
    client
        .get(format!(
            "https://remote-config.gog.com/components/galaxy_client/clients/{client_id}"
        ))
        .query(&[("component_version", GALAXY_VERSION)])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|_| anyhow!("remote cloud configuration request failed"))?
        .json()
        .map_err(|_| anyhow!("remote cloud configuration is malformed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_configuration() {
        let config: RemoteConfiguration = serde_json::from_str(r#"{"content":{"Windows":{"cloudStorage":{"enabled":true,"locations":[{"name":"main","location":"<?DOCUMENTS?>/Game"}]}}}}"#).unwrap();
        let storage = config.content.windows.unwrap().cloud_storage.unwrap();
        assert!(storage.enabled);
        assert_eq!(storage.locations.len(), 1);
    }

    #[test]
    fn decodes_zlib_repository_credentials() {
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(br#"{"nested":{"clientId":"id","clientSecret":"top-secret-value"}}"#)
            .unwrap();
        let credentials = decode_credentials(&encoder.finish().unwrap()).unwrap();
        assert_eq!(credentials.client_id, "id");
        assert_eq!(credentials.client_secret, "top-secret-value");
        assert!(!format!("{credentials:?}").contains("top-secret-value"));
    }

    #[test]
    fn rejects_invalid_or_unusable_repositories_without_leaking_content() {
        assert_eq!(
            decode_credentials(b"secret").unwrap_err().to_string(),
            "repository compression is invalid"
        );
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(br#"{"private":"secret"}"#).unwrap();
        let error = decode_credentials(&encoder.finish().unwrap())
            .unwrap_err()
            .to_string();
        assert_eq!(error, "game credentials are absent");
        assert!(!error.contains("secret"));
    }

    #[test]
    fn enforces_compressed_and_expanded_limits() {
        assert_eq!(
            decode_credentials_with_limits(b"123", 2, 100)
                .unwrap_err()
                .to_string(),
            "compressed repository exceeds the safety limit"
        );
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&[b'x'; 128]).unwrap();
        assert_eq!(
            decode_credentials_with_limits(&encoder.finish().unwrap(), 1024, 32)
                .unwrap_err()
                .to_string(),
            "expanded repository exceeds the safety limit"
        );
    }

    #[test]
    fn parses_absent_windows_cloud_configuration() {
        let config: RemoteConfiguration =
            serde_json::from_str(r#"{"content":{"Windows":{}}}"#).unwrap();
        assert!(config.content.windows.unwrap().cloud_storage.is_none());
        let config: RemoteConfiguration = serde_json::from_str(r#"{"content":{}}"#).unwrap();
        assert!(config.content.windows.is_none());
    }
}
