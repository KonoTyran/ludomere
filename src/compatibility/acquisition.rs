//! Explicit, worker-thread-only acquisition from the publishers' HTTPS endpoints.
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const MAX_DOWNLOAD: u64 = 4 * 1024 * 1024 * 1024;
const MAX_EXTRACTED: u64 = 16 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 300_000;
// Valve lists checksums for every image artifact; current manifests exceed 256 KiB.
const MAX_RUNTIME_CHECKSUMS: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtonFamily {
    Ge,
    Umu,
}

impl ProtonFamily {
    fn repository(self) -> &'static str {
        match self {
            Self::Ge => "GloriousEggroll/proton-ge-custom",
            Self::Umu => "Open-Wine-Components/umu-proton",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Release {
    pub name: String,
    pub family: ProtonFamily,
    archive: Asset,
    checksum: Option<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Clone, Debug)]
pub struct DownloadProgress {
    pub phase: &'static str,
    pub completed: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct RuntimeRequirement {
    pub name: &'static str,
    pub variant: &'static str,
    pub path: PathBuf,
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    ensure!(
        !cancelled.load(Ordering::Relaxed) && !crate::profile_reset::stopping_operations(),
        "Download cancelled"
    );
    Ok(())
}

fn trusted_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "repo.steampowered.com"
            )
        )
}

fn client() -> Result<Client> {
    Client::builder()
        .user_agent(crate::identity::USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !trusted_url(attempt.url()) {
                attempt.error("Component download redirected outside approved publishers")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .context("Could not initialize component downloads")
}

fn response(client: &Client, url: &str) -> Result<reqwest::blocking::Response> {
    ensure!(trusted_url(&Url::parse(url)?), "Untrusted component source");
    // Do not include reqwest errors: redirects can contain signed URLs.
    let response = client
        .get(url)
        .send()
        .map_err(|_| anyhow::anyhow!("Component request failed; check your connection"))?;
    ensure!(
        response.status().is_success(),
        "Component publisher returned HTTP {}",
        response.status().as_u16()
    );
    Ok(response)
}

fn metadata(client: &Client, url: &str, limit: u64, cancelled: &AtomicBool) -> Result<Vec<u8>> {
    check_cancelled(cancelled)?;
    read_metadata(response(client, url)?, limit, cancelled)
}

fn read_metadata(mut source: impl Read, limit: u64, cancelled: &AtomicBool) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        check_cancelled(cancelled)?;
        let count = source
            .read(&mut buffer)
            .map_err(|_| anyhow::anyhow!("Could not read component metadata"))?;
        if count == 0 {
            break;
        }
        ensure!(
            (body.len() + count) as u64 <= limit,
            "Component metadata exceeded its size limit"
        );
        body.extend_from_slice(&buffer[..count]);
    }
    Ok(body)
}

/// Fetch stable historical releases as well as current releases, without downloading payloads.
pub fn list_releases(family: ProtonFamily, cancelled: &AtomicBool) -> Result<Vec<Release>> {
    let client = client()?;
    let mut releases = Vec::new();
    for page in 1..=50 {
        let batch: Vec<GithubRelease> = serde_json::from_slice(&metadata(
            &client,
            &format!(
                "https://api.github.com/repos/{}/releases?per_page=100&page={page}",
                family.repository()
            ),
            8 * 1024 * 1024,
            cancelled,
        )?)?;
        let done = batch.len() < 100;
        releases.extend(
            batch
                .into_iter()
                .filter_map(|release| catalog_entry(family, release)),
        );
        if done {
            return Ok(releases);
        }
    }
    bail!(
        "Release catalog exceeded 5000 entries; narrow the supported upstream catalog before retrying"
    )
}

fn catalog_entry(family: ProtonFamily, release: GithubRelease) -> Option<Release> {
    if release.draft
        || release.prerelease
        || ["alpha", "beta", "preview", "experimental", "-rc", "-test"]
            .iter()
            .any(|label| release.tag_name.to_ascii_lowercase().contains(label))
    {
        return None;
    }
    let prefix = match family {
        ProtonFamily::Ge => "GE-Proton",
        ProtonFamily::Umu => "UMU-Proton",
    };
    if !(release.tag_name.starts_with(prefix)
        || family == ProtonFamily::Ge && release.tag_name.contains("-GE-"))
        || !safe_name(&release.tag_name)
    {
        return None;
    }
    let archive = release
        .assets
        .iter()
        .find(|asset| {
            [
                format!("{}.tar.gz", release.tag_name),
                format!("{}-x86_64.tar.gz", release.tag_name),
                format!("Proton-{}.tar.gz", release.tag_name),
            ]
            .contains(&asset.name)
                && asset.size > 0
                && asset.size <= MAX_DOWNLOAD
        })?
        .clone();
    let checksum = release
        .assets
        .iter()
        .find(|asset| {
            asset.name == format!("{}.sha512sum", archive.name.trim_end_matches(".tar.gz"))
        })
        .cloned();
    if checksum.is_none()
        && !archive
            .digest
            .as_deref()
            .is_some_and(|digest| valid_digest(digest, "sha256:", 64))
    {
        return None;
    }
    let base = format!(
        "https://github.com/{}/releases/download/",
        family.repository()
    );
    if !archive.browser_download_url.starts_with(&base)
        || checksum
            .as_ref()
            .is_some_and(|asset| !asset.browser_download_url.starts_with(&base))
    {
        return None;
    }
    Some(Release {
        name: release.tag_name,
        family,
        archive,
        checksum,
    })
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 200
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        && name != "."
        && name != ".."
}

fn valid_digest(value: &str, prefix: &str, length: usize) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|hash| hash.len() == length && hash.bytes().all(|c| c.is_ascii_hexdigit()))
}

fn checksum_for(text: &[u8], filename: &str, length: usize) -> Result<String> {
    for line in std::str::from_utf8(text)?.lines() {
        let mut fields = line.split_whitespace();
        if let (Some(hash), Some(name)) = (fields.next(), fields.next())
            && name.trim_start_matches('*') == filename
            && fields.next().is_none()
            && valid_digest(hash, "", length)
        {
            return Ok(hash.to_ascii_lowercase());
        }
    }
    bail!("Publisher checksum does not identify the requested archive")
}

/// Download only after a direct user action; the returned directory is application-owned.
pub fn download_proton(
    release: &Release,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf> {
    let _activity = crate::profile_reset::begin_activity("Proton download")?;
    ensure!(safe_name(&release.name), "Invalid Proton release name");
    let client = client()?;
    let expected = if let Some(checksum) = &release.checksum {
        checksum_for(
            &metadata(
                &client,
                &checksum.browser_download_url,
                64 * 1024,
                cancelled,
            )?,
            &release.archive.name,
            128,
        )?
    } else {
        release
            .archive
            .digest
            .as_deref()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .context("Release has no publisher checksum")?
            .to_ascii_lowercase()
    };
    let root = owned_directory("proton")?;
    let _lock = download_lock(&root)?;
    let destination = root.join(&release.name);
    ensure!(
        !destination.try_exists()?,
        "This Proton release is already installed"
    );
    let stage = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(&root)?;
    let archive = stage.path().join("archive");
    download(
        &client,
        &release.archive.browser_download_url,
        Some(release.archive.size),
        &expected,
        &archive,
        cancelled,
        &mut progress,
    )?;
    let extracted = stage.path().join("extracted");
    fs::create_dir(&extracted)?;
    let payload = extract(
        flate2::read::GzDecoder::new(File::open(&archive)?),
        &extracted,
        cancelled,
        &mut progress,
    )?;
    ensure!(
        payload.join("proton").is_file() && payload.join("toolmanifest.vdf").is_file(),
        "Archive is not a Proton distribution"
    );
    runtime_requirement(&payload)?;
    check_cancelled(cancelled)?;
    fs::rename(payload, &destination)?;
    progress(DownloadProgress {
        phase: "Complete",
        completed: 1,
        total: Some(1),
    });
    Ok(destination)
}

fn owned_directory(child: &str) -> Result<PathBuf> {
    let root = crate::identity::data_root().join(child);
    fs::create_dir_all(&root)?;
    ensure!(
        !fs::symlink_metadata(&root)?.file_type().is_symlink(),
        "Component storage must not be a symbolic link"
    );
    Ok(root.canonicalize()?)
}

fn download_lock(root: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(".download.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("Another component download is in progress")?;
    Ok(lock)
}

fn download(
    client: &Client,
    url: &str,
    size: Option<u64>,
    expected: &str,
    target: &Path,
    cancelled: &AtomicBool,
    progress: &mut impl FnMut(DownloadProgress),
) -> Result<()> {
    check_cancelled(cancelled)?;
    let source = response(client, url)?;
    let total = size.or(source.content_length());
    receive_archive(source, total, expected, target, cancelled, progress)
}

fn receive_archive(
    mut source: impl Read,
    total: Option<u64>,
    expected: &str,
    target: &Path,
    cancelled: &AtomicBool,
    progress: &mut impl FnMut(DownloadProgress),
) -> Result<()> {
    ensure!(
        total.is_none_or(|size| size <= MAX_DOWNLOAD),
        "Component archive exceeds {}",
        crate::domain::human_size(MAX_DOWNLOAD)
    );
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)?;
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    let mut completed = 0;
    let mut buffer = [0; 128 * 1024];
    loop {
        check_cancelled(cancelled)?;
        let count = source
            .read(&mut buffer)
            .map_err(|_| anyhow::anyhow!("Component download interrupted"))?;
        if count == 0 {
            break;
        }
        completed += count as u64;
        ensure!(
            completed <= MAX_DOWNLOAD && total.is_none_or(|size| completed <= size),
            "Component exceeds its declared size"
        );
        target.write_all(&buffer[..count])?;
        sha256.update(&buffer[..count]);
        sha512.update(&buffer[..count]);
        progress(DownloadProgress {
            phase: "Downloading",
            completed,
            total,
        });
    }
    ensure!(
        total.is_none_or(|size| completed == size),
        "Incomplete component download"
    );
    let actual = if expected.len() == 128 {
        format!("{:x}", sha512.finalize())
    } else {
        format!("{:x}", sha256.finalize())
    };
    ensure!(
        actual == expected,
        "Component checksum mismatch; nothing was installed"
    );
    target.sync_all()?;
    Ok(())
}

/// UMU 1.4.4's manifest mapping. Unknown future runtime IDs fail closed.
pub fn runtime_requirement(proton: &Path) -> Result<Option<RuntimeRequirement>> {
    let mut manifest = String::new();
    File::open(proton.join("toolmanifest.vdf"))?
        .take(64 * 1024 + 1)
        .read_to_string(&mut manifest)?;
    ensure!(manifest.len() <= 64 * 1024, "Proton manifest is too large");
    let tokens = manifest_tokens(&manifest)?;
    ensure!(
        tokens.len() >= 3
            && tokens[0] == "manifest"
            && tokens[1] == "{"
            && tokens.last().is_some_and(|token| token == "}"),
        "Invalid Proton tool manifest"
    );
    let fields = &tokens[2..tokens.len() - 1];
    ensure!(
        fields.len().is_multiple_of(2) && !fields.iter().any(|token| token == "{" || token == "}"),
        "Invalid Proton manifest fields"
    );
    let requirements: Vec<_> = fields
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|pair| pair[0] == "require_tool_appid")
        .map(|pair| pair[1].as_str())
        .collect();
    ensure!(
        requirements.len() <= 1,
        "Ambiguous Proton runtime requirement"
    );
    let Some(appid) = requirements.first() else {
        return Ok(None);
    };
    let (name, variant) = match *appid {
        "1391110" => ("soldier", "steamrt2"),
        "1628350" => ("sniper", "steamrt3"),
        "4183110" => ("steamrt4", "steamrt4"),
        _ => bail!(
            "This Proton requires an unsupported Steam Linux Runtime ({appid}); choose another version"
        ),
    };
    Ok(Some(RuntimeRequirement {
        name,
        variant,
        path: crate::identity::data_root().join("umu").join(variant),
    }))
}

fn manifest_tokens(input: &str) -> Result<Vec<String>> {
    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            for next in chars.by_ref() {
                if next == '\n' {
                    break;
                }
            }
        } else if c == '"' {
            let mut token = String::new();
            let mut closed = false;
            while let Some(next) = chars.next() {
                if next == '"' {
                    closed = true;
                    break;
                }
                if next == '\\' {
                    token.push(chars.next().context("Truncated manifest escape")?);
                } else {
                    token.push(next);
                }
            }
            ensure!(closed, "Unterminated manifest token");
            tokens.push(token);
        } else if c == '{' || c == '}' {
            tokens.push(c.to_string());
        } else {
            let mut token = String::from(c);
            while chars
                .peek()
                .is_some_and(|c| !c.is_whitespace() && !matches!(c, '{' | '}'))
            {
                token.push(chars.next().unwrap());
            }
            tokens.push(token);
        }
    }
    Ok(tokens)
}

pub fn runtime_ready(runtime: &RuntimeRequirement) -> bool {
    [
        ".installed.ok",
        "_v2-entry-point",
        "toolmanifest.vdf",
        "pressure-vessel/bin/pv-verify",
        "VERSIONS.txt",
    ]
    .iter()
    .all(|file| runtime.path.join(file).is_file())
        && fs::read_dir(&runtime.path).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{}_platform_", runtime.name))
                    && entry.path().join("files").is_dir()
            })
        })
}

pub fn download_runtime(
    runtime: &RuntimeRequirement,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf> {
    let _activity = crate::profile_reset::begin_activity("runtime download")?;
    ensure!(
        matches!(
            (runtime.name, runtime.variant),
            ("soldier", "steamrt2") | ("sniper", "steamrt3") | ("steamrt4", "steamrt4")
        ),
        "Unsupported runtime"
    );
    let root = owned_directory("umu")?;
    let destination = root.join(runtime.variant);
    ensure!(
        runtime.path
            == crate::identity::data_root()
                .join("umu")
                .join(runtime.variant),
        "Runtime destination is not application-owned"
    );
    let _lock = download_lock(&root)?;
    ensure!(
        !destination.try_exists()?,
        "Runtime directory already exists; move the incomplete application runtime aside before retrying"
    );
    let client = client()?;
    let base = format!("https://repo.steampowered.com/{}/images", runtime.variant);
    let version = String::from_utf8(metadata(
        &client,
        &format!("{base}/latest-public-beta.txt"),
        256,
        cancelled,
    )?)?;
    let version = version.trim();
    ensure!(
        safe_name(version) && version.bytes().all(|c| c.is_ascii_digit() || c == b'.'),
        "Invalid runtime build identifier"
    );
    let archive_name = format!(
        "SteamLinuxRuntime_{}.tar.xz",
        runtime.name.trim_start_matches("steamrt")
    );
    let expected = checksum_for(
        &metadata(
            &client,
            &format!("{base}/{version}/SHA256SUMS"),
            MAX_RUNTIME_CHECKSUMS,
            cancelled,
        )?,
        &archive_name,
        64,
    )?;
    let stage = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(&root)?;
    let archive = stage.path().join("archive");
    download(
        &client,
        &format!("{base}/{version}/{archive_name}"),
        None,
        &expected,
        &archive,
        cancelled,
        &mut progress,
    )?;
    let extracted = stage.path().join("extracted");
    fs::create_dir(&extracted)?;
    let payload = extract(
        xz2::read::XzDecoder::new_stream(
            File::open(&archive)?,
            xz2::stream::Stream::new_stream_decoder(256 * 1024 * 1024, 0)?,
        ),
        &extracted,
        cancelled,
        &mut progress,
    )?;
    // The complete archive was checked against Valve's SHA256SUMS; do not execute pv-verify here.
    fs::write(payload.join(".installed.ok"), "ok\n")?;
    ensure!(
        runtime_ready(&RuntimeRequirement {
            path: payload.clone(),
            ..runtime.clone()
        }),
        "Runtime archive is incomplete"
    );
    if !payload.join("umu").try_exists()? {
        symlink("_v2-entry-point", payload.join("umu"))?;
    }
    check_cancelled(cancelled)?;
    fs::rename(payload, &destination)?;
    progress(DownloadProgress {
        phase: "Complete",
        completed: 1,
        total: Some(1),
    });
    Ok(destination)
}

fn relative_path(path: &Path) -> Result<PathBuf> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => (),
            _ => bail!("Unsafe archive path"),
        }
    }
    ensure!(!clean.as_os_str().is_empty(), "Empty archive path");
    Ok(clean)
}

fn confined_link(path: &Path, target: &Path, hard: bool) -> Result<PathBuf> {
    ensure!(!target.is_absolute(), "Absolute archive link");
    let mut resolved = if hard {
        PathBuf::new()
    } else {
        path.parent().context("Invalid link path")?.to_path_buf()
    };
    for part in target.components() {
        match part {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => (),
            Component::ParentDir => {
                ensure!(resolved.pop(), "Archive link escapes destination");
            }
            _ => bail!("Unsafe archive link"),
        }
    }
    ensure!(
        resolved.components().next() == path.components().next(),
        "Archive link escapes distribution"
    );
    Ok(resolved)
}

fn validate_link_graph(links: &[(PathBuf, PathBuf, PathBuf, bool)]) -> Result<()> {
    let targets: HashMap<_, _> = links
        .iter()
        .map(|(path, target, _, hard)| (path.clone(), (target, hard)))
        .collect();
    for (path, target, _, hard) in links {
        ensure!(
            !path
                .ancestors()
                .skip(1)
                .any(|parent| targets.contains_key(parent)),
            "Archive entry has a link as its parent"
        );
        if *hard {
            continue;
        }
        let mut resolved = path.parent().unwrap().to_path_buf();
        let mut pending: VecDeque<_> = target
            .components()
            .map(|part| part.as_os_str().to_os_string())
            .collect();
        let mut followed = 0;
        while let Some(part) = pending.pop_front() {
            if part == "." {
                continue;
            }
            if part == ".." {
                ensure!(
                    resolved.components().count() > 1,
                    "Archive link chain escapes distribution"
                );
                resolved.pop();
            } else {
                resolved.push(&part);
                if let Some((next, false)) = targets.get(&resolved) {
                    followed += 1;
                    ensure!(
                        followed <= 40,
                        "Archive contains a cyclic or excessive link chain"
                    );
                    resolved.pop();
                    for part in next.components().rev() {
                        pending.push_front(part.as_os_str().to_os_string());
                    }
                }
            }
        }
    }
    Ok(())
}

fn extract(
    reader: impl Read,
    destination: &Path,
    cancelled: &AtomicBool,
    progress: &mut impl FnMut(DownloadProgress),
) -> Result<PathBuf> {
    let mut archive = tar::Archive::new(reader.take(MAX_EXTRACTED + 1));
    let mut links = Vec::new();
    let mut paths = HashSet::new();
    let mut top = None;
    let mut expanded = 0u64;
    let mut next_path = None;
    let mut next_link = None;
    // Raw entries let us bound extension metadata before tar allocates it.
    for (count, entry) in archive.entries()?.raw(true).enumerate() {
        check_cancelled(cancelled)?;
        ensure!(count < MAX_ENTRIES, "Archive contains too many entries");
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        let size = entry.size();
        expanded = expanded
            .checked_add(size)
            .context("Archive size overflow")?;
        ensure!(
            expanded <= MAX_EXTRACTED,
            "Expanded archive exceeds {}",
            crate::domain::human_size(MAX_EXTRACTED)
        );
        if kind.is_gnu_longname() || kind.is_gnu_longlink() || kind.is_pax_local_extensions() {
            ensure!(size <= 64 * 1024, "Archive extension metadata is too large");
            let mut body = String::new();
            entry.read_to_string(&mut body)?;
            if kind.is_gnu_longname() {
                ensure!(next_path.is_none(), "Duplicate archive path extension");
                next_path = Some(PathBuf::from(body.trim_end_matches('\0')));
            } else if kind.is_gnu_longlink() {
                ensure!(next_link.is_none(), "Duplicate archive link extension");
                next_link = Some(PathBuf::from(body.trim_end_matches('\0')));
            } else {
                for line in body.lines() {
                    let (length, data) = line.split_once(' ').context("Invalid PAX record")?;
                    ensure!(
                        length.parse::<usize>()? == line.len() + 1,
                        "Invalid PAX length"
                    );
                    let (key, value) = data.split_once('=').context("Invalid PAX field")?;
                    match key {
                        "path" => next_path = Some(PathBuf::from(value)),
                        "linkpath" => next_link = Some(PathBuf::from(value)),
                        "size" => bail!("PAX size override is unsupported"),
                        key if key.starts_with("GNU.sparse") => {
                            bail!("Sparse archive is unsupported")
                        }
                        _ => (),
                    }
                }
            }
            continue;
        }
        let path = relative_path(&next_path.take().unwrap_or(entry.path()?.into_owned()))?;
        let first = PathBuf::from(path.components().next().unwrap().as_os_str());
        if let Some(top) = &top {
            ensure!(top == &first, "Archive has multiple distribution roots");
        } else {
            top = Some(first);
        }
        ensure!(
            kind.is_dir() || paths.insert(path.clone()),
            "Duplicate archive entry"
        );
        let output = destination.join(&path);
        if kind.is_dir() {
            fs::create_dir_all(output)?;
        } else if kind.is_file() {
            fs::create_dir_all(output.parent().unwrap())?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&output)?;
            let mut buffer = [0; 128 * 1024];
            loop {
                check_cancelled(cancelled)?;
                let count = entry.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                file.write_all(&buffer[..count])?;
            }
            fs::set_permissions(
                output,
                fs::Permissions::from_mode(0o600 | (entry.header().mode()? & 0o111)),
            )?;
        } else if kind.is_symlink() || kind.is_hard_link() {
            let target = next_link
                .take()
                .or(entry.link_name()?.map(|path| path.into_owned()))
                .context("Archive link has no target")?;
            let resolved = confined_link(&path, &target, kind.is_hard_link())?;
            links.push((path, target, resolved, kind.is_hard_link()));
        } else {
            bail!("Unsupported archive entry type");
        }
        next_link = None;
        progress(DownloadProgress {
            phase: "Extracting",
            completed: expanded,
            total: None,
        });
    }
    ensure!(
        next_path.is_none() && next_link.is_none(),
        "Dangling archive extension"
    );
    // Files and directories first; no archive-controlled link can redirect any write.
    validate_link_graph(&links)?;
    links.sort_by_key(|entry| !entry.3);
    for (path, target, resolved, hard) in links {
        check_cancelled(cancelled)?;
        let output = destination.join(path);
        fs::create_dir_all(output.parent().unwrap())?;
        if hard {
            ensure!(
                fs::symlink_metadata(destination.join(&resolved))?
                    .file_type()
                    .is_file(),
                "Hard link does not target a regular file"
            );
            fs::hard_link(destination.join(resolved), output)?;
        } else {
            symlink(target, output)?;
        }
    }
    Ok(destination.join(top.context("Archive is empty")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn fixture(entries: &[(&str, tar::EntryType, &str)]) -> Vec<u8> {
        let mut archive = tar::Builder::new(Vec::new());
        for (path, kind, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(0o6755);
            if kind.is_file() {
                header.set_size(contents.len() as u64);
                header.set_cksum();
                archive
                    .append_data(&mut header, path, contents.as_bytes())
                    .unwrap();
            } else if kind.is_symlink() || kind.is_hard_link() {
                header.set_size(0);
                archive.append_link(&mut header, path, contents).unwrap();
            } else {
                header.set_size(0);
                header.set_cksum();
                archive
                    .append_data(&mut header, path, std::io::empty())
                    .unwrap();
            }
        }
        archive.into_inner().unwrap()
    }

    #[test]
    fn extraction_preserves_internal_links_and_strips_special_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = fixture(&[
            ("Proton/bin/wine", tar::EntryType::Regular, "payload"),
            ("Proton/wine", tar::EntryType::Symlink, "bin/wine"),
            ("Proton/bin/wine64", tar::EntryType::Link, "Proton/bin/wine"),
        ]);
        let root = extract(
            Cursor::new(bytes),
            temp.path(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(fs::read(root.join("wine")).unwrap(), b"payload");
        assert_eq!(fs::read(root.join("bin/wine64")).unwrap(), b"payload");
        assert_eq!(
            fs::metadata(root.join("wine"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o711
        );
    }

    #[test]
    fn extraction_rejects_external_links_devices_duplicates_and_multiple_roots() {
        for entries in [
            vec![("Proton/escape", tar::EntryType::Symlink, "/tmp/outside")],
            vec![("Proton/escape", tar::EntryType::Symlink, "../../outside")],
            vec![("Proton/device", tar::EntryType::Char, "")],
            vec![
                ("Proton/file", tar::EntryType::Regular, "a"),
                ("Proton/file", tar::EntryType::Regular, "b"),
            ],
            vec![
                ("Proton/file", tar::EntryType::Regular, "a"),
                ("Other/file", tar::EntryType::Regular, "b"),
            ],
            vec![
                ("Proton/link", tar::EntryType::Symlink, "."),
                ("Proton/escape", tar::EntryType::Symlink, "link/../outside"),
            ],
            vec![
                ("Proton/link", tar::EntryType::Symlink, "."),
                ("Proton/link/child", tar::EntryType::Symlink, "../outside"),
            ],
        ] {
            let temp = tempfile::tempdir().unwrap();
            assert!(
                extract(
                    Cursor::new(fixture(&entries)),
                    temp.path(),
                    &AtomicBool::new(false),
                    &mut |_| {}
                )
                .is_err(),
                "{entries:?}"
            );
        }
    }

    #[test]
    fn extraction_rejects_traversal_extensions_and_cancellation() {
        assert!(relative_path(Path::new("../escape")).is_err());
        assert!(relative_path(Path::new("/escape")).is_err());
        let mut archive = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::GNULongName);
        header.set_size(65537);
        header.set_mode(0o600);
        header.set_cksum();
        archive
            .append_data(&mut header, "././@LongLink", &vec![b'a'; 65537][..])
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        assert!(
            extract(
                Cursor::new(archive.into_inner().unwrap()),
                temp.path(),
                &AtomicBool::new(false),
                &mut |_| {}
            )
            .unwrap_err()
            .to_string()
            .contains("metadata")
        );
        assert!(
            extract(
                Cursor::new(fixture(&[("Proton/file", tar::EntryType::Regular, "a")])),
                temp.path(),
                &AtomicBool::new(true),
                &mut |_| {}
            )
            .unwrap_err()
            .to_string()
            .contains("cancelled")
        );
    }

    #[test]
    fn archive_receiver_verifies_digest_size_and_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let expected = format!("{:x}", Sha256::digest(b"payload"));
        assert_eq!(
            receive_archive(
                Cursor::new(b""),
                Some(MAX_DOWNLOAD + 1),
                &expected,
                &temp.path().join("oversized"),
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_err()
            .to_string(),
            "Component archive exceeds 4.3 GB"
        );
        assert!(!temp.path().join("oversized").exists());
        receive_archive(
            Cursor::new(b"payload"),
            Some(7),
            &expected,
            &temp.path().join("good"),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        for (name, size, digest, cancelled) in [
            ("short", Some(8), expected.as_str(), false),
            ("long", Some(6), expected.as_str(), false),
            ("digest", Some(7), "incorrect", false),
            ("cancelled", Some(7), expected.as_str(), true),
        ] {
            assert!(
                receive_archive(
                    Cursor::new(b"payload"),
                    size,
                    digest,
                    &temp.path().join(name),
                    &AtomicBool::new(cancelled),
                    &mut |_| {}
                )
                .is_err()
            );
        }
        let cancelled = AtomicBool::new(false);
        assert!(
            receive_archive(
                Cursor::new(vec![0; 256 * 1024]),
                None,
                &expected,
                &temp.path().join("midway"),
                &cancelled,
                &mut |_| {
                    cancelled.store(true, Ordering::Relaxed);
                }
            )
            .is_err()
        );
    }

    #[test]
    fn catalog_filters_prereleases_architecture_and_untrusted_assets() {
        let entry = |prerelease, url: &str| GithubRelease {
            tag_name: "GE-Proton11-7".into(),
            draft: false,
            prerelease,
            assets: vec![Asset {
                name: "GE-Proton11-7-x86_64.tar.gz".into(),
                browser_download_url: url.into(),
                size: 123,
                digest: Some(format!("sha256:{}", "a".repeat(64))),
            }],
        };
        let url = "https://github.com/GloriousEggroll/proton-ge-custom/releases/download/GE-Proton11-7/GE-Proton11-7-x86_64.tar.gz";
        assert!(catalog_entry(ProtonFamily::Ge, entry(false, url)).is_some());
        assert!(catalog_entry(ProtonFamily::Ge, entry(true, url)).is_none());
        assert!(
            catalog_entry(
                ProtonFamily::Ge,
                entry(false, "https://example.com/archive")
            )
            .is_none()
        );
        let mut arm = entry(false, url);
        arm.assets[0].name = "GE-Proton11-7-aarch64.tar.gz".into();
        assert!(catalog_entry(ProtonFamily::Ge, arm).is_none());
        assert!(!trusted_url(&Url::parse("http://github.com/file").unwrap()));
        assert!(!trusted_url(
            &Url::parse("https://github.com.evil.invalid/file").unwrap()
        ));
        assert!(!trusted_url(
            &Url::parse("https://user:secret@github.com/file").unwrap()
        ));
    }

    #[test]
    fn runtime_manifest_maps_supported_ids_and_rejects_unknown_or_malformed() {
        let proton = tempfile::tempdir().unwrap();
        for (appid, variant) in [
            ("1391110", "steamrt2"),
            ("1628350", "steamrt3"),
            ("4183110", "steamrt4"),
        ] {
            fs::write(
                proton.path().join("toolmanifest.vdf"),
                format!("// comment\n\"manifest\" {{ \"require_tool_appid\" \"{appid}\" }}"),
            )
            .unwrap();
            assert_eq!(
                runtime_requirement(proton.path()).unwrap().unwrap().variant,
                variant
            );
        }
        for manifest in [
            "manifest { require_tool_appid 999 }",
            "manifest {",
            "manifest { require_tool_appid 1628350 require_tool_appid 1391110 }",
            "manifest { \"unterminated }",
        ] {
            fs::write(proton.path().join("toolmanifest.vdf"), manifest).unwrap();
            assert!(runtime_requirement(proton.path()).is_err());
        }
        fs::write(
            proton.path().join("toolmanifest.vdf"),
            "manifest { commandline proton }",
        )
        .unwrap();
        assert!(runtime_requirement(proton.path()).unwrap().is_none());
    }

    #[test]
    fn checksum_requires_exact_filename_and_valid_hash() {
        let hash = "a".repeat(64);
        assert_eq!(
            checksum_for(
                format!("{hash} *archive.tar.xz\n").as_bytes(),
                "archive.tar.xz",
                64
            )
            .unwrap(),
            hash
        );
        assert!(
            checksum_for(
                format!("{hash} *other.tar.xz\n").as_bytes(),
                "archive.tar.xz",
                64
            )
            .is_err()
        );
        assert!(checksum_for(b"invalid *archive.tar.xz", "archive.tar.xz", 64).is_err());
    }

    #[test]
    fn runtime_checksums_accept_large_manifests_with_bounded_cancellable_reads() {
        let hash = "a".repeat(64);
        let mut manifest = format!("{hash} *other-runtime-artifact.tar.gz\n").repeat(3500);
        manifest.push_str(&format!("{hash} *SteamLinuxRuntime_sniper.tar.xz\n"));
        assert!(manifest.len() > 256 * 1024);
        let cancelled = AtomicBool::new(false);
        let body = read_metadata(manifest.as_bytes(), MAX_RUNTIME_CHECKSUMS, &cancelled).unwrap();
        assert_eq!(
            checksum_for(&body, "SteamLinuxRuntime_sniper.tar.xz", 64).unwrap(),
            hash
        );
        assert!(read_metadata(manifest.as_bytes(), 256 * 1024, &cancelled).is_err());
        assert!(
            read_metadata(
                std::io::repeat(b'a').take(MAX_RUNTIME_CHECKSUMS + 1),
                MAX_RUNTIME_CHECKSUMS,
                &cancelled
            )
            .is_err()
        );
        cancelled.store(true, Ordering::Relaxed);
        assert!(
            read_metadata(manifest.as_bytes(), MAX_RUNTIME_CHECKSUMS, &cancelled)
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }

    #[test]
    fn runtime_directory_alone_is_not_ready() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = RuntimeRequirement {
            name: "sniper",
            variant: "steamrt3",
            path: temp.path().into(),
        };
        assert!(!runtime_ready(&runtime));
        fs::write(temp.path().join(".installed.ok"), "ok\n").unwrap();
        assert!(!runtime_ready(&runtime));
    }
}
