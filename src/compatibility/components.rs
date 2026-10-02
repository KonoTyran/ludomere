//! Metadata-only component checks. Upstream Comet manages its own peer payloads.
use anyhow::{Context, Result, ensure};
use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const MAX_METADATA: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct ComponentRelease {
    pub version: String,
}

#[derive(Clone, Debug)]
pub struct ComponentUpdates {
    pub comet: std::result::Result<ComponentRelease, String>,
    pub peers: std::result::Result<ComponentRelease, String>,
}

fn official(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(url.host_str(), Some("cfg.gog.com" | "api.github.com"))
}

fn client() -> Result<Client> {
    Client::builder()
        .user_agent(crate::identity::USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 || !official(attempt.url()) {
                attempt.error("Component metadata redirected outside official publishers")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .context("Could not initialize component version checks")
}

fn read_bounded(mut source: impl Read, limit: u64, cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "Component check cancelled");
        let count = source
            .read(&mut buffer)
            .map_err(|_| anyhow::anyhow!("Could not read component metadata"))?;
        if count == 0 {
            break;
        }
        ensure!(
            (bytes.len() + count) as u64 <= limit,
            "Component metadata exceeded its size limit"
        );
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

fn metadata(client: &Client, url: &str, cancel: &AtomicBool) -> Result<Vec<u8>> {
    ensure!(!cancel.load(Ordering::Relaxed), "Component check cancelled");
    ensure!(
        official(&Url::parse(url)?),
        "Unofficial component metadata source"
    );
    // Never retain reqwest errors or effective URLs.
    let response = client
        .get(url)
        .send()
        .map_err(|_| anyhow::anyhow!("Component version request failed; check your connection"))?;
    ensure!(
        response.status().is_success(),
        "Component publisher returned HTTP {}",
        response.status().as_u16()
    );
    read_bounded(response, MAX_METADATA, cancel)
}

fn peer_release(bytes: &[u8]) -> Result<ComponentRelease> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Manifest {
        application_type: String,
        project_name: String,
        #[serde(rename = "baseURI")]
        base_uri: String,
        version: String,
    }
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    ensure!(
        manifest.application_type == "GogGalaxy"
            && manifest.project_name == "GalaxyPeer"
            && manifest.base_uri
                == "https://content-system.gog.com/open_link/download?path=/open/galaxy/client",
        "Unsupported GOG peer metadata"
    );
    version_parts(&manifest.version)?;
    Ok(ComponentRelease {
        version: manifest.version,
    })
}

fn comet_release(bytes: &[u8]) -> Result<ComponentRelease> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        draft: bool,
        prerelease: bool,
    }
    let release: Release = serde_json::from_slice(bytes)?;
    ensure!(
        !release.draft && !release.prerelease,
        "No stable Comet release is available"
    );
    version_parts(&release.tag_name)?;
    Ok(ComponentRelease {
        version: release.tag_name.trim_start_matches('v').into(),
    })
}

/// Reads official release metadata only; never installs or updates a component.
pub fn check_component_updates(cancel: &AtomicBool) -> ComponentUpdates {
    let client = match client() {
        Ok(client) => client,
        Err(error) => {
            return ComponentUpdates {
                comet: Err(error.to_string()),
                peers: Err(error.to_string()),
            };
        }
    };
    ComponentUpdates {
        comet: metadata(
            &client,
            "https://api.github.com/repos/imLinguin/comet/releases/latest",
            cancel,
        )
        .and_then(|bytes| comet_release(&bytes))
        .map_err(|error| error.to_string()),
        peers: metadata(
            &client,
            "https://cfg.gog.com/desktop-galaxy-peer/7/master/files-windows.json",
            cancel,
        )
        .and_then(|bytes| peer_release(&bytes))
        .map_err(|error| error.to_string()),
    }
}

/// This is Comet's cached version report, not a verification of its installed DLLs.
pub fn installed_peer_version() -> Result<Option<String>> {
    cached_peer_version(
        &super::comet::data_directory().join("redist/.desktop-galaxy-peer-windows.toml"),
    )
}

fn cached_peer_version(path: &Path) -> Result<Option<String>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.is_file(),
        "Comet peer metadata is not a regular file"
    );
    let release: ComponentRelease = toml::from_str(std::str::from_utf8(&read_bounded(
        file,
        16 * 1024,
        &AtomicBool::new(false),
    )?)?)?;
    version_parts(&release.version)?;
    Ok(Some(release.version))
}

fn version_parts(version: &str) -> Result<Vec<u64>> {
    ensure!(version.len() <= 48, "Component version is too long");
    let parts = version
        .strip_prefix('v')
        .unwrap_or(version)
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<Vec<u64>, _>>()?;
    ensure!(
        (2..=6).contains(&parts.len()),
        "Unsupported component version"
    );
    Ok(parts)
}

pub fn newer_version(candidate: &str, installed: &str) -> Result<bool> {
    let mut candidate = version_parts(candidate)?;
    let mut installed = version_parts(installed)?;
    let length = candidate.len().max(installed.len());
    candidate.resize(length, 0);
    installed.resize(length, 0);
    Ok(candidate > installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Cursor};

    #[test]
    fn official_metadata_formats_and_stable_versions() {
        let peer = br#"{"applicationType":"GogGalaxy","baseURI":"https://content-system.gog.com/open_link/download?path=/open/galaxy/client","projectName":"GalaxyPeer","version":"1.2.33.1","files":[]}"#;
        assert_eq!(peer_release(peer).unwrap().version, "1.2.33.1");
        assert!(peer_release(br#"{"applicationType":"GogGalaxy","baseUri":"wrong","projectName":"GalaxyPeer","version":"1.2.33.1"}"#).is_err());
        assert_eq!(
            comet_release(br#"{"tag_name":"v0.3.2","draft":false,"prerelease":false}"#)
                .unwrap()
                .version,
            "0.3.2"
        );
        assert!(
            comet_release(br#"{"tag_name":"v0.4.0","draft":false,"prerelease":true}"#).is_err()
        );
        assert!(
            comet_release(br#"{"tag_name":"v0.4.0-rc1","draft":false,"prerelease":false}"#)
                .is_err()
        );
    }

    #[test]
    fn cached_upstream_version_is_read_only_and_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".desktop-galaxy-peer-windows.toml");
        assert_eq!(cached_peer_version(&path).unwrap(), None);
        fs::write(&path, "time = 145625863\nversion = \"1.2.33.1\"\n").unwrap();
        assert_eq!(
            cached_peer_version(&path).unwrap().as_deref(),
            Some("1.2.33.1")
        );
        fs::write(&path, "version = \"../bad\"").unwrap();
        assert!(cached_peer_version(&path).is_err());
        fs::write(&path, vec![b'a'; 16 * 1024 + 1]).unwrap();
        assert!(cached_peer_version(&path).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("missing", &path).unwrap();
        assert!(cached_peer_version(&path).is_err());
    }

    #[test]
    fn metadata_limits_cancellation_and_official_origins() {
        assert!(read_bounded(Cursor::new(b"oversized"), 3, &AtomicBool::new(false)).is_err());
        assert!(read_bounded(Cursor::new(b"metadata"), 100, &AtomicBool::new(true)).is_err());
        assert!(!official(
            &Url::parse("https://cfg.gog.com.evil.test/file").unwrap()
        ));
        assert!(!official(&Url::parse("http://cfg.gog.com/file").unwrap()));
        assert!(!official(
            &Url::parse("https://user:secret@api.github.com/file").unwrap()
        ));
        assert!(!official(
            &Url::parse("https://content-system.gog.com/payload").unwrap()
        ));
    }

    #[test]
    fn numeric_versions_distinguish_updates_equal_and_older_versions() {
        assert!(newer_version("v0.3.10", "0.3.2").unwrap());
        assert!(!newer_version("0.3.2.0", "v0.3.2").unwrap());
        assert!(!newer_version("1.2.32.9", "1.2.33.1").unwrap());
        assert!(newer_version("0.4-rc1", "0.3.2").is_err());
    }
}
