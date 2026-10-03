//! GOG's content-addressed redistributables; preparation does not execute anything.
use super::depot_manifest::{DepotEntry, DepotManifest};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

const INDEX: &str = "https://content-system.gog.com/dependencies/repository?generation=2";
const LINKS: &str =
    "https://content-system.gog.com/open_link?generation=2&_version=2&path=/dependencies/store/";
const META: &str = "https://gog-cdn-fastly.gog.com/content-system/v2/dependencies/meta";
const MAX_METADATA: usize = 8 * 1024 * 1024;
const MAX_PLAN: usize = 16 * 1024 * 1024;
static CACHE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Method {
    Exe { path: String, args: Vec<String> },
    Msi { path: String, args: Vec<String> },
    GameFiles,
    ScriptInterpreter { path: String, args: Vec<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dependency {
    pub id: String,
    pub name: String,
    pub manifest_id: String,
    pub manifest_bytes: Vec<u8>,
    pub method: Method,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub version: u32,
    pub catalog_build: String,
    pub entries: Vec<Dependency>,
}

#[derive(Debug)]
pub struct PreparedDependency {
    pub dependency: Dependency,
    pub root: PathBuf,
}

impl Dependency {
    pub fn identity(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("serializable dependency"))
        )
    }

    pub fn manifest(&self) -> Result<DepotManifest> {
        ensure!(
            valid_id(&self.id)
                && self.name.len() <= 512
                && !self.name.chars().any(char::is_control),
            "Invalid dependency identity"
        );
        ensure!(
            valid_digest(&self.manifest_id)
                && self.manifest_bytes.len() <= MAX_METADATA
                && format!("{:x}", md5::compute(&self.manifest_bytes)) == self.manifest_id,
            "Dependency manifest identity does not match its frozen plan"
        );
        let mut bytes = inflate(&self.manifest_bytes, &|| false)?;
        if matches!(self.method, Method::GameFiles) {
            let mut root: serde_json::Value =
                serde_json::from_slice(&bytes).context("Invalid dependency manifest JSON")?;
            let mut changed = false;
            if let Some(items) = root
                .get_mut("depot")
                .and_then(|depot| depot.get_mut("items"))
                .and_then(serde_json::Value::as_array_mut)
            {
                for item in items {
                    if let Some(path) = item.get_mut("path")
                        && let Some(relative) =
                            path.as_str().and_then(|path| path.strip_prefix('/'))
                    {
                        // Official game-local dependencies can use a depot-root prefix
                        // (language_setup). Remove only that marker; the normal parser
                        // and dependency validator still check every remaining component.
                        *path = serde_json::Value::String(relative.to_owned());
                        changed = true;
                    }
                }
            }
            if changed {
                bytes = serde_json::to_vec(&root)?;
                ensure!(
                    bytes.len() <= MAX_METADATA,
                    "Dependency metadata exceeds its safety limit"
                );
            }
        }
        let manifest =
            super::depot_manifest::parse(&bytes).context("Invalid dependency manifest")?;
        ensure!(
            !matches!(self.method, Method::ScriptInterpreter { .. }) || self.id == "ISI",
            "Unexpected dependency script interpreter"
        );
        validate_manifest(&manifest, &self.method)?;
        Ok(manifest)
    }
}

impl Plan {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1
                && self.catalog_build.len() <= 128
                && (!self.catalog_build.is_empty() || self.entries.is_empty())
                && self.catalog_build.bytes().all(|byte| byte.is_ascii_digit())
                && self.entries.len() <= 128
                && serde_json::to_vec(self)?.len() <= MAX_PLAN,
            "Invalid or oversized dependency plan; prepare the installation again"
        );
        let mut ids = HashSet::new();
        for entry in &self.entries {
            ensure!(ids.insert(&entry.id), "Duplicate dependency in frozen plan");
            entry.manifest()?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Index {
    build_id: String,
    repository_manifest: String,
}
#[derive(Deserialize)]
struct Catalog {
    depots: Vec<CatalogEntry>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEntry {
    dependency_id: String,
    #[serde(default)]
    readable_name: String,
    manifest: String,
    executable: Executable,
}
#[derive(Deserialize)]
struct Executable {
    path: String,
    arguments: String,
}

/// Resolve every requested dependency before transferring game or redistributable payloads.
pub fn resolve(ids: &[String], cancelled: impl Fn() -> bool) -> Result<Plan> {
    if ids.is_empty() {
        return Ok(Plan {
            version: 1,
            catalog_build: String::new(),
            entries: Vec::new(),
        });
    }
    let client = client()?;
    let index: Index = serde_json::from_slice(&fetch(&client, INDEX, &cancelled)?)
        .map_err(|_| anyhow::anyhow!("Invalid GOG dependency repository index"))?;
    let url = official_url(&index.repository_manifest)?;
    let identity = url.path().rsplit('/').next().unwrap_or_default();
    ensure!(
        url.path()
            .starts_with("/content-system/v2/dependencies/meta/")
            && valid_digest(identity),
        "Unexpected dependency catalog location"
    );
    let bytes = fetch(&client, url.as_str(), &cancelled)?;
    ensure!(
        format!("{:x}", md5::compute(&bytes)) == identity,
        "GOG dependency catalog checksum mismatch"
    );
    let catalog: Catalog = serde_json::from_slice(&inflate(&bytes, &cancelled)?)
        .map_err(|_| anyhow::anyhow!("Invalid GOG dependency catalog"))?;
    resolve_catalog(ids, index.build_id, catalog, |identity| {
        fetch(
            &client,
            &format!("{META}/{}/{}/{identity}", &identity[..2], &identity[2..4]),
            &cancelled,
        )
    })
}

fn resolve_catalog(
    ids: &[String],
    build: String,
    catalog: Catalog,
    mut manifest: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<Plan> {
    ensure!(
        ids.len() <= 128 && ids.iter().all(|id| valid_id(id)),
        "Invalid or excessive required GOG dependency IDs"
    );
    let mut plan = Plan {
        version: 1,
        catalog_build: build,
        entries: Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut errors = Vec::new();
    let mut frozen_bytes = 0_usize;
    for id in ids.iter().filter(|id| seen.insert(id.as_str())) {
        let matches = catalog
            .depots
            .iter()
            .filter(|entry| &entry.dependency_id == id)
            .collect::<Vec<_>>();
        let result = (|| {
            ensure!(matches.len() == 1, "missing or ambiguous catalog record");
            let entry = matches[0];
            ensure!(valid_digest(&entry.manifest), "invalid manifest identity");
            let path = entry.executable.path.replace('\\', "/");
            let args =
                crate::installation::depot_actions::windows_arguments(&entry.executable.arguments)?;
            let method = if path.is_empty() {
                ensure!(
                    args.is_empty(),
                    "game-local content has unexplained setup arguments"
                );
                Method::GameFiles
            } else {
                relative_path(&path)?;
                ensure!(
                    path.starts_with("__redist/"),
                    "unsupported game-local setup executable"
                );
                match Path::new(&path)
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                {
                    Some("exe") if id == "ISI" => Method::ScriptInterpreter { path, args },
                    Some("exe") => Method::Exe { path, args },
                    Some("msi") => Method::Msi { path, args },
                    _ => bail!("unsupported setup executable type"),
                }
            };
            let dependency = Dependency {
                id: id.clone(),
                name: if entry.readable_name.is_empty() {
                    id.clone()
                } else {
                    entry.readable_name.clone()
                },
                manifest_id: entry.manifest.clone(),
                manifest_bytes: manifest(&entry.manifest)?,
                method,
            };
            dependency.manifest()?;
            Ok(dependency)
        })();
        match result {
            Ok(dependency) => {
                frozen_bytes += serde_json::to_vec(&dependency)?.len();
                ensure!(
                    frozen_bytes <= MAX_PLAN,
                    "Required dependency metadata exceeds its frozen plan limit"
                );
                plan.entries.push(dependency);
            }
            Err(error)
                if error
                    .downcast_ref::<crate::download::depot::DepotCancelled>()
                    .is_some() =>
            {
                return Err(error);
            }
            Err(error) => errors.push(format!("{id}: {error:#}")),
        }
    }
    ensure!(
        errors.is_empty(),
        "GOG prerequisite preparation failed: {}. Choose an offline installer or retry preparation; no game files were downloaded",
        errors.join("; ")
    );
    plan.validate()?;
    Ok(plan)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn relative_path(value: &str) -> Result<&Path> {
    ensure!(
        !value.is_empty()
            && value.len() <= 1024
            && !value.contains(['\\', ':'])
            && !value.chars().any(char::is_control),
        "Unsafe dependency path"
    );
    let path = Path::new(value);
    ensure!(
        path.components()
            .all(|part| matches!(part, Component::Normal(_)))
            && value
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "Unsafe dependency path"
    );
    Ok(path)
}

fn validate_manifest(manifest: &DepotManifest, method: &Method) -> Result<()> {
    let mut names = HashSet::new();
    let mut components = HashMap::new();
    let mut files = HashSet::new();
    for entry in &manifest.entries {
        let path = match entry {
            DepotEntry::File(file) => {
                files.insert(file.path.as_str());
                &file.path
            }
            DepotEntry::Directory { path } => path,
            DepotEntry::Link { .. } => {
                bail!("Dependency manifests containing links are unsupported")
            }
        };
        relative_path(path)?;
        for prefix in Path::new(path)
            .ancestors()
            .filter(|prefix| !prefix.as_os_str().is_empty())
        {
            let prefix = prefix
                .to_str()
                .context("Invalid dependency path encoding")?;
            if let Some(previous) = components.insert(prefix.to_ascii_lowercase(), prefix) {
                ensure!(
                    previous == prefix,
                    "Dependency directory names collide on Windows"
                );
            }
        }
        ensure!(
            names.insert(path.to_ascii_lowercase()),
            "Dependency paths collide on Windows"
        );
        if matches!(method, Method::GameFiles) {
            ensure!(
                path.split('/').next() != Some("__redist")
                    && path.split('/').all(|part| !part.starts_with(".ludomere")),
                "Game dependency content overlaps a control directory"
            );
        } else {
            ensure!(
                path == "__redist" || path.starts_with("__redist/"),
                "Shared dependency content escapes __redist"
            );
        }
    }
    ensure!(!files.is_empty(), "Dependency contains no files");
    for file in &files {
        ensure!(
            !names
                .iter()
                .any(|name| name.starts_with(&format!("{}/", file.to_ascii_lowercase()))),
            "Dependency file overlaps a directory"
        );
    }
    if let Method::Exe { path, args }
    | Method::Msi { path, args }
    | Method::ScriptInterpreter { path, args } = method
    {
        ensure!(
            !matches!(method, Method::ScriptInterpreter { .. }) || args.is_empty(),
            "Unsupported GOG interpreter arguments; choose an offline installer instead"
        );
        relative_path(path)?;
        ensure!(
            files.contains(path.as_str()),
            "Dependency executable is absent from its verified manifest"
        );
        ensure!(
            args.len() <= 128
                && args.iter().all(|arg| arg.len() <= 4096
                    && !arg.chars().any(char::is_control)
                    && !arg.contains(['{', '}', '%'])),
            "Unsupported dependency setup arguments"
        );
        let extension = Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        ensure!(
            extension.eq_ignore_ascii_case(if matches!(method, Method::Msi { .. }) {
                "msi"
            } else {
                "exe"
            }),
            "Dependency method does not match its executable"
        );
    }
    Ok(())
}

fn official_url(url: &str) -> Result<reqwest::Url> {
    let url =
        reqwest::Url::parse(url).map_err(|_| anyhow::anyhow!("Invalid dependency service URL"))?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && url.fragment().is_none()
            && matches!(
                url.host_str(),
                Some(
                    "content-system.gog.com"
                        | "gog-cdn-fastly.gog.com"
                        | "gog-cdn.gog.com"
                        | "gog-cdn.gcdn.co"
                )
            ),
        "Dependency service returned an unsupported origin"
    );
    Ok(url)
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(crate::identity::USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 || official_url(attempt.url().as_str()).is_err() {
                attempt.error("Unsupported dependency redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}
fn check(cancelled: &impl Fn() -> bool) -> Result<()> {
    if cancelled() {
        return Err(crate::download::depot::DepotCancelled.into());
    }
    Ok(())
}
fn bounded(mut input: impl Read, limit: usize, cancelled: &impl Fn() -> bool) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        check(cancelled)?;
        let count = input
            .read(&mut buffer)
            .map_err(|_| anyhow::anyhow!("Reading dependency metadata failed"))?;
        if count == 0 {
            return Ok(bytes);
        }
        ensure!(
            bytes.len().saturating_add(count) <= limit,
            "Dependency metadata exceeds its safety limit"
        );
        bytes.extend_from_slice(&buffer[..count]);
    }
}
fn fetch(
    client: &reqwest::blocking::Client,
    url: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<u8>> {
    check(cancelled)?;
    let response = client
        .get(official_url(url)?)
        .send()
        .map_err(|_| anyhow::anyhow!("GOG dependency metadata request failed"))?
        .error_for_status()
        .map_err(|_| anyhow::anyhow!("GOG dependency metadata request was rejected"))?;
    bounded(response, MAX_METADATA, cancelled)
}
fn inflate(bytes: &[u8], cancelled: &impl Fn() -> bool) -> Result<Vec<u8>> {
    bounded(
        flate2::read::ZlibDecoder::new(bytes),
        MAX_METADATA,
        cancelled,
    )
}

fn cache_root() -> PathBuf {
    crate::identity::data_root().join("gog-dependencies")
}

/// Reverification is required before executing a previously cached dependency.
pub fn verify_cached(
    dependency: &Dependency,
    cancelled: impl Fn() -> bool,
) -> Result<Option<PathBuf>> {
    let manifest = dependency.manifest()?;
    let root = cache_root().join(dependency.identity());
    match open_directory(&root) {
        Ok(_) => {
            verify_files(&root, &manifest, &cancelled)?;
            verify_inventory(&root, &manifest)?;
            Ok(Some(root))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn acquire(
    plan: &Plan,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(u64, u64),
) -> Result<Vec<PreparedDependency>> {
    let client = client()?;
    let mut links = None;
    acquire_with(
        plan,
        &cache_root(),
        &cancelled,
        &mut progress,
        |chunk, output, progress| {
            if links.is_none() {
                links = Some(
                    serde_json::from_slice::<crate::download::depot::SecureLinks>(&fetch(
                        &client, LINKS, &cancelled,
                    )?)
                    .map_err(|_| anyhow::anyhow!("Invalid GOG dependency download endpoints"))?,
                );
            }
            let url = links
                .as_ref()
                .unwrap()
                .urls
                .iter()
                .filter_map(|endpoint| dependency_chunk_url(endpoint, &chunk.compressed_md5).ok())
                .next()
                .context("No supported official dependency download endpoint")?;
            crate::download::depot::download_chunk_to_with_progress(
                &client,
                url.as_str(),
                chunk,
                output,
                progress,
            )?;
            check(&cancelled)
        },
    )
}

fn dependency_chunk_url(
    endpoint: &crate::download::depot::SecureEndpoint,
    digest: &str,
) -> Result<reqwest::Url> {
    ensure!(valid_digest(digest), "Invalid dependency chunk digest");
    if !endpoint.parameters.is_empty() {
        return official_url(&crate::download::depot::chunk_url(endpoint, digest)?);
    }
    let mut url = official_url(&endpoint.url_format)?;
    ensure!(
        url.path().trim_end_matches('/') == "/content-system/v2/dependencies/store",
        "Unexpected dependency download directory"
    );
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid dependency download directory"))?
        .pop_if_empty()
        .extend([&digest[..2], &digest[2..4], digest]);
    Ok(url)
}

fn acquire_with(
    plan: &Plan,
    base: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(u64, u64),
    mut fetch_chunk: impl FnMut(
        &super::depot_manifest::DepotChunk,
        &mut dyn Write,
        &mut dyn FnMut(u64),
    ) -> Result<()>,
) -> Result<Vec<PreparedDependency>> {
    plan.validate()?;
    let _lock = loop {
        check(cancelled)?;
        match CACHE.try_lock() {
            Ok(lock) => break lock,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => bail!("Dependency cache is unavailable"),
        }
    };
    check(cancelled)?;
    ensure_private_directory(base)?;
    let total = plan.entries.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.manifest()?.totals()?.compressed)
            .context("Dependency size overflow")
    })?;
    let mut completed = 0_u64;
    let mut prepared = Vec::new();
    for dependency in &plan.entries {
        check(cancelled)?;
        let manifest = dependency.manifest()?;
        let root = base.join(dependency.identity());
        let present = match open_directory(&root) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if present
            && verify_files(&root, &manifest, cancelled)
                .and_then(|()| verify_inventory(&root, &manifest))
                .is_ok()
        {
            completed += manifest.totals()?.compressed;
            progress(completed, total);
            prepared.push(PreparedDependency {
                dependency: dependency.clone(),
                root,
            });
            continue;
        }
        check(cancelled)?;
        let staging = tempfile::Builder::new()
            .prefix(".acquire-")
            .tempdir_in(base)?;
        let target = staging.path().join("payload");
        let journal = staging.path().join("transfer.json");
        crate::download::depot::materialize_streamed_controlled(
            &manifest,
            &target,
            &journal,
            &HashSet::new(),
            |chunks, file, saved| {
                for (index, job) in chunks.iter().enumerate() {
                    check(cancelled)?;
                    let writer = crate::download::depot::FileRegionWriter::new(
                        file.try_clone()?,
                        job.offset,
                    );
                    let mut writer = CancelWriter {
                        inner: writer,
                        cancelled,
                    };
                    let mut received = 0_u64;
                    let result = fetch_chunk(job.chunk, &mut writer, &mut |bytes| {
                        received = received
                            .saturating_add(bytes)
                            .min(job.chunk.compressed_size);
                        progress(completed + received, total);
                    });
                    check(cancelled)?;
                    result?;
                    completed += job.chunk.compressed_size;
                    progress(completed, total);
                    saved(index)?;
                }
                Ok(())
            },
            cancelled,
        )?;
        verify_files(&target, &manifest, cancelled)?;
        verify_inventory(&target, &manifest)?;
        check(cancelled)?;
        let damaged = staging.path().join("previous");
        if present {
            fs::rename(&root, &damaged)?;
        }
        if let Err(error) = fs::rename(&target, &root) {
            if present && fs::rename(&damaged, &root).is_err() {
                return Err(error).with_context(|| {
                    format!(
                        "Dependency publication failed; previous cache retained at {}",
                        staging.keep().display()
                    )
                });
            }
            return Err(error.into());
        }
        File::open(base)?.sync_all()?;
        prepared.push(PreparedDependency {
            dependency: dependency.clone(),
            root,
        });
    }
    Ok(prepared)
}

struct CancelWriter<'a, W, C> {
    inner: W,
    cancelled: &'a C,
}
impl<W: Write, C: Fn() -> bool> Write for CancelWriter<'_, W, C> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            return Err(std::io::Error::other("Dependency acquisition cancelled"));
        }
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn open_directory(path: &Path) -> std::io::Result<File> {
    if !path.is_absolute() {
        return Err(std::io::Error::other(
            "Dependency cache path is not absolute",
        ));
    }
    let mut directory = File::open("/")?;
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => directory = open_child(&directory, name, true)?,
            _ => return Err(std::io::Error::other("Unsafe dependency cache path")),
        }
    }
    Ok(directory)
}
fn open_child(parent: &File, name: &std::ffi::OsStr, directory: bool) -> std::io::Result<File> {
    let name = std::ffi::CString::new(name.as_bytes())?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn open_file(root: &Path, path: &str) -> Result<File> {
    let relative = relative_path(path)?;
    let mut parent = open_directory(root)?;
    for part in relative.parent().unwrap().components() {
        parent = open_child(&parent, part.as_os_str(), true)?;
    }
    let file = open_child(&parent, relative.file_name().unwrap(), false)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.nlink() == 1 && metadata.uid() == unsafe { libc::geteuid() },
        "Unsafe cached dependency file"
    );
    Ok(file)
}
fn ensure_private_directory(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Dependency cache path is not absolute");
    let mut current = PathBuf::from("/");
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                current.push(name);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) => ensure!(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "Unsafe dependency cache directory"
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        fs::DirBuilder::new().mode(0o700).create(&current)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            _ => bail!("Unsafe dependency cache path"),
        }
    }
    let metadata = open_directory(path)?.metadata()?;
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o022 == 0,
        "Dependency cache directory is not privately owned"
    );
    Ok(())
}
fn verify_files(
    root: &Path,
    manifest: &DepotManifest,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    for entry in &manifest.entries {
        if let DepotEntry::File(file) = entry {
            check(cancelled)?;
            let mut input = open_file(root, &file.path)?;
            crate::download::depot::validate_open_file_with_progress(&mut input, file, |_| {
                check(cancelled)
            })?;
        }
    }
    Ok(())
}

fn verify_inventory(root: &Path, manifest: &DepotManifest) -> Result<()> {
    let mut expected = HashMap::new();
    for entry in &manifest.entries {
        let (path, directory) = match entry {
            DepotEntry::File(file) => (Path::new(&file.path), false),
            DepotEntry::Directory { path } => (Path::new(path), true),
            DepotEntry::Link { .. } => bail!("Unexpected dependency link"),
        };
        expected.insert(path.to_path_buf(), directory);
        for parent in path
            .ancestors()
            .skip(1)
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            expected.insert(parent.to_path_buf(), true);
        }
    }
    let mut pending = vec![(PathBuf::new(), open_directory(root)?)];
    while let Some((relative, directory)) = pending.pop() {
        for entry in fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))? {
            let name = entry?.file_name();
            let path = relative.join(&name);
            let is_directory = expected.remove(&path).context(
                "Unexpected file in verified dependency cache; acquire the dependency again",
            )?;
            let file = open_child(&directory, &name, is_directory)?;
            let metadata = file.metadata()?;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() }
                    && (is_directory || metadata.is_file() && metadata.nlink() == 1),
                "Unsafe dependency cache entry"
            );
            if is_directory {
                pending.push((path, file));
            }
        }
    }
    ensure!(expected.is_empty(), "Dependency cache is incomplete");
    Ok(())
}

/// Publish only declared game-local dependency files, never arbitrary cache directory contents.
pub fn publish_game_files(
    prepared: &PreparedDependency,
    destination: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    ensure!(
        prepared.dependency.method == Method::GameFiles,
        "Only game-local dependency content can be published into a game"
    );
    ensure!(
        prepared.root == cache_root().join(prepared.dependency.identity()),
        "Unexpected dependency cache root"
    );
    let mut manifest = prepared.dependency.manifest()?;
    verify_files(&prepared.root, &manifest, &cancelled)?;
    verify_inventory(&prepared.root, &manifest)?;
    let mut chunks = HashMap::new();
    for entry in &mut manifest.entries {
        if let DepotEntry::File(file) = entry {
            file.small_file = None;
            let mut offset = 0;
            for chunk in &file.chunks {
                chunks
                    .entry((chunk.md5.clone(), chunk.size))
                    .or_insert((file.path.clone(), offset));
                offset += chunk.size;
            }
        }
    }
    manifest.small_files_containers.clear();
    let journal = destination.join(format!(
        ".ludomere-dependency-{}.json",
        prepared.dependency.identity()
    ));
    crate::download::depot::materialize_streamed_controlled(
        &manifest,
        destination,
        &journal,
        &HashSet::new(),
        |jobs, output, saved| {
            for (index, job) in jobs.iter().enumerate() {
                check(&cancelled)?;
                let (path, offset) = chunks
                    .get(&(job.chunk.md5.clone(), job.chunk.size))
                    .context("Dependency chunk is missing from verified cache")?;
                let mut input = open_file(&prepared.root, path)?;
                input.seek(SeekFrom::Start(*offset))?;
                let writer =
                    crate::download::depot::FileRegionWriter::new(output.try_clone()?, job.offset);
                let mut writer = CancelWriter {
                    inner: writer,
                    cancelled: &cancelled,
                };
                let copied = std::io::copy(&mut input.take(job.chunk.size), &mut writer)?;
                ensure!(
                    copied == job.chunk.size,
                    "Cached dependency became incomplete"
                );
                saved(index)?;
            }
            Ok(())
        },
        &cancelled,
    )?;
    verify_files(destination, &manifest, &cancelled)?;
    crate::download::depot::finish_journal(&journal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn compressed(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn official_plain_dependency_endpoints_build_verified_chunk_urls() {
        // Selected non-secret fields of the official anonymous open_link response.
        let links: crate::download::depot::SecureLinks = serde_json::from_str(r#"{"urls":[
            {"url_format":"https://gog-cdn-fastly.gog.com/content-system/v2/dependencies/store","parameters":{}},
            {"url_format":"https://gog-cdn.gcdn.co/content-system/v2/dependencies/store","parameters":{}}
        ]}"#).unwrap();
        let digest = "1c2be02b2fc5e07355c68c7ef8116477";
        for endpoint in &links.urls {
            let url = dependency_chunk_url(endpoint, digest).unwrap();
            assert_eq!(
                url.path(),
                "/content-system/v2/dependencies/store/1c/2b/1c2be02b2fc5e07355c68c7ef8116477"
            );
            assert!(url.query().is_none());
        }
        let templated = crate::download::depot::SecureEndpoint {
            url_format: "https://gog-cdn-fastly.gog.com/{path}?token={token}".into(),
            parameters: HashMap::from([
                ("path".into(), "content-system/v2/dependencies/store".into()),
                ("token".into(), "synthetic".into()),
            ]),
        };
        let url = dependency_chunk_url(&templated, digest).unwrap();
        assert_eq!(
            url.path(),
            dependency_chunk_url(&links.urls[0], digest).unwrap().path()
        );
        assert_eq!(url.query(), Some("token=synthetic"));
        let plain_with_query = crate::download::depot::SecureEndpoint {
            url_format: "https://gog-cdn-fastly.gog.com/content-system/v2/dependencies/store/?token=synthetic".into(),
            parameters: HashMap::new(),
        };
        let plain_url = dependency_chunk_url(&plain_with_query, digest).unwrap();
        assert_eq!(plain_url.path(), url.path());
        assert_eq!(plain_url.query(), Some("token=synthetic"));
        for base in [
            "http://gog-cdn-fastly.gog.com/content-system/v2/dependencies/store",
            "https://untrusted.invalid/content-system/v2/dependencies/store",
            "https://gog-cdn-fastly.gog.com/unexpected",
            "https://gog-cdn-fastly.gog.com/content-system/v2/dependencies/store/{missing}",
        ] {
            let endpoint = crate::download::depot::SecureEndpoint {
                url_format: base.into(),
                parameters: HashMap::new(),
            };
            assert!(dependency_chunk_url(&endpoint, digest).is_err());
        }
        assert!(dependency_chunk_url(&links.urls[0], "../invalid").is_err());
    }

    fn fixture(path: &str, bytes: &[u8], small: bool) -> (Dependency, Vec<u8>) {
        let encoded = compressed(bytes);
        let chunk = serde_json::json!({"compressedMd5":format!("{:x}", md5::compute(&encoded)),"compressedSize":encoded.len(),"md5":format!("{:x}", md5::compute(bytes)),"size":bytes.len()});
        let mut file = serde_json::json!({"type":"DepotFile","path":path,"chunks":[chunk.clone()],"sha256":format!("{:x}",Sha256::digest(bytes))});
        let mut depot = serde_json::json!({"items":[]});
        if small {
            file["sfcRef"] = serde_json::json!({"offset":0,"size":bytes.len()});
            depot["smallFilesContainer"] = serde_json::json!({"chunks":[chunk]});
        }
        depot["items"] = serde_json::json!([file]);
        let manifest_bytes = compressed(
            &serde_json::to_vec(&serde_json::json!({"version":2,"depot":depot})).unwrap(),
        );
        (
            Dependency {
                id: "fixture".into(),
                name: "Inert fixture".into(),
                manifest_id: format!("{:x}", md5::compute(&manifest_bytes)),
                manifest_bytes,
                method: if path.starts_with("__redist/") {
                    Method::Exe {
                        path: path.into(),
                        args: vec!["/S".into()],
                    }
                } else {
                    Method::GameFiles
                },
            },
            encoded,
        )
    }

    fn plan(dependency: Dependency) -> Plan {
        Plan {
            version: 1,
            catalog_build: "59705672826648994".into(),
            entries: vec![dependency],
        }
    }

    #[test]
    fn resolves_exact_official_openal_metadata_and_rejects_unknowns_together() {
        // Official metadata e0b13c7ba5ef3712faf6dd2855857e45; no executable bytes.
        let hex = "789c8d524b4bc34010bef757943d8bddd7ecc39b209ef4e4514ac9ecce92609b846e1435e4bf9ba46029adc539ed7c7c8ff960fbc5721c16a96d3a76b7ece77586aa8e7679845e7fa169fa936d2686f2bd7e3b67fead382a9b5dbba79c293e47180d980812894b94290071ab008271c1527242186d2dbbf98fd94bf54da31b380f02ae28768750e375028d514aeb8541e75350842222a8c48d836ba1f910e5b8d7de5ca40d67e8fadc8fb545574eb76c367b8a55ee564d4bf5fdd3aa29b6559dbb5bfaa40b67b05c1612cc24449794b742472d88739f2c5042e1545056709e64c002d10aeb348e4db542724e140150cbe84c8a97dcbbaf762ac71ea6cff1586d899d708ecdd6f36b3858b00fdae7aaa947a55c0c3f20428136";
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let catalog = || {
            serde_json::from_str(r#"{"depots":[{"dependencyId":"openAL","readableName":"OpenAL","manifest":"e0b13c7ba5ef3712faf6dd2855857e45","executable":{"path":"__redist/openAL/oalinst.exe","arguments":"/S"}}]}"#).unwrap()
        };
        let resolved = resolve_catalog(
            &["openAL".into(), "openAL".into()],
            "59705672826648994".into(),
            catalog(),
            |_| Ok(bytes.clone()),
        )
        .unwrap();
        assert_eq!(resolved.entries.len(), 1);
        assert_eq!(
            resolved.entries[0].method,
            Method::Exe {
                path: "__redist/openAL/oalinst.exe".into(),
                args: vec!["/S".into()]
            }
        );
        assert_eq!(
            resolved.entries[0]
                .manifest()
                .unwrap()
                .totals()
                .unwrap()
                .uncompressed,
            809496
        );
        let failed = resolve_catalog(
            &["OpenAL".into(), "unknown".into()],
            "1".into(),
            catalog(),
            |_| unreachable!(),
        )
        .unwrap_err()
        .to_string();
        assert!(failed.contains("OpenAL:") && failed.contains("unknown:"));
        let mut tampered = resolved.entries[0].clone();
        tampered.manifest_bytes[0] ^= 1;
        assert!(tampered.manifest().is_err());
        assert!(resolve(&[], || false).unwrap().entries.is_empty());
    }

    #[test]
    fn preflight_types_paths_and_invocation_are_validated_before_acquisition() {
        for (id, path, args, expected) in [
            ("XNA", "__redist/XNA/setup.msi", "/qn", true),
            ("ISI", "__redist/ISI/scriptinterpreter.exe", "", true),
            (
                "ISI",
                "__redist/ISI/scriptinterpreter.exe",
                "/unsupported",
                false,
            ),
            ("DOSBox074", "", "", true),
            ("arbitrary", "game/setup.exe", "/S", false),
            ("unknown", "__redist/setup.bat", "", false),
            ("arguments", "", "/S", false),
        ] {
            let payload_path = if path.is_empty() {
                "DOSBOX/dosbox.exe"
            } else {
                path
            };
            let (dependency, _) = fixture(payload_path, b"inert", false);
            let catalog = Catalog {
                depots: vec![CatalogEntry {
                    dependency_id: id.into(),
                    readable_name: id.into(),
                    manifest: dependency.manifest_id.clone(),
                    executable: Executable {
                        path: path.into(),
                        arguments: args.into(),
                    },
                }],
            };
            assert_eq!(
                resolve_catalog(&[id.into()], "1".into(), catalog, |_| Ok(dependency
                    .manifest_bytes
                    .clone()))
                .is_ok(),
                expected,
                "{id}"
            );
        }
        for path in [
            "../escape",
            "C:/outside",
            ".ludomere/installation.json",
            "__redistWrong/setup.exe",
        ] {
            let (dependency, _) = fixture(path, b"inert", false);
            if path == "__redistWrong/setup.exe" {
                assert!(dependency.manifest().is_ok());
            } else {
                assert!(dependency.manifest().is_err(), "{path}");
            }
        }
        let (mut dependency, _) = fixture("__redist/setup.exe", b"inert", false);
        let identity = dependency.identity();
        dependency.method = Method::Exe {
            path: "__redist/absent.exe".into(),
            args: vec![],
        };
        assert_ne!(identity, dependency.identity());
        assert!(dependency.manifest().is_err());
        let (mut dependency, _) = fixture("DOSBOX/a", b"inert", false);
        let mut manifest = dependency.manifest().unwrap();
        let mut other = manifest.entries[0].clone();
        if let DepotEntry::File(file) = &mut other {
            file.path = "dosbox/b".into();
        }
        manifest.entries.push(other);
        dependency.manifest_bytes = compressed(manifest.canonical_json().unwrap().as_bytes());
        dependency.manifest_id = format!("{:x}", md5::compute(&dependency.manifest_bytes));
        assert!(dependency.manifest().is_err());
    }

    #[test]
    fn metadata_bounds_and_origins_reject_untrusted_or_cancelled_input() {
        assert!(bounded(&b"12345"[..], 4, &|| false).is_err());
        assert!(inflate(&compressed(&vec![b'a'; MAX_METADATA + 1]), &|| false).is_err());
        assert!(
            bounded(&b"a"[..], 4, &|| true)
                .unwrap_err()
                .is::<crate::download::depot::DepotCancelled>()
        );
        for url in [
            "http://gog-cdn-fastly.gog.com/file",
            "https://gog-cdn-fastly.gog.com.attacker.test/file",
            "https://user:secret@gog-cdn-fastly.gog.com/file",
            "https://gog-cdn-fastly.gog.com:444/file",
            "file:///tmp/payload",
        ] {
            assert!(official_url(url).is_err());
        }
        assert!(
            official_url("https://gog-cdn.gcdn.co/content-system/v2/dependencies/store/a").is_ok()
        );
    }

    #[test]
    fn accepts_pinned_official_msi_isi_and_game_local_manifests() {
        for (id, identity, bytes, path, arguments) in [
            (
                "XNA",
                "c8b208847566d2d0a3200616a6dee454",
                &include_bytes!("../../tests/fixtures/gog-dependencies/XNA.zlib")[..],
                "__redist/XNA/xnafx31_redist.msi",
                "/qn",
            ),
            (
                "ISI",
                "3b51748f2f2a4dd12c832c811616d556",
                &include_bytes!("../../tests/fixtures/gog-dependencies/ISI.zlib")[..],
                "__redist/ISI/scriptinterpreter.exe",
                "",
            ),
            (
                "DOSBox074",
                "51295899ad1d8a2eed9c9c1309380e25",
                &include_bytes!("../../tests/fixtures/gog-dependencies/DOSBox074.zlib")[..],
                "",
                "",
            ),
            (
                // Public metadata only, fetched 2026-10-02 from GOG's dependency
                // catalog build 59705672826648994; compressed bytes hash below.
                "language_setup",
                "39757bac2293fab156a465e52c23b552",
                &include_bytes!("../../tests/fixtures/gog-dependencies/language_setup.zlib")[..],
                "",
                "",
            ),
        ] {
            let catalog = Catalog {
                depots: vec![CatalogEntry {
                    dependency_id: id.into(),
                    readable_name: id.into(),
                    manifest: identity.into(),
                    executable: Executable {
                        path: path.into(),
                        arguments: arguments.into(),
                    },
                }],
            };
            let resolved =
                resolve_catalog(&[id.into()], "59705672826648994".into(), catalog, |_| {
                    Ok(bytes.to_vec())
                })
                .unwrap();
            assert_eq!(resolved.entries[0].manifest_id, identity);
            assert_eq!(resolved.entries[0].manifest_bytes, bytes);
            let restored: Plan =
                serde_json::from_slice(&serde_json::to_vec(&resolved).unwrap()).unwrap();
            restored.validate().unwrap();
            assert_eq!(restored, resolved);
            if id == "language_setup" {
                assert!(
                    super::super::depot_manifest::parse(&inflate(bytes, &|| false).unwrap())
                        .is_err()
                );
                let parsed = resolved.entries[0].manifest().unwrap();
                let DepotEntry::File(file) = &parsed.entries[0] else {
                    panic!("file expected");
                };
                assert_eq!(file.path, "language_setup.exe");
                assert_eq!(file.size, 6_216_288);
                assert_eq!(
                    file.sha256.as_deref(),
                    Some("12fa2e59d74549f1af662068c32063e7fb21aff090e9a5211e5051bf59c84d73")
                );
                let mut tampered = resolved.entries[0].clone();
                tampered.manifest_bytes[0] ^= 1;
                assert!(
                    tampered
                        .manifest()
                        .unwrap_err()
                        .to_string()
                        .contains("identity")
                );
            }
            assert_eq!(
                resolved.entries[0].manifest().unwrap().entries.len(),
                if id == "DOSBox074" { 14 } else { 1 }
            );
            match id {
                "XNA" => assert!(matches!(resolved.entries[0].method, Method::Msi { .. })),
                "ISI" => assert!(matches!(
                    resolved.entries[0].method,
                    Method::ScriptInterpreter { .. }
                )),
                _ => assert_eq!(resolved.entries[0].method, Method::GameFiles),
            }
        }
    }

    #[test]
    fn dependency_root_paths_retain_safety_checks_and_error_causes() {
        for path in [
            "/",
            "//server/file",
            "/\\server/file",
            "\\server\\file",
            "/C:/file",
            "C:/file",
            "/../file",
            "/dir/../../file",
            "/dir/./file",
            "/dir//file",
            "/bad\nname",
            "/.ludomere/control",
            "/__redist/file",
        ] {
            let (dependency, _) = fixture(path, b"inert", false);
            assert!(
                dependency.manifest().is_err(),
                "accepted unsafe path {path:?}"
            );
        }
        for method in [
            Method::Exe {
                path: "__redist/setup.exe".into(),
                args: Vec::new(),
            },
            Method::Msi {
                path: "__redist/setup.msi".into(),
                args: Vec::new(),
            },
        ] {
            let (mut dependency, _) = fixture("/__redist/setup.exe", b"inert", false);
            dependency.method = method;
            assert!(
                dependency.manifest().is_err(),
                "shared installers must retain strict wire paths"
            );
        }
        let (mut dependency, _) = fixture("/same", b"inert", false);
        let mut root: serde_json::Value =
            serde_json::from_slice(&inflate(&dependency.manifest_bytes, &|| false).unwrap())
                .unwrap();
        let mut duplicate = root["depot"]["items"][0].clone();
        duplicate["path"] = serde_json::json!("SAME");
        root["depot"]["items"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        dependency.manifest_bytes = compressed(&serde_json::to_vec(&root).unwrap());
        dependency.manifest_id = format!("{:x}", md5::compute(&dependency.manifest_bytes));
        assert!(
            dependency
                .manifest()
                .unwrap_err()
                .chain()
                .any(|error| error.to_string().contains("colliding"))
        );

        let (unsafe_dependency, _) = fixture("/../escape", b"inert", false);
        let error = resolve_catalog(
            std::slice::from_ref(&unsafe_dependency.id),
            "1".into(),
            Catalog {
                depots: vec![CatalogEntry {
                    dependency_id: unsafe_dependency.id.clone(),
                    readable_name: "Fixture".into(),
                    manifest: unsafe_dependency.manifest_id.clone(),
                    executable: Executable {
                        path: String::new(),
                        arguments: String::new(),
                    },
                }],
            },
            |_| Ok(unsafe_dependency.manifest_bytes.clone()),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("Invalid dependency manifest") && error.contains("unsafe depot path"),
            "{error}"
        );
    }

    #[test]
    fn cancelled_or_invalid_payload_never_publishes_and_symlink_cache_is_rejected() {
        let (dependency, _) = fixture("__redist/fixture/setup.exe", b"inert payload bytes", false);
        let plan = plan(dependency.clone());
        for cancel in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let cancelled = AtomicBool::new(false);
            let result = acquire_with(
                &plan,
                temp.path(),
                &|| cancelled.load(Ordering::Relaxed),
                &mut |_, _| {},
                |_, output, _| {
                    output.write_all(b"wrong payload bytes")?;
                    if cancel {
                        cancelled.store(true, Ordering::Relaxed);
                        output.write_all(b"never written")?;
                    }
                    Ok(())
                },
            );
            let error = result.unwrap_err();
            assert_eq!(error.is::<crate::download::depot::DepotCancelled>(), cancel);
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        }
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("sentinel"), b"untouched").unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join(dependency.identity()))
            .unwrap();
        assert!(
            acquire_with(
                &plan,
                temp.path(),
                &|| false,
                &mut |_, _| {},
                |_, _, _| panic!("unsafe cache fetched")
            )
            .is_err()
        );
        assert_eq!(
            fs::read(outside.path().join("sentinel")).unwrap(),
            b"untouched"
        );
    }

    #[test]
    fn cancelled_writer_and_cache_wait_return_without_publishing() {
        let cancelled = AtomicBool::new(false);
        let mut writer = CancelWriter {
            inner: Vec::new(),
            cancelled: &|| cancelled.load(Ordering::Relaxed),
        };
        writer.write_all(b"first").unwrap();
        cancelled.store(true, Ordering::Relaxed);
        assert_eq!(
            writer.write_all(b"next").unwrap_err().kind(),
            std::io::ErrorKind::Other
        );
        let held = CACHE.lock().unwrap();
        cancelled.store(false, Ordering::Relaxed);
        let temp = tempfile::tempdir().unwrap();
        let empty = Plan {
            version: 1,
            catalog_build: String::new(),
            entries: vec![],
        };
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                cancelled.store(true, Ordering::Relaxed);
            });
            assert!(
                acquire_with(
                    &empty,
                    temp.path(),
                    &|| cancelled.load(Ordering::Relaxed),
                    &mut |_, _| {},
                    |_, _, _| unreachable!()
                )
                .unwrap_err()
                .is::<crate::download::depot::DepotCancelled>()
            );
        });
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(held);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn verified_http_chunks_cache_reuse_repair_and_failed_transfer_are_atomic() {
        use std::net::TcpListener;
        let (dependency, encoded) =
            fixture("__redist/fixture/setup.exe", b"inert payload bytes", false);
        let plan = plan(dependency.clone());
        let temp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/chunk", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                encoded.len()
            )
            .unwrap();
            socket.write_all(&encoded).unwrap();
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let prepared = acquire_with(
            &plan,
            temp.path(),
            &|| false,
            &mut |_, _| {},
            |chunk, output, progress| {
                Ok(crate::download::depot::download_chunk_to_with_progress(
                    &client, &url, chunk, output, progress,
                )?)
            },
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(
            fs::read(prepared[0].root.join("__redist/fixture/setup.exe")).unwrap(),
            b"inert payload bytes"
        );
        acquire_with(&plan, temp.path(), &|| false, &mut |_, _| {}, |_, _, _| {
            panic!("intact cache fetched again")
        })
        .unwrap();
        fs::write(
            prepared[0].root.join("__redist/fixture/extra.dll"),
            b"unexpected",
        )
        .unwrap();
        assert!(verify_inventory(&prepared[0].root, &dependency.manifest().unwrap()).is_err());
        let failed = acquire_with(
            &plan,
            temp.path(),
            &|| false,
            &mut |_, _| {},
            |_, output, _| {
                output.write_all(b"bad")?;
                bail!("failed response")
            },
        );
        assert!(failed.is_err());
        assert!(prepared[0].root.join("__redist/fixture/setup.exe").exists());
        let mut progress = Vec::new();
        acquire_with(
            &plan,
            temp.path(),
            &|| false,
            &mut |done, _| progress.push(done),
            |_, output, report| {
                report(1);
                report(2);
                output.write_all(b"inert payload bytes")?;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(&progress[..2], &[1, 3]);
        assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(
            *progress.last().unwrap(),
            dependency.manifest().unwrap().totals().unwrap().compressed
        );
        assert!(!prepared[0].root.join("__redist/fixture/extra.dll").exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn sfc_game_local_files_publish_from_verified_cache_and_preserve_other_files() {
        let (dependency, _) = fixture("/DOSBOX/fixture.conf", b"inert configuration", true);
        let plan = plan(dependency.clone());
        let prepared = acquire_with(
            &plan,
            &cache_root(),
            &|| false,
            &mut |_, _| {},
            |_, output, _| {
                output.write_all(b"inert configuration")?;
                Ok(())
            },
        )
        .unwrap();
        let game = tempfile::tempdir().unwrap();
        fs::write(game.path().join("savedata"), b"preserved").unwrap();
        publish_game_files(&prepared[0], game.path(), || false).unwrap();
        assert_eq!(
            fs::read(game.path().join("DOSBOX/fixture.conf")).unwrap(),
            b"inert configuration"
        );
        assert_eq!(
            fs::read(game.path().join("savedata")).unwrap(),
            b"preserved"
        );
        assert!(verify_cached(&dependency, || false).unwrap().is_some());
        fs::write(prepared[0].root.join("DOSBOX/fixture.conf"), b"damaged").unwrap();
        assert!(verify_cached(&dependency, || false).is_err());
        assert!(publish_game_files(&prepared[0], game.path(), || false).is_err());
        fs::remove_dir_all(&prepared[0].root).unwrap();
    }
}
