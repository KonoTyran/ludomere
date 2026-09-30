//! Confirmed official Comet updates, isolated from package-owned helpers.
use super::Build;
use anyhow::{Context, Result, ensure};
use reqwest::{Url, blocking::Client};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const RELEASE: &str = "https://api.github.com/repos/imLinguin/comet/releases/latest";
const ASSETS: &str = "https://github.com/imLinguin/comet/releases/download/";
const MAX_BINARY: u64 = 64 * 1024 * 1024;
const BINARIES: &[(&str, &str)] = &[
    ("comet-x86_64-unknown-linux-gnu", "comet"),
    ("GalaxyCommunication-dummy.exe", "GalaxyCommunication.exe"),
];

#[derive(Clone, Debug)]
pub struct CometUpdate {
    pub version: String,
    pub download_bytes: u64,
    assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

pub(super) fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(super) fn validate_version(value: &str) -> Result<()> {
    ensure!(
        value.len() <= 32
            && value.split('.').count() == 3
            && value.split('.').all(|v| !v.is_empty()
                && v.bytes().all(|c| c.is_ascii_digit())
                && v.parse::<u32>().is_ok()),
        "Invalid Comet version"
    );
    Ok(())
}

pub(super) fn local_bytes(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(file.metadata()?.is_file(), "Comet file must be regular");
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= maximum,
        "Comet file exceeds size limit"
    );
    Ok(bytes)
}

fn storage() -> PathBuf {
    crate::identity::data_root().join("comet/helpers")
}

pub(super) fn active_directory() -> Result<Option<PathBuf>> {
    active_at(&storage())
}

fn active_at(root: &Path) -> Result<Option<PathBuf>> {
    let pointer = root.join("current.json");
    if !pointer.try_exists()? {
        return Ok(None);
    }
    ensure!(
        fs::symlink_metadata(root)?.is_dir(),
        "Comet storage must be a real directory"
    );
    let directory: String = serde_json::from_slice(&local_bytes(&pointer, 256)?)?;
    ensure!(
        !directory.is_empty()
            && !directory.starts_with('.')
            && directory.len() <= 128
            && directory
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || b".-".contains(&c)),
        "Invalid active Comet directory"
    );
    let path = root.join(directory);
    super::verify_build(&path)?;
    Ok(Some(path))
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Relaxed) && !crate::profile_reset::stopping_operations(),
        "Comet update cancelled"
    );
    Ok(())
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(crate::identity::USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5
                || url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port_or_known_default() != Some(443)
                || !matches!(
                    url.host_str(),
                    Some(
                        "api.github.com"
                            | "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.error("Comet update redirected outside its publisher")
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}

fn fetch(
    url: &str,
    limit: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<Vec<u8>> {
    cancelled(cancel)?;
    let response = client()?
        .get(url)
        .send()
        .map_err(|_| anyhow::anyhow!("Comet update request failed; check your connection"))?;
    ensure!(
        response.status().is_success(),
        "Comet publisher returned HTTP {}",
        response.status().as_u16()
    );
    let mut response = response;
    let mut bytes = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        cancelled(cancel)?;
        let count = response
            .read(&mut buffer)
            .map_err(|_| anyhow::anyhow!("Comet update transfer interrupted"))?;
        if count == 0 {
            break;
        }
        ensure!(
            (bytes.len() + count) as u64 <= limit,
            "Comet update exceeds size limit"
        );
        bytes.extend_from_slice(&buffer[..count]);
        progress(bytes.len() as u64);
    }
    Ok(bytes)
}

fn candidate_from_bytes(bytes: &[u8]) -> Result<CometUpdate> {
    ensure!(
        bytes.len() <= 1024 * 1024,
        "Comet release metadata exceeds size limit"
    );
    let release: Release = serde_json::from_slice(bytes)?;
    ensure!(
        !release.draft && !release.prerelease,
        "Comet release is not stable"
    );
    let version = release
        .tag_name
        .strip_prefix('v')
        .context("Unsupported Comet release tag")?
        .to_owned();
    validate_version(&version)?;
    let candidate = CometUpdate {
        version,
        download_bytes: 0,
        assets: release
            .assets
            .into_iter()
            .filter(|asset| BINARIES.iter().any(|(name, _)| *name == asset.name))
            .collect(),
    };
    validate_candidate(&candidate)?;
    Ok(CometUpdate {
        download_bytes: candidate.assets.iter().map(|asset| asset.size).sum(),
        ..candidate
    })
}

fn validate_candidate(candidate: &CometUpdate) -> Result<()> {
    validate_version(&candidate.version)?;
    ensure!(
        candidate.assets.len() == BINARIES.len(),
        "Comet release has no complete Linux helper pair"
    );
    for (name, _) in BINARIES {
        let matches: Vec<_> = candidate
            .assets
            .iter()
            .filter(|asset| asset.name == *name)
            .collect();
        ensure!(matches.len() == 1, "Duplicate or missing Comet asset");
        let asset = matches[0];
        ensure!(
            asset.size > 0
                && asset.size <= MAX_BINARY
                && asset
                    .digest
                    .as_deref()
                    .and_then(|digest| digest.strip_prefix("sha256:"))
                    .is_some_and(is_digest),
            "Comet release lacks a supported publisher SHA-256 digest"
        );
        let expected = format!("{ASSETS}v{}/{}", candidate.version, name);
        ensure!(
            asset.browser_download_url == expected && Url::parse(&expected)?.query().is_none(),
            "Comet asset is not from the official release"
        );
    }
    Ok(())
}

/// Fetches metadata only; never executes or downloads a helper binary.
pub fn update_candidate(cancel: &AtomicBool) -> Result<Option<CometUpdate>> {
    let candidate = candidate_from_bytes(&fetch(RELEASE, 1024 * 1024, cancel, |_| {})?)?;
    candidate_for_directory(candidate, &super::effective_directory()?)
}

fn candidate_for_directory(
    candidate: CometUpdate,
    effective: &Path,
) -> Result<Option<CometUpdate>> {
    match fs::symlink_metadata(effective) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Some(candidate)),
        result => {
            result?;
        }
    };
    if crate::compatibility::components::newer_version(
        &candidate.version,
        &super::verify_build(effective)?.version,
    )? {
        Ok(Some(candidate))
    } else {
        Ok(None)
    }
}

/// Called only after confirmation. Comet itself retains upstream peer acquisition behavior.
pub fn install_update(
    candidate: &CometUpdate,
    cancel: &AtomicBool,
    mut progress: impl FnMut(crate::compatibility::acquisition::DownloadProgress),
) -> Result<()> {
    let _activity = crate::profile_reset::begin_activity("Comet update")?;
    validate_candidate(candidate)?;
    let mut files = BTreeMap::new();
    let mut completed_before = 0;
    for asset in &candidate.assets {
        let bytes = fetch(
            &asset.browser_download_url,
            asset.size,
            cancel,
            |completed| {
                progress(crate::compatibility::acquisition::DownloadProgress {
                    phase: "Downloading official Comet",
                    completed: completed_before + completed,
                    total: Some(candidate.download_bytes),
                })
            },
        )?;
        ensure!(
            bytes.len() as u64 == asset.size
                && format!("sha256:{:x}", Sha256::digest(&bytes))
                    == asset.digest.as_deref().unwrap(),
            "Comet asset checksum mismatch"
        );
        completed_before += asset.size;
        files.insert(asset.name.clone(), bytes);
    }
    publish_at(
        &storage(),
        &super::bundled_directory(),
        candidate,
        &files,
        cancel,
    )
}

fn publish_at(
    root: &Path,
    bundled: &Path,
    candidate: &CometUpdate,
    files: &BTreeMap<String, Vec<u8>>,
    cancel: &AtomicBool,
) -> Result<()> {
    validate_candidate(candidate)?;
    cancelled(cancel)?;
    for asset in &candidate.assets {
        let bytes = files.get(&asset.name).context("Incomplete Comet update")?;
        ensure!(
            bytes.len() as u64 == asset.size
                && format!("sha256:{:x}", Sha256::digest(bytes))
                    == asset.digest.as_deref().unwrap(),
            "Comet asset checksum mismatch"
        );
    }
    if let Some(parent) = root.parent()
        && parent.try_exists()?
    {
        ensure!(
            fs::symlink_metadata(parent)?.is_dir(),
            "Comet storage parent must be a real directory"
        );
    }
    fs::create_dir_all(root)?;
    ensure!(
        fs::symlink_metadata(root)?.is_dir(),
        "Comet storage must be a real directory"
    );
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(".lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("Another Comet update is running")?;
    let mut versions = Vec::new();
    if let Some(path) = active_at(root)? {
        versions.push(super::verify_build(&path)?.version);
    }
    match fs::symlink_metadata(bundled) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        result => {
            result?;
            versions.push(super::verify_build(bundled)?.version);
        }
    }
    for installed in versions {
        ensure!(
            crate::compatibility::components::newer_version(&candidate.version, &installed)?,
            "Comet update is not newer than the installed helper"
        );
    }
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(root)?;
    let mut checksums = BTreeMap::new();
    for (asset_name, filename) in BINARIES {
        cancelled(cancel)?;
        let bytes = files.get(*asset_name).context("Incomplete Comet update")?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if *filename == "comet" { 0o755 } else { 0o644 })
            .open(staging.path().join(filename))?;
        output.write_all(bytes)?;
        output.sync_all()?;
        checksums.insert((*filename).into(), format!("{:x}", Sha256::digest(bytes)));
    }
    let build = serde_json::to_vec(&Build {
        version: candidate.version.clone(),
        files: checksums,
    })?;
    let mut metadata = File::create(staging.path().join("build.json"))?;
    metadata.write_all(&build)?;
    metadata.sync_all()?;
    let mut provenance = File::create(staging.path().join("upstream-assets.json"))?;
    provenance.write_all(&serde_json::to_vec(&candidate.assets)?)?;
    provenance.sync_all()?;
    super::verify_build(staging.path())?;
    cancelled(cancel)?;
    let directory = format!("{}-{:x}", candidate.version, Sha256::digest(&build));
    let destination = root.join(&directory);
    if destination.try_exists()? {
        ensure!(
            fs::symlink_metadata(&destination)?.is_dir(),
            "Invalid staged Comet directory"
        );
        for name in [
            "comet",
            "GalaxyCommunication.exe",
            "build.json",
            "upstream-assets.json",
        ] {
            ensure!(
                local_bytes(&destination.join(name), MAX_BINARY)?
                    == local_bytes(&staging.path().join(name), MAX_BINARY)?,
                "An existing staged Comet update is damaged"
            );
        }
    } else {
        fs::rename(staging.path(), &destination)?;
    }
    let mut pointer = tempfile::NamedTempFile::new_in(root)?;
    pointer.write_all(&serde_json::to_vec(&directory)?)?;
    pointer.as_file().sync_all()?;
    cancelled(cancel)?;
    pointer.persist(root.join("current.json"))?;
    File::open(root)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(version: &str) -> (CometUpdate, BTreeMap<String, Vec<u8>>) {
        let files: BTreeMap<String, Vec<u8>> = BINARIES
            .iter()
            .map(|(name, _)| {
                (
                    (*name).into(),
                    format!("synthetic-{version}-{name}").into_bytes(),
                )
            })
            .collect();
        let assets: Vec<_> = files
            .iter()
            .map(|(name, bytes)| Asset {
                name: name.clone(),
                browser_download_url: format!("{ASSETS}v{version}/{name}"),
                size: bytes.len() as u64,
                digest: Some(format!("sha256:{:x}", Sha256::digest(bytes))),
            })
            .collect();
        (
            CometUpdate {
                version: version.into(),
                download_bytes: assets.iter().map(|asset| asset.size).sum(),
                assets,
            },
            files,
        )
    }

    #[test]
    fn missing_helper_bootstraps_current_release_but_existing_helpers_remain_protected() {
        let root = tempfile::tempdir().unwrap();
        let bundled = root.path().join("bundled");
        let storage = root.path().join("updates");
        let (candidate, files) = fixture("0.3.2");
        assert!(
            candidate_for_directory(candidate.clone(), &bundled)
                .unwrap()
                .is_some()
        );
        publish_at(
            &storage,
            &bundled,
            &candidate,
            &files,
            &AtomicBool::new(false),
        )
        .unwrap();
        let active = active_at(&storage).unwrap().unwrap();
        assert_eq!(
            super::super::verify_build(&active).unwrap().version,
            "0.3.2"
        );
        assert!(
            candidate_for_directory(candidate.clone(), &active)
                .unwrap()
                .is_none()
        );
        assert!(
            publish_at(
                &storage,
                &bundled,
                &candidate,
                &files,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        let (older, old_files) = fixture("0.3.1");
        assert!(
            candidate_for_directory(older.clone(), &active)
                .unwrap()
                .is_none()
        );
        assert!(
            publish_at(
                &storage,
                &bundled,
                &older,
                &old_files,
                &AtomicBool::new(false)
            )
            .is_err()
        );

        fs::create_dir(&bundled).unwrap();
        assert!(candidate_for_directory(candidate.clone(), &bundled).is_err());
        assert!(
            publish_at(
                &root.path().join("other"),
                &bundled,
                &candidate,
                &files,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        fs::remove_dir(&bundled).unwrap();
        std::os::unix::fs::symlink(root.path().join("absent"), &bundled).unwrap();
        assert!(candidate_for_directory(candidate.clone(), &bundled).is_err());
        assert!(
            publish_at(
                &root.path().join("other"),
                &bundled,
                &candidate,
                &files,
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }

    #[test]
    fn verified_publication_keeps_previous_version_and_rejects_downgrade() {
        let root = tempfile::tempdir().unwrap();
        let (first, files) = fixture("90.0.0");
        publish_at(
            root.path(),
            &root.path().join("missing"),
            &first,
            &files,
            &AtomicBool::new(false),
        )
        .unwrap();
        let previous = active_at(root.path()).unwrap().unwrap();
        let (second, files) = fixture("91.0.0");
        publish_at(
            root.path(),
            &root.path().join("missing"),
            &second,
            &files,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(previous.join("comet").exists());
        assert_eq!(
            super::super::verify_build(&active_at(root.path()).unwrap().unwrap())
                .unwrap()
                .version,
            "91.0.0"
        );
        let (first, files) = fixture("90.0.0");
        assert!(
            publish_at(
                root.path(),
                &root.path().join("missing"),
                &first,
                &files,
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }

    #[test]
    fn failed_and_cancelled_updates_preserve_active_helper() {
        let root = tempfile::tempdir().unwrap();
        let (first, files) = fixture("90.0.0");
        publish_at(
            root.path(),
            &root.path().join("missing"),
            &first,
            &files,
            &AtomicBool::new(false),
        )
        .unwrap();
        let pointer = fs::read(root.path().join("current.json")).unwrap();
        let (next, mut files) = fixture("91.0.0");
        assert!(
            publish_at(
                root.path(),
                &root.path().join("missing"),
                &next,
                &files,
                &AtomicBool::new(true)
            )
            .is_err()
        );
        files
            .get_mut("comet-x86_64-unknown-linux-gnu")
            .unwrap()
            .push(0);
        assert!(
            publish_at(
                root.path(),
                &root.path().join("missing"),
                &next,
                &files,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(fs::read(root.path().join("current.json")).unwrap(), pointer);
    }

    #[test]
    fn interrupted_pointer_publication_reuses_complete_directory() {
        let root = tempfile::tempdir().unwrap();
        let (candidate, files) = fixture("90.0.0");
        publish_at(
            root.path(),
            &root.path().join("missing"),
            &candidate,
            &files,
            &AtomicBool::new(false),
        )
        .unwrap();
        let expected = fs::read(root.path().join("current.json")).unwrap();
        fs::remove_file(root.path().join("current.json")).unwrap();
        publish_at(
            root.path(),
            &root.path().join("missing"),
            &candidate,
            &files,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            fs::read(root.path().join("current.json")).unwrap(),
            expected
        );
    }

    #[test]
    fn rejects_unofficial_missing_digest_and_duplicate_release_assets() {
        let (mut candidate, _) = fixture("90.0.0");
        candidate.assets[0].browser_download_url = candidate.assets[0]
            .browser_download_url
            .replace("imLinguin/comet", "other/comet");
        assert!(validate_candidate(&candidate).is_err());
        let (mut candidate, _) = fixture("90.0.0");
        candidate.assets[0].digest = None;
        assert!(validate_candidate(&candidate).is_err());
        let (mut candidate, _) = fixture("90.0.0");
        candidate.assets[1] = candidate.assets[0].clone();
        assert!(validate_candidate(&candidate).is_err());
        assert!(validate_version("../../file").is_err());
    }

    #[test]
    fn stable_release_metadata_selects_only_the_official_helper_pair() {
        let (candidate, _) = fixture("90.0.0");
        let mut release = serde_json::json!({"tag_name":"v90.0.0", "draft":false, "prerelease":false, "assets":candidate.assets});
        let parsed = candidate_from_bytes(&serde_json::to_vec(&release).unwrap()).unwrap();
        assert_eq!(parsed.version, "90.0.0");
        assert_eq!(parsed.download_bytes, candidate.download_bytes);
        release["prerelease"] = true.into();
        assert!(candidate_from_bytes(&serde_json::to_vec(&release).unwrap()).is_err());
    }

    #[test]
    fn storage_symlinks_and_pointer_traversal_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(external.path(), root.path().join("comet")).unwrap();
        let (candidate, files) = fixture("90.0.0");
        assert!(
            publish_at(
                &root.path().join("comet/helpers"),
                &root.path().join("missing"),
                &candidate,
                &files,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(!external.path().join("helpers").exists());
        fs::write(root.path().join("current.json"), br#""../external""#).unwrap();
        assert!(active_at(root.path()).is_err());
    }
}
