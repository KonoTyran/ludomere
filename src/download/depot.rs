use crate::gog::depot_manifest::{DepotChunk, DepotEntry, DepotFile, DepotManifest};
use anyhow::{Context, Result, bail};
use flate2::{bufread::ZlibDecoder, read::ZlibDecoder as SliceZlibDecoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepotCancelled;

impl std::fmt::Display for DepotCancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("depot operation cancelled")
    }
}

impl std::error::Error for DepotCancelled {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferErrorKind {
    AuthenticationOrExpired,
    Transient,
    PermanentHttp,
    Integrity,
    DecodeOrManifest,
}

#[derive(Debug)]
pub struct TransferError {
    kind: TransferErrorKind,
    message: &'static str,
}

impl TransferError {
    pub const fn kind(&self) -> TransferErrorKind {
        self.kind
    }
    fn new(kind: TransferErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for TransferError {}

#[derive(Deserialize)]
pub struct SecureLinks {
    pub urls: Vec<SecureEndpoint>,
}

#[derive(Deserialize)]
pub struct SecureEndpoint {
    pub url_format: String,
    pub parameters: HashMap<String, SecureParameter>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub enum SecureParameter {
    String(String),
    Unsigned(u64),
    Signed(i64),
    Boolean(bool),
}

impl From<&str> for SecureParameter {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

impl std::fmt::Display for SecureParameter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(value) => formatter.write_str(value),
            Self::Unsigned(value) => value.fmt(formatter),
            Self::Signed(value) => value.fmt(formatter),
            Self::Boolean(value) => value.fmt(formatter),
        }
    }
}

impl std::fmt::Debug for SecureLinks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecureLinks")
            .field("endpoint_count", &self.urls.len())
            .finish()
    }
}

impl std::fmt::Debug for SecureEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecureEndpoint([redacted])")
    }
}

const JOURNAL_LIMIT: u64 = 4 * 1024 * 1024;
pub(crate) const TRANSFER_WORKERS: usize = 8;
const CHECKPOINT_CHUNKS: usize = 32;
const TRANSFER_BUFFER: usize = 512 * 1024;

pub(crate) struct ChunkWrite<'a> {
    pub chunk: &'a DepotChunk,
    pub offset: u64,
}

pub(crate) struct FileRegionWriter {
    file: File,
    offset: u64,
    written: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
}

impl FileRegionWriter {
    pub(crate) fn new(file: File, offset: u64) -> Self {
        Self {
            file,
            offset,
            written: None,
        }
    }

    pub(crate) fn new_counted(
        file: File,
        offset: u64,
        written: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            file,
            offset,
            written: Some(written),
        }
    }
}

impl Write for FileRegionWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        #[cfg(unix)]
        use std::os::unix::fs::FileExt;
        #[cfg(unix)]
        let written = self.file.write_at(buffer, self.offset)?;
        #[cfg(not(unix))]
        let written = {
            self.file.seek(SeekFrom::Start(self.offset))?;
            self.file.write(buffer)?
        };
        self.offset += written as u64;
        if let Some(total) = &self.written {
            total.fetch_add(written as u64, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
const JOURNAL: &str = ".test-depot-journal.json";

#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    version: u32,
    manifest_identity: String,
    #[serde(default)]
    container_chunks: Vec<Vec<JournalChunk>>,
    files: Vec<JournalFile>,
}

#[derive(Clone, Serialize, Deserialize)]
struct JournalFile {
    path: String,
    identity: String,
    chunks: Vec<JournalChunk>,
}

#[derive(Clone, Serialize, Deserialize)]
struct JournalChunk {
    index: usize,
    offset: u64,
    size: u64,
    md5: String,
}

pub fn acquire_secure_links(
    client: &reqwest::blocking::Client,
    access_token: &str,
    product_id: i64,
    root_path: &str,
) -> Result<SecureLinks> {
    acquire_secure_links_at(
        client,
        access_token,
        "https://content-system.gog.com",
        product_id,
        root_path,
    )
}

pub fn acquire_secure_links_at(
    client: &reqwest::blocking::Client,
    access_token: &str,
    base_url: &str,
    product_id: i64,
    root_path: &str,
) -> Result<SecureLinks> {
    let links: SecureLinks = client
        .get(format!("{base_url}/products/{product_id}/secure_link"))
        .bearer_auth(access_token)
        .query(&[("_version", "2"), ("generation", "2"), ("path", root_path)])
        .send()
        .map_err(|_| anyhow::anyhow!("requesting GOG secure links failed"))?
        .error_for_status()
        .map_err(|_| anyhow::anyhow!("GOG secure-link request was rejected"))?
        .json()
        .map_err(|_| anyhow::anyhow!("decoding GOG secure-link response failed"))?;
    if links.urls.is_empty() {
        bail!("GOG returned no secure download endpoints");
    }
    Ok(links)
}

pub fn chunk_url(endpoint: &SecureEndpoint, compressed_md5: &str) -> Result<String> {
    if compressed_md5.len() != 32 || !compressed_md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid compressed chunk digest");
    }
    let mut parameters = endpoint.parameters.clone();
    let path = match parameters.get_mut("path") {
        Some(SecureParameter::String(path)) => path,
        _ => bail!("secure endpoint has no string path parameter"),
    };
    path.push('/');
    path.push_str(&format!(
        "{}/{}/{}",
        &compressed_md5[..2],
        &compressed_md5[2..4],
        compressed_md5
    ));
    let mut url = endpoint.url_format.clone();
    for (name, value) in parameters {
        url = url.replace(&format!("{{{name}}}"), &value.to_string());
    }
    if url.contains('{') || url.contains('}') {
        bail!("secure endpoint contains unresolved parameters");
    }
    Ok(url)
}

pub fn download_chunk(
    client: &reqwest::blocking::Client,
    url: &str,
    chunk: &DepotChunk,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    download_chunk_to(client, url, chunk, &mut output)?;
    Ok(output)
}

pub fn download_chunk_to(
    client: &reqwest::blocking::Client,
    url: &str,
    chunk: &DepotChunk,
    output: &mut dyn Write,
) -> std::result::Result<(), TransferError> {
    download_chunk_to_with_progress(client, url, chunk, output, |_| {})
}

pub fn download_chunk_to_with_progress(
    client: &reqwest::blocking::Client,
    url: &str,
    chunk: &DepotChunk,
    output: &mut dyn Write,
    progress: impl FnMut(u64),
) -> std::result::Result<(), TransferError> {
    let response = client.get(url).send().map_err(|_| {
        TransferError::new(TransferErrorKind::Transient, "depot chunk transport failed")
    })?;
    let status = response.status();
    if !status.is_success() {
        let kind = match status.as_u16() {
            401 | 403 => TransferErrorKind::AuthenticationOrExpired,
            408 | 429 => TransferErrorKind::Transient,
            _ if status.is_server_error() => TransferErrorKind::Transient,
            _ => TransferErrorKind::PermanentHttp,
        };
        return Err(TransferError::new(kind, "depot chunk request was rejected"));
    }
    stream_chunk(response, chunk, output, progress)
}

#[cfg(test)]
fn read_compressed(reader: impl Read, expected: u64) -> Result<Vec<u8>> {
    let limit = expected
        .checked_add(1)
        .context("compressed chunk size overflows")?;
    let mut bytes = Vec::with_capacity(usize::try_from(expected.min(1024 * 1024)).unwrap());
    reader.take(limit).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != expected {
        bail!("compressed chunk size does not match its manifest");
    }
    Ok(bytes)
}

pub fn decode_chunk(compressed: &[u8], chunk: &DepotChunk) -> Result<Vec<u8>> {
    if compressed.len() as u64 != chunk.compressed_size {
        bail!("compressed chunk size does not match its manifest");
    }
    if format!("{:x}", md5::compute(compressed)) != chunk.compressed_md5 {
        bail!("compressed chunk checksum does not match its manifest");
    }
    let mut decoder = SliceZlibDecoder::new(compressed).take(chunk.size + 1);
    let mut expanded = Vec::new();
    decoder.read_to_end(&mut expanded)?;
    if expanded.len() as u64 != chunk.size {
        bail!("expanded chunk size does not match its manifest");
    }
    if format!("{:x}", md5::compute(&expanded)) != chunk.md5 {
        bail!("expanded chunk checksum does not match its manifest");
    }
    Ok(expanded)
}

struct BoundedHashReader<R, P> {
    inner: R,
    remaining: u64,
    read: u64,
    md5: md5::Context,
    progress: P,
}

impl<R: Read, P: FnMut(u64)> Read for BoundedHashReader<R, P> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let limit = usize::try_from(self.remaining.min(buffer.len() as u64)).unwrap();
        let read = self.inner.read(&mut buffer[..limit])?;
        super::bandwidth::acquire(read as u64, || false);
        self.remaining -= read as u64;
        self.read += read as u64;
        self.md5.consume(&buffer[..read]);
        (self.progress)(read as u64);
        Ok(read)
    }
}

fn stream_chunk(
    reader: impl Read,
    chunk: &DepotChunk,
    output: &mut dyn Write,
    progress: impl FnMut(u64),
) -> std::result::Result<(), TransferError> {
    let limit = chunk.compressed_size.checked_add(1).ok_or_else(|| {
        TransferError::new(
            TransferErrorKind::DecodeOrManifest,
            "compressed chunk size overflows",
        )
    })?;
    let source = BoundedHashReader {
        inner: reader,
        remaining: limit,
        read: 0,
        md5: md5::Context::new(),
        progress,
    };
    let buffered = BufReader::with_capacity(TRANSFER_BUFFER, source);
    let mut decoder = ZlibDecoder::new(buffered);
    let mut expanded_md5 = md5::Context::new();
    let mut expanded = 0_u64;
    let mut buffer = vec![0_u8; TRANSFER_BUFFER];
    loop {
        let read = decoder.read(&mut buffer).map_err(|_| {
            TransferError::new(
                TransferErrorKind::DecodeOrManifest,
                "depot chunk decompression failed",
            )
        })?;
        if read == 0 {
            break;
        }
        expanded = expanded.checked_add(read as u64).ok_or_else(|| {
            TransferError::new(
                TransferErrorKind::DecodeOrManifest,
                "expanded chunk size overflows",
            )
        })?;
        if expanded > chunk.size {
            return Err(TransferError::new(
                TransferErrorKind::Integrity,
                "expanded chunk exceeds manifest size",
            ));
        }
        output.write_all(&buffer[..read]).map_err(|_| {
            TransferError::new(TransferErrorKind::Transient, "writing depot chunk failed")
        })?;
        expanded_md5.consume(&buffer[..read]);
    }
    let mut buffered = decoder.into_inner();
    if buffered.read(&mut buffer).map_err(|_| {
        TransferError::new(TransferErrorKind::Transient, "reading depot chunk failed")
    })? != 0
    {
        return Err(TransferError::new(
            TransferErrorKind::Integrity,
            "compressed chunk contains trailing data",
        ));
    }
    let source = buffered.into_inner();
    if source.read != chunk.compressed_size || expanded != chunk.size {
        return Err(TransferError::new(
            TransferErrorKind::Integrity,
            "depot chunk size does not match manifest",
        ));
    }
    if format!("{:x}", source.md5.compute()) != chunk.compressed_md5
        || format!("{:x}", expanded_md5.compute()) != chunk.md5
    {
        return Err(TransferError::new(
            TransferErrorKind::Integrity,
            "depot chunk checksum does not match manifest",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn materialize<F>(manifest: &DepotManifest, root: &Path, mut fetch: F) -> Result<Vec<PathBuf>>
where
    F: FnMut(&DepotChunk) -> Result<Vec<u8>>,
{
    let journal = root.join(".test-depot-journal.json");
    let files = materialize_streamed_controlled(
        manifest,
        root,
        &journal,
        &HashSet::new(),
        |chunks, output, completed| {
            for (index, job) in chunks.iter().enumerate() {
                let mut writer = FileRegionWriter::new(output.try_clone()?, job.offset);
                writer.write_all(&fetch(job.chunk)?)?;
                completed(index)?;
            }
            Ok(())
        },
        || false,
    )?;
    finish_journal(&journal)?;
    Ok(files)
}

#[cfg(test)]
pub(crate) fn materialize_journaled<F, C>(
    manifest: &DepotManifest,
    root: &Path,
    mut fetch: F,
    mut cancelled: C,
) -> Result<Vec<PathBuf>>
where
    F: FnMut(&DepotChunk) -> Result<Vec<u8>>,
    C: FnMut() -> bool,
{
    let journal = root.join(".test-depot-journal.json");
    materialize_streamed_controlled(
        manifest,
        root,
        &journal,
        &HashSet::new(),
        |chunks, output, completed| {
            for (index, job) in chunks.iter().enumerate() {
                let mut writer = FileRegionWriter::new(output.try_clone()?, job.offset);
                writer.write_all(&fetch(job.chunk)?)?;
                completed(index)?;
            }
            Ok(())
        },
        &mut cancelled,
    )
}

pub(crate) fn materialize_streamed_controlled<F, C>(
    manifest: &DepotManifest,
    root: &Path,
    journal_path: &Path,
    trusted_files: &HashSet<String>,
    mut fetch: F,
    mut cancelled: C,
) -> Result<Vec<PathBuf>>
where
    F: FnMut(&[ChunkWrite<'_>], &File, &mut dyn FnMut(usize) -> Result<()>) -> Result<()>,
    C: FnMut() -> bool,
{
    manifest.totals()?;
    prepare_root(root, journal_path)?;
    let mut journal = if journal_path.exists() {
        load_journal(root, journal_path, manifest)?
    } else {
        Journal {
            version: 1,
            manifest_identity: manifest.identity(),
            container_chunks: Vec::new(),
            files: Vec::new(),
        }
    };
    write_journal(journal_path, &journal)?;
    let mut files = Vec::new();
    materialize_small_files(
        manifest,
        root,
        journal_path,
        trusted_files,
        &mut journal,
        &mut fetch,
        &mut cancelled,
    )?;
    for entry in &manifest.entries {
        if cancelled() {
            return Err(DepotCancelled.into());
        }
        match entry {
            DepotEntry::Directory { path } => safe_create_dir_all(&destination(root, path)?)?,
            DepotEntry::File(file) => {
                if file.small_file.is_some() {
                    files.push(destination(root, &file.path)?);
                    continue;
                }
                materialize_file(
                    root,
                    journal_path,
                    trusted_files,
                    file,
                    &mut journal,
                    &mut fetch,
                    &mut cancelled,
                )?;
                files.push(destination(root, &file.path)?);
            }
            DepotEntry::Link { .. } => {}
        }
    }
    for entry in &manifest.entries {
        if let DepotEntry::Link { path, target } = entry {
            let link = destination(root, path)?;
            if let Some(parent) = link.parent() {
                safe_create_dir_all(parent)?;
            }
            let temporary = part_path(&link)?;
            remove_non_directory(&temporary)?;
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(target, &temporary)?;
                if fs::symlink_metadata(&link)
                    .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                {
                    fs::remove_dir_all(&link)?;
                }
                fs::rename(temporary, link)?;
            }
            #[cfg(not(unix))]
            bail!("depot links are unsupported on this platform");
        }
    }
    Ok(files)
}

fn materialize_small_files<F, C>(
    manifest: &DepotManifest,
    root: &Path,
    journal_path: &Path,
    trusted_files: &HashSet<String>,
    journal: &mut Journal,
    fetch: &mut F,
    cancelled: &mut C,
) -> Result<()>
where
    F: FnMut(&[ChunkWrite<'_>], &File, &mut dyn FnMut(usize) -> Result<()>) -> Result<()>,
    C: FnMut() -> bool,
{
    for (container_index, container) in manifest.small_files_containers.iter().enumerate() {
        materialize_small_files_container(
            manifest,
            container_index,
            container,
            (root, journal_path, trusted_files),
            journal,
            fetch,
            cancelled,
        )?;
    }
    Ok(())
}

fn materialize_small_files_container<F, C>(
    manifest: &DepotManifest,
    container_index: usize,
    container: &crate::gog::depot_manifest::SmallFilesContainer,
    paths: (&Path, &Path, &HashSet<String>),
    journal: &mut Journal,
    fetch: &mut F,
    cancelled: &mut C,
) -> Result<()>
where
    F: FnMut(&[ChunkWrite<'_>], &File, &mut dyn FnMut(usize) -> Result<()>) -> Result<()>,
    C: FnMut() -> bool,
{
    let (root, journal_path, trusted_files) = paths;
    let files = manifest
        .entries
        .iter()
        .filter_map(|entry| match entry {
            DepotEntry::File(file)
                if file
                    .small_file
                    .is_some_and(|reference| reference.container_index == container_index) =>
            {
                Some(file)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if !files.is_empty() && files.iter().all(|file| trusted_files.contains(&file.path)) {
        return Ok(());
    }
    let container_path = root.join(format!(".ludomere-small-files-{container_index}.part"));
    reject_symlink(&container_path)?;
    let mut source = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&container_path)?;
    let pseudo = DepotFile {
        path: ".ludomere-small-files.part".into(),
        size: container
            .chunks
            .iter()
            .try_fold(0_u64, |sum, chunk| sum.checked_add(chunk.size))
            .context("small-files container size overflows")?,
        executable: false,
        support: false,
        md5: None,
        sha256: None,
        chunks: container.chunks.clone(),
        small_file: None,
    };
    while journal.container_chunks.len() <= container_index {
        journal.container_chunks.push(Vec::new());
    }
    let valid = validate_completed(
        &mut source,
        &pseudo,
        &JournalFile {
            path: pseudo.path.clone(),
            identity: file_identity(&pseudo),
            chunks: journal.container_chunks[container_index].clone(),
        },
    )
    .unwrap_or(0);
    journal.container_chunks[container_index].truncate(valid);
    source.set_len(
        journal.container_chunks[container_index]
            .last()
            .map_or(0, |chunk| chunk.offset + chunk.size),
    )?;
    let mut offset = journal.container_chunks[container_index]
        .last()
        .map_or(0, |chunk| chunk.offset + chunk.size);
    for batch_start in (valid..container.chunks.len()).step_by(CHECKPOINT_CHUNKS) {
        if cancelled() {
            return Err(DepotCancelled.into());
        }
        let batch_end = (batch_start + CHECKPOINT_CHUNKS).min(container.chunks.len());
        let batch = &container.chunks[batch_start..batch_end];
        let mut next_offset = offset;
        let jobs = batch
            .iter()
            .map(|chunk| {
                let job = ChunkWrite {
                    chunk,
                    offset: next_offset,
                };
                next_offset = next_offset
                    .checked_add(chunk.size)
                    .context("small-files container offsets overflow")?;
                Ok(job)
            })
            .collect::<Result<Vec<_>>>()?;
        fetch(&jobs, &source, &mut |_| Ok(()))?;
        for (relative, chunk) in batch.iter().enumerate() {
            let index = batch_start + relative;
            journal.container_chunks[container_index].push(JournalChunk {
                index,
                offset,
                size: chunk.size,
                md5: chunk.md5.clone(),
            });
            offset = offset
                .checked_add(chunk.size)
                .context("small-files container offsets overflow")?;
        }
        source.sync_data()?;
        write_journal(journal_path, journal)?;
        if cancelled() {
            return Err(DepotCancelled.into());
        }
    }
    for file in files {
        if cancelled() {
            return Err(DepotCancelled.into());
        }
        let reference = file.small_file.context("small-file reference is missing")?;
        let final_path = destination(root, &file.path)?;
        if let Some(parent) = final_path.parent() {
            safe_create_dir_all(parent)?;
        }
        let identity = file_identity(file);
        let index = journal
            .files
            .iter()
            .position(|saved| saved.path == file.path)
            .unwrap_or_else(|| {
                journal.files.push(JournalFile {
                    path: file.path.clone(),
                    identity: identity.clone(),
                    chunks: Vec::new(),
                });
                journal.files.len() - 1
            });
        if journal.files[index].identity != identity {
            journal.files[index] = JournalFile {
                path: file.path.clone(),
                identity,
                chunks: Vec::new(),
            };
        }
        if final_path.is_file()
            && (trusted_files.contains(&file.path)
                || validate_complete_file(&final_path, file).is_ok())
        {
            journal.files[index].chunks = completed_chunks(file)?;
            write_journal(journal_path, journal)?;
            continue;
        }
        let temporary = part_path(&final_path)?;
        reject_symlink(&temporary)?;
        let mut output = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        source.seek(SeekFrom::Start(reference.offset))?;
        copy_small_file_range(&mut source, &mut output, reference.size, cancelled)?;
        output.sync_all()?;
        validate_complete_file(&temporary, file)?;
        if fs::symlink_metadata(&final_path)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            fs::remove_dir_all(&final_path)?;
        }
        fs::rename(&temporary, &final_path)?;
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&final_path, fs::Permissions::from_mode(0o755))?;
        }
        journal.files[index].chunks = completed_chunks(file)?;
        write_journal(journal_path, journal)?;
    }
    fs::remove_file(container_path)?;
    journal.container_chunks[container_index].clear();
    write_journal(journal_path, journal)
}

fn copy_small_file_range<C>(
    source: &mut File,
    output: &mut File,
    mut remaining: u64,
    cancelled: &mut C,
) -> Result<()>
where
    C: FnMut() -> bool,
{
    let mut buffer = [0_u8; 64 * 1024];
    while remaining != 0 {
        if cancelled() {
            return Err(DepotCancelled.into());
        }
        let size = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
        source.read_exact(&mut buffer[..size])?;
        output.write_all(&buffer[..size])?;
        remaining -= size as u64;
    }
    Ok(())
}

pub(crate) fn finish_journal(journal_path: &Path) -> Result<()> {
    fs::remove_file(journal_path)?;
    sync_dir(
        journal_path
            .parent()
            .context("depot journal has no parent")?,
    )
}

pub(crate) fn abandon_materialization(
    manifest: &DepotManifest,
    root: &Path,
    journal_path: &Path,
) -> Result<()> {
    let journal = load_journal(root, journal_path, manifest)?;
    for saved in journal.files {
        remove_non_directory(&part_path(&destination(root, &saved.path)?)?)?;
    }
    for index in 0..journal.container_chunks.len() {
        remove_non_directory(&root.join(format!(".ludomere-small-files-{index}.part")))?;
    }
    finish_journal(journal_path)
}

pub(crate) fn journal_verification_total(
    manifest: &DepotManifest,
    root: &Path,
    journal_path: &Path,
) -> u64 {
    let Ok(journal) = load_journal(root, journal_path, manifest) else {
        return 0;
    };
    let files = manifest_file_index(manifest);
    journal
        .files
        .iter()
        .filter_map(|saved| files.get(saved.path.as_str()).copied())
        .filter(|file| file.small_file.is_none())
        .flat_map(|file| &file.chunks)
        .map(|chunk| chunk.compressed_size)
        .chain(
            journal
                .container_chunks
                .iter()
                .enumerate()
                .flat_map(|(index, saved)| {
                    saved.iter().filter_map(move |saved| {
                        manifest
                            .small_files_containers
                            .get(index)?
                            .chunks
                            .get(saved.index)
                            .map(|chunk| chunk.compressed_size)
                    })
                }),
        )
        .sum()
}

pub(crate) fn verify_installed_files<F, C>(
    manifest: &DepotManifest,
    root: &Path,
    mut checked: F,
    mut cancelled: C,
) -> Result<HashSet<String>>
where
    F: FnMut(u64),
    C: FnMut() -> bool,
{
    let files = manifest.entries.iter().filter_map(|entry| match entry {
        DepotEntry::File(file) => Some(file),
        _ => None,
    });
    verify_files_parallel(files, root, &mut checked, &mut cancelled)
}

pub(crate) fn verify_existing_files<S, F, C>(
    manifest: &DepotManifest,
    root: &Path,
    trusted: &HashSet<String>,
    mut started: S,
    mut checked: F,
    mut cancelled: C,
) -> Result<HashSet<String>>
where
    S: FnMut(u64),
    F: FnMut(u64),
    C: FnMut() -> bool,
{
    let files = manifest
        .entries
        .iter()
        .filter_map(|entry| match entry {
            DepotEntry::File(file) if !trusted.contains(&file.path) => Some(file),
            _ => None,
        })
        .filter(|file| {
            fs::symlink_metadata(root.join(&file.path))
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        })
        .collect::<Vec<_>>();
    let total = files.iter().try_fold(0_u64, |total, file| {
        total
            .checked_add(file.size)
            .context("existing-file verification size overflows")
    })?;
    started(total);
    verify_files_parallel(files, root, &mut checked, &mut cancelled)
}

fn verify_files_parallel<'a, I, F, C>(
    files: I,
    root: &Path,
    checked: &mut F,
    cancelled: &mut C,
) -> Result<HashSet<String>>
where
    I: IntoIterator<Item = &'a DepotFile>,
    F: FnMut(u64),
    C: FnMut() -> bool,
{
    enum Event {
        Progress(u64),
        Done(String, u64, u64, bool),
    }
    let jobs = files
        .into_iter()
        .map(|file| Ok((file, destination(root, &file.path)?)))
        .collect::<Result<std::collections::VecDeque<_>>>()?;
    let worker_count = jobs
        .len()
        .min(std::thread::available_parallelism().map_or(1, usize::from))
        .min(4);
    let queue = std::sync::Mutex::new(jobs);
    let stopped = std::sync::atomic::AtomicBool::new(false);
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut progress = 0_u64;
    let mut valid = HashSet::new();
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let queue = &queue;
            let stopped = &stopped;
            scope.spawn(move || {
                while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    let Some((file, path)) = queue.lock().unwrap().pop_front() else {
                        break;
                    };
                    let mut reported = 0_u64;
                    let result = validate_complete_file_with_progress(&path, file, |bytes| {
                        if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                            return Err(DepotCancelled.into());
                        }
                        reported = reported.saturating_add(bytes);
                        sender
                            .send(Event::Progress(bytes))
                            .map_err(|_| DepotCancelled)?;
                        Ok(())
                    });
                    if sender
                        .send(Event::Done(
                            file.path.clone(),
                            file.size,
                            reported,
                            result.is_ok(),
                        ))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        drop(sender);
        while let Ok(event) = receiver.recv() {
            if cancelled() {
                stopped.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            match event {
                Event::Progress(bytes) => progress = progress.saturating_add(bytes),
                Event::Done(path, size, reported, is_valid) => {
                    progress = progress.saturating_add(size.saturating_sub(reported));
                    if is_valid {
                        valid.insert(path);
                    }
                }
            }
            checked(progress);
        }
    });
    if stopped.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(DepotCancelled.into());
    }
    Ok(valid)
}

pub(crate) fn journal_progress<F>(
    manifest: &DepotManifest,
    root: &Path,
    journal_path: &Path,
    mut checked: F,
) -> (u64, HashSet<String>)
where
    F: FnMut(u64),
{
    let Ok(journal) = load_journal(root, journal_path, manifest) else {
        return (0, HashSet::new());
    };
    let files = manifest_file_index(manifest);
    let mut progress = 0_u64;
    let mut verified = 0_u64;
    let mut trusted = HashSet::new();
    for saved in &journal.files {
        let Some(file) = files
            .get(saved.path.as_str())
            .copied()
            .filter(|file| file.small_file.is_none())
        else {
            continue;
        };
        let expected = file.chunks.iter().map(|chunk| chunk.compressed_size).sum();
        verified = verified.saturating_add({
            let final_path = root.join(&file.path);
            if journal_file_complete(&final_path, file, saved) {
                trusted.insert(file.path.clone());
                expected
            } else {
                let valid = part_path(&root.join(&file.path))
                    .ok()
                    .and_then(|part| fs::OpenOptions::new().read(true).open(part).ok())
                    .and_then(|mut output| validate_completed(&mut output, file, saved).ok())
                    .unwrap_or(0);
                file.chunks
                    .iter()
                    .take(valid)
                    .map(|chunk| chunk.compressed_size)
                    .sum()
            }
        });
        progress = progress.saturating_add(expected);
        checked(progress);
    }
    for (index, container) in manifest.small_files_containers.iter().enumerate() {
        let expected = journal
            .container_chunks
            .get(index)
            .into_iter()
            .flatten()
            .filter_map(|saved| container.chunks.get(saved.index))
            .map(|chunk| chunk.compressed_size)
            .sum::<u64>();
        verified = verified.saturating_add({
            let complete = manifest
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    DepotEntry::File(file)
                        if file
                            .small_file
                            .is_some_and(|reference| reference.container_index == index) =>
                    {
                        Some(file)
                    }
                    _ => None,
                })
                .all(|file| {
                    journal
                        .files
                        .iter()
                        .find(|saved| saved.path == file.path)
                        .is_some_and(|saved| {
                            journal_file_complete(&root.join(&file.path), file, saved)
                        })
                });
            if complete {
                trusted.extend(manifest.entries.iter().filter_map(|entry| {
                    match entry {
                        DepotEntry::File(file)
                            if file
                                .small_file
                                .is_some_and(|reference| reference.container_index == index) =>
                        {
                            Some(file.path.clone())
                        }
                        _ => None,
                    }
                }));
                container
                    .chunks
                    .iter()
                    .map(|chunk| chunk.compressed_size)
                    .sum()
            } else {
                expected
            }
        });
        progress = progress.saturating_add(expected);
        checked(progress);
    }
    (verified, trusted)
}

pub(crate) fn pending_chunks(
    manifest: &DepotManifest,
    root: &Path,
    journal_path: &Path,
    trusted_files: &HashSet<String>,
) -> Result<Vec<crate::gog::depot_manifest::DepotChunk>> {
    let journal = journal_path
        .exists()
        .then(|| load_journal(root, journal_path, manifest))
        .transpose()?;
    let mut pending = Vec::new();
    for (index, container) in manifest.small_files_containers.iter().enumerate() {
        let all_trusted = manifest
            .entries
            .iter()
            .filter_map(|entry| match entry {
                DepotEntry::File(file)
                    if file
                        .small_file
                        .is_some_and(|reference| reference.container_index == index) =>
                {
                    Some(file)
                }
                _ => None,
            })
            .all(|file| trusted_files.contains(&file.path));
        if all_trusted {
            continue;
        }
        let valid = journal
            .as_ref()
            .and_then(|journal| journal.container_chunks.get(index))
            .map_or(0, |saved| {
                let pseudo = DepotFile {
                    path: ".ludomere-small-files.part".into(),
                    size: container.chunks.iter().map(|chunk| chunk.size).sum(),
                    executable: false,
                    support: false,
                    md5: None,
                    sha256: None,
                    chunks: container.chunks.clone(),
                    small_file: None,
                };
                fs::OpenOptions::new()
                    .read(true)
                    .open(root.join(format!(".ludomere-small-files-{index}.part")))
                    .ok()
                    .and_then(|mut output| {
                        validate_completed(
                            &mut output,
                            &pseudo,
                            &JournalFile {
                                path: pseudo.path.clone(),
                                identity: file_identity(&pseudo),
                                chunks: saved.clone(),
                            },
                        )
                        .ok()
                    })
                    .unwrap_or(0)
            });
        pending.extend(
            container.chunks[valid.min(container.chunks.len())..]
                .iter()
                .cloned(),
        );
    }
    for entry in &manifest.entries {
        let DepotEntry::File(file) = entry else {
            continue;
        };
        if file.small_file.is_some() || trusted_files.contains(&file.path) {
            continue;
        }
        let saved = journal
            .as_ref()
            .and_then(|journal| journal.files.iter().find(|saved| saved.path == file.path));
        let valid = saved
            .filter(|saved| saved.identity == file_identity(file))
            .and_then(|saved| {
                part_path(&root.join(&file.path))
                    .ok()
                    .and_then(|part| fs::OpenOptions::new().read(true).open(part).ok())
                    .and_then(|mut output| validate_completed(&mut output, file, saved).ok())
            })
            .unwrap_or(0);
        pending.extend(file.chunks[valid.min(file.chunks.len())..].iter().cloned());
    }
    Ok(pending)
}

#[allow(dead_code)] // consumed by the forward transaction-planning slice
pub(crate) fn journal_staged_bytes_at(
    manifest: &DepotManifest,
    root: &Path,
    journal_path: &Path,
) -> Result<u64> {
    if !journal_path.exists() {
        return Ok(0);
    }
    let journal = load_journal(root, journal_path, manifest)?;
    let mut total = 0_u64;
    for saved in &journal.files {
        let file = manifest
            .entries
            .iter()
            .find_map(|entry| match entry {
                DepotEntry::File(file) if file.path == saved.path => Some(file),
                _ => None,
            })
            .context("journal file is absent from manifest")?;
        let final_path = destination(root, &file.path)?;
        let staged = if journal_file_complete(&final_path, file, saved) {
            file.size
        } else if saved.chunks.is_empty() {
            0
        } else {
            let part = part_path(&final_path)?;
            let mut part = fs::OpenOptions::new()
                .read(true)
                .open(part)
                .context("journal references missing staged file data")?;
            let valid = validate_completed(&mut part, file, saved)?;
            if valid != saved.chunks.len() {
                bail!("journal references corrupt staged chunk data");
            }
            saved.chunks.iter().try_fold(0_u64, |sum, chunk| {
                sum.checked_add(chunk.size)
                    .context("staged byte count overflows")
            })?
        };
        total = checked_staged_add(total, staged)?;
    }
    Ok(total)
}

#[cfg(test)]
fn journal_staged_bytes(manifest: &DepotManifest, root: &Path) -> Result<u64> {
    journal_staged_bytes_at(manifest, root, &root.join(JOURNAL))
}

#[allow(dead_code)] // retained separately so overflow behavior remains directly testable
fn checked_staged_add(total: u64, staged: u64) -> Result<u64> {
    total
        .checked_add(staged)
        .context("staged byte count overflows")
}

fn materialize_file<F, C>(
    root: &Path,
    journal_path: &Path,
    trusted_files: &HashSet<String>,
    file: &DepotFile,
    journal: &mut Journal,
    fetch: &mut F,
    cancelled: &mut C,
) -> Result<()>
where
    F: FnMut(&[ChunkWrite<'_>], &File, &mut dyn FnMut(usize) -> Result<()>) -> Result<()>,
    C: FnMut() -> bool,
{
    let final_path = destination(root, &file.path)?;
    if let Some(parent) = final_path.parent() {
        safe_create_dir_all(parent)?;
    }
    let temporary = part_path(&final_path)?;
    reject_symlink(&temporary)?;
    let identity = file_identity(file);
    let position = journal
        .files
        .iter()
        .position(|saved| saved.path == file.path);
    if position.is_some_and(|index| journal.files[index].identity != identity) {
        fs::remove_file(&temporary).ok();
        journal.files.remove(position.unwrap());
    }
    let index = journal
        .files
        .iter()
        .position(|saved| saved.path == file.path)
        .unwrap_or_else(|| {
            journal.files.push(JournalFile {
                path: file.path.clone(),
                identity: identity.clone(),
                chunks: Vec::new(),
            });
            journal.files.len() - 1
        });
    if final_path.is_file() {
        if trusted_files.contains(&file.path) || validate_complete_file(&final_path, file).is_ok() {
            journal.files[index].chunks = completed_chunks(file)?;
            write_journal(journal_path, journal)?;
            return Ok(());
        }
        journal.files[index].chunks.clear();
    }
    let mut output = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&temporary)?;
    let valid = validate_completed(&mut output, file, &journal.files[index]).unwrap_or(0);
    journal.files[index].chunks.truncate(valid);
    output.set_len(
        journal.files[index]
            .chunks
            .last()
            .map_or(0, |chunk| chunk.offset + chunk.size),
    )?;
    let offset = journal.files[index]
        .chunks
        .last()
        .map_or(0, |chunk| chunk.offset + chunk.size);
    if cancelled() {
        return Err(DepotCancelled.into());
    }
    let mut next_offset = offset;
    let jobs = file.chunks[valid..]
        .iter()
        .map(|chunk| {
            let job = ChunkWrite {
                chunk,
                offset: next_offset,
            };
            next_offset = next_offset
                .checked_add(chunk.size)
                .context("file chunk offsets overflow")?;
            Ok(job)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut done = vec![false; jobs.len()];
    let mut contiguous = 0;
    let mut checkpointed = 0;
    fetch(&jobs, &output, &mut |relative| {
        let slot = done
            .get_mut(relative)
            .context("depot transfer reported an invalid chunk")?;
        *slot = true;
        while done.get(contiguous).copied() == Some(true) {
            contiguous += 1;
        }
        if contiguous - checkpointed >= CHECKPOINT_CHUNKS {
            append_journal_chunks(
                file,
                valid + checkpointed,
                valid + contiguous,
                journal,
                index,
            )?;
            checkpointed = contiguous;
            output.sync_data()?;
            write_journal(journal_path, journal)?;
        }
        Ok(())
    })?;
    if contiguous != jobs.len() {
        bail!("depot transfer did not complete every chunk");
    }
    if checkpointed != contiguous {
        append_journal_chunks(
            file,
            valid + checkpointed,
            valid + contiguous,
            journal,
            index,
        )?;
        output.sync_data()?;
        write_journal(journal_path, journal)?;
    }
    if cancelled() {
        return Err(DepotCancelled.into());
    }
    if fs::symlink_metadata(&final_path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        fs::remove_dir_all(&final_path)?;
    }
    fs::rename(&temporary, &final_path)?;
    #[cfg(unix)]
    if file.executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&final_path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn append_journal_chunks(
    file: &DepotFile,
    start: usize,
    end: usize,
    journal: &mut Journal,
    file_index: usize,
) -> Result<()> {
    let mut offset = file.chunks[..start]
        .iter()
        .try_fold(0_u64, |total, chunk| {
            total
                .checked_add(chunk.size)
                .context("file chunk offsets overflow")
        })?;
    for (chunk_index, chunk) in file.chunks[start..end].iter().enumerate() {
        journal.files[file_index].chunks.push(JournalChunk {
            index: start + chunk_index,
            offset,
            size: chunk.size,
            md5: chunk.md5.clone(),
        });
        offset = offset
            .checked_add(chunk.size)
            .context("file chunk offsets overflow")?;
    }
    Ok(())
}

fn file_identity(file: &DepotFile) -> String {
    let mut hash = Sha256::new();
    for value in [
        &file.path,
        file.md5.as_deref().unwrap_or(""),
        file.sha256.as_deref().unwrap_or(""),
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.update(file.size.to_be_bytes());
    hash.update([u8::from(file.executable)]);
    for chunk in &file.chunks {
        hash.update(chunk.compressed_md5.as_bytes());
        hash.update(chunk.compressed_size.to_be_bytes());
        hash.update(chunk.md5.as_bytes());
        hash.update(chunk.size.to_be_bytes());
    }
    format!("sha256:{:x}", hash.finalize())
}

fn completed_chunks(file: &DepotFile) -> Result<Vec<JournalChunk>> {
    let mut offset = 0;
    file.chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let completed = JournalChunk {
                index,
                offset,
                size: chunk.size,
                md5: chunk.md5.clone(),
            };
            offset = offset
                .checked_add(chunk.size)
                .context("file chunk offsets overflow")?;
            Ok(completed)
        })
        .collect()
}

fn validate_complete_file(path: &Path, file: &DepotFile) -> Result<()> {
    validate_complete_file_with_progress(path, file, |_| Ok(()))
}

fn validate_complete_file_with_progress<F>(
    path: &Path,
    file: &DepotFile,
    mut checked: F,
) -> Result<()>
where
    F: FnMut(u64) -> Result<()>,
{
    let mut input = File::open(path)?;
    if input.metadata()?.len() != file.size {
        bail!("staged file size does not match manifest");
    }
    let mut whole_md5 = file.md5.as_ref().map(|_| md5::Context::new());
    let mut whole_sha256 = file.sha256.as_ref().map(|_| Sha256::new());
    for chunk in &file.chunks {
        let mut remaining = chunk.size;
        let mut chunk_md5 = md5::Context::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        while remaining != 0 {
            let take = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
            input.read_exact(&mut buffer[..take])?;
            chunk_md5.consume(&buffer[..take]);
            if let Some(hash) = &mut whole_md5 {
                hash.consume(&buffer[..take]);
            }
            if let Some(hash) = &mut whole_sha256 {
                hash.update(&buffer[..take]);
            }
            remaining -= take as u64;
            checked(take as u64)?;
        }
        if format!("{:x}", chunk_md5.compute()) != chunk.md5 {
            bail!("staged file chunk does not match manifest");
        }
    }
    if file
        .md5
        .as_deref()
        .zip(whole_md5)
        .is_some_and(|(expected, hash)| format!("{:x}", hash.compute()) != *expected)
        || file
            .sha256
            .as_deref()
            .zip(whole_sha256)
            .is_some_and(|(expected, hash)| format!("{:x}", hash.finalize()) != expected)
    {
        bail!("staged file checksum does not match manifest");
    }
    Ok(())
}

fn validate_completed(output: &mut File, file: &DepotFile, saved: &JournalFile) -> Result<usize> {
    let mut expected_offset = 0_u64;
    let length = output.metadata()?.len();
    let mut fitting = 0;
    for (index, completed) in saved.chunks.iter().enumerate() {
        let Some(chunk) = file.chunks.get(index) else {
            bail!("journal chunk is out of bounds")
        };
        if completed.index != index
            || completed.offset != expected_offset
            || completed.size != chunk.size
            || completed.md5 != chunk.md5
        {
            bail!("journal chunk metadata does not match manifest");
        }
        expected_offset = expected_offset
            .checked_add(chunk.size)
            .context("journal offsets overflow")?;
        if expected_offset <= length {
            fitting = index + 1;
        }
    }
    if fitting != 0 {
        let completed = &saved.chunks[fitting - 1];
        if hash_region(output, completed.offset, completed.size)? != completed.md5 {
            fitting -= 1;
        }
    }
    Ok(fitting)
}

fn journal_file_complete(path: &Path, file: &DepotFile, saved: &JournalFile) -> bool {
    if saved.chunks.len() != file.chunks.len()
        || !fs::symlink_metadata(path).is_ok_and(|metadata| {
            metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() == file.size
        })
    {
        return false;
    }
    let mut offset = 0_u64;
    for (index, completed) in saved.chunks.iter().enumerate() {
        let Some(chunk) = file.chunks.get(index) else {
            return false;
        };
        if completed.index != index
            || completed.offset != offset
            || completed.size != chunk.size
            || completed.md5 != chunk.md5
        {
            return false;
        }
        let Some(next) = offset.checked_add(completed.size) else {
            return false;
        };
        offset = next;
    }
    offset == file.size
}

fn hash_region(file: &mut File, offset: u64, size: u64) -> Result<String> {
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = size;
    let mut hash = md5::Context::new();
    let mut buffer = [0_u8; 64 * 1024];
    while remaining != 0 {
        let take = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
        file.read_exact(&mut buffer[..take])?;
        hash.consume(&buffer[..take]);
        remaining -= take as u64;
    }
    Ok(format!("{:x}", hash.compute()))
}

fn load_journal(root: &Path, path: &Path, manifest: &DepotManifest) -> Result<Journal> {
    reject_symlink(path)?;
    let file = File::open(path)?;
    if file.metadata()?.len() > JOURNAL_LIMIT {
        bail!("depot journal exceeds size limit");
    }
    let journal: Journal = serde_json::from_reader(file)?;
    if journal.version != 1 || journal.manifest_identity != manifest.identity() {
        bail!("depot journal is incompatible");
    }
    let files = manifest_file_index(manifest);
    for saved in &journal.files {
        destination(root, &saved.path)?;
        let file = files
            .get(saved.path.as_str())
            .copied()
            .context("journal file is absent from manifest")?;
        if saved.identity != file_identity(file) {
            bail!("journal file identity mismatch");
        }
    }
    Ok(journal)
}

fn manifest_file_index(manifest: &DepotManifest) -> HashMap<&str, &DepotFile> {
    manifest
        .entries
        .iter()
        .filter_map(|entry| match entry {
            DepotEntry::File(file) => Some((file.path.as_str(), file)),
            _ => None,
        })
        .collect()
}

fn write_journal(path: &Path, journal: &Journal) -> Result<()> {
    let parent = path.parent().context("depot journal has no parent")?;
    let name = path.file_name().context("depot journal has no file name")?;
    let temporary = parent.join(format!("{}.tmp", name.to_string_lossy()));
    reject_symlink(path)?;
    reject_symlink(&temporary)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer(&mut file, journal)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    sync_dir(parent)
}

fn prepare_root(root: &Path, journal_path: &Path) -> Result<()> {
    reject_symlink_components(root)?;
    fs::create_dir_all(root)?;
    reject_symlink_components(root)?;
    let parent = journal_path
        .parent()
        .context("depot journal has no parent")?;
    reject_symlink_components(parent)?;
    fs::create_dir_all(parent)?;
    reject_symlink_components(parent)
}

fn safe_create_dir_all(path: &Path) -> Result<()> {
    reject_symlink_components(path)?;
    fs::create_dir_all(path)?;
    reject_symlink_components(path)
}

fn part_path(path: &Path) -> Result<PathBuf> {
    let name = path.file_name().context("depot file has no name")?;
    Ok(path.with_file_name(format!("{}.ludomere.part", name.to_string_lossy())))
}

fn remove_non_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            bail!("depot temporary path is a directory")
        }
        Ok(_) => fs::remove_file(path).map_err(Into::into),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("depot staging path is a symlink");
    }
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("depot path crosses a symlink");
        }
    }
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all().map_err(Into::into)
}

fn destination(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe depot output path");
    }
    Ok(root.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MeteredReader<R> {
        inner: R,
        max: std::rc::Rc<std::cell::Cell<usize>>,
    }
    impl<R: Read> Read for MeteredReader<R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.max.set(self.max.get().max(buffer.len()));
            self.inner.read(buffer)
        }
    }
    struct MeteredWriter {
        bytes: Vec<u8>,
        max: usize,
    }
    impl Write for MeteredWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.max = self.max.max(bytes.len());
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn encoded(value: &[u8]) -> (DepotChunk, Vec<u8>) {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(value).unwrap();
        let compressed = encoder.finish().unwrap();
        (
            DepotChunk {
                compressed_md5: format!("{:x}", md5::compute(&compressed)),
                compressed_size: compressed.len() as u64,
                md5: format!("{:x}", md5::compute(value)),
                size: value.len() as u64,
            },
            compressed,
        )
    }

    #[test]
    fn verifies_both_chunk_representations() {
        let (chunk, compressed) = encoded(b"payload");
        assert_eq!(decode_chunk(&compressed, &chunk).unwrap(), b"payload");
        let mut corrupt = compressed.clone();
        corrupt[0] ^= 1;
        assert!(decode_chunk(&corrupt, &chunk).is_err());
        assert!(read_compressed(std::io::Cursor::new(vec![0; 3]), 2).is_err());
        assert!(read_compressed(std::io::Cursor::new(vec![0; 1]), 2).is_err());
    }

    #[test]
    fn streams_large_chunks_with_fixed_buffers_and_strict_framing() {
        let expanded = vec![b'x'; 2 * 1024 * 1024];
        let (chunk, compressed) = encoded(&expanded);
        let max_read = std::rc::Rc::new(std::cell::Cell::new(0));
        let reader = MeteredReader {
            inner: std::io::Cursor::new(&compressed),
            max: max_read.clone(),
        };
        let mut writer = MeteredWriter {
            bytes: Vec::new(),
            max: 0,
        };
        let reported = std::cell::Cell::new(0_u64);
        stream_chunk(reader, &chunk, &mut writer, |bytes| {
            reported.set(reported.get() + bytes)
        })
        .unwrap();
        assert_eq!(writer.bytes, expanded);
        assert_eq!(reported.get(), chunk.compressed_size);
        assert!(max_read.get() <= TRANSFER_BUFFER);
        assert!(writer.max <= TRANSFER_BUFFER);

        let mut trailing = compressed.clone();
        trailing.push(0);
        let mut trailing_chunk = chunk.clone();
        trailing_chunk.compressed_size += 1;
        trailing_chunk.compressed_md5 = format!("{:x}", md5::compute(&trailing));
        assert_eq!(
            stream_chunk(
                std::io::Cursor::new(trailing),
                &trailing_chunk,
                &mut std::io::sink(),
                |_| {}
            )
            .unwrap_err()
            .kind(),
            TransferErrorKind::Integrity
        );
        assert!(
            stream_chunk(
                std::io::Cursor::new(&compressed[..compressed.len() - 1]),
                &chunk,
                &mut std::io::sink(),
                |_| {}
            )
            .is_err()
        );
        let mut short = chunk.clone();
        short.size -= 1;
        assert_eq!(
            stream_chunk(
                std::io::Cursor::new(compressed),
                &short,
                &mut std::io::sink(),
                |_| {}
            )
            .unwrap_err()
            .kind(),
            TransferErrorKind::Integrity
        );
    }

    #[test]
    fn classifies_http_failures_without_exposing_signed_urls() {
        use std::net::TcpListener;
        for (status, expected) in [
            (401, TransferErrorKind::AuthenticationOrExpired),
            (403, TransferErrorKind::AuthenticationOrExpired),
            (429, TransferErrorKind::Transient),
            (500, TransferErrorKind::Transient),
            (404, TransferErrorKind::PermanentHttp),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 1024];
                let _ = stream.read(&mut request);
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            });
            let chunk = DepotChunk {
                compressed_md5: "0".repeat(32),
                compressed_size: 1,
                md5: "0".repeat(32),
                size: 1,
            };
            let sentinel = "signed-token-sentinel";
            let error = download_chunk_to(
                &reqwest::blocking::Client::new(),
                &format!("http://{address}/chunk?token={sentinel}"),
                &chunk,
                &mut std::io::sink(),
            )
            .unwrap_err();
            assert_eq!(error.kind(), expected);
            assert!(!format!("{error:?} {error}").contains(sentinel));
        }
        let chunk = DepotChunk {
            compressed_md5: "0".repeat(32),
            compressed_size: 1,
            md5: "0".repeat(32),
            size: 1,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        assert_eq!(
            download_chunk_to(
                &reqwest::blocking::Client::new(),
                &format!("http://{address}/?token=secret"),
                &chunk,
                &mut std::io::sink()
            )
            .unwrap_err()
            .kind(),
            TransferErrorKind::Transient
        );
    }

    #[test]
    fn builds_hash_addressed_secure_url() {
        let endpoint = SecureEndpoint {
            url_format: "https://cdn.example/{path}?token={token}&dirs={dirs}&expires={expires_at}"
                .into(),
            parameters: HashMap::from([
                ("path".into(), "game".into()),
                ("token".into(), "redacted".into()),
                ("dirs".into(), SecureParameter::Unsigned(4)),
                ("expires_at".into(), SecureParameter::Unsigned(123)),
            ]),
        };
        assert_eq!(
            chunk_url(&endpoint, "0123456789abcdef0123456789abcdef").unwrap(),
            "https://cdn.example/game/01/23/0123456789abcdef0123456789abcdef?token=redacted&dirs=4&expires=123"
        );
    }

    #[test]
    fn materializes_files_and_permissions() {
        let (chunk, compressed) = encoded(b"payload");
        let file = DepotFile {
            small_file: None,
            path: "bin/game".into(),
            size: 7,
            executable: true,
            support: false,
            md5: Some(format!("{:x}", md5::compute(b"payload"))),
            sha256: Some(format!("{:x}", Sha256::digest(b"payload"))),
            chunks: vec![chunk],
        };
        let manifest = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![DepotEntry::File(file)],
        };
        let root = std::env::temp_dir().join(format!(
            "ludomere-depot-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        materialize(&manifest, &root, |chunk| decode_chunk(&compressed, chunk)).unwrap();
        assert_eq!(fs::read(root.join("bin/game")).unwrap(), b"payload");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materializes_small_files_from_multiple_verified_containers() {
        let (first, _) = encoded(b"info");
        let (second, _) = encoded(b"hashdb");
        let (overlapping, _) = encoded(b"foha");
        let (container_chunk, container_bytes) = encoded(b"infohashdb");
        let (third, _) = encoded(b"script");
        let (other_container, other_bytes) = encoded(b"script");
        let small_file =
            |path: &str, chunk: DepotChunk, container_index: usize, offset: u64| -> DepotEntry {
                DepotEntry::File(DepotFile {
                    small_file: Some(crate::gog::depot_manifest::SmallFileRef {
                        container_index,
                        offset,
                        size: chunk.size,
                    }),
                    path: path.into(),
                    size: chunk.size,
                    executable: false,
                    support: false,
                    md5: Some(chunk.md5.clone()),
                    sha256: None,
                    chunks: vec![chunk],
                })
            };
        let manifest = DepotManifest {
            small_files_containers: vec![
                crate::gog::depot_manifest::SmallFilesContainer {
                    chunks: vec![container_chunk.clone()],
                },
                crate::gog::depot_manifest::SmallFilesContainer {
                    chunks: vec![other_container.clone()],
                },
            ],
            generation: 2,
            entries: vec![
                small_file("goggame.info", first, 0, 0),
                small_file("goggame.hashdb", second, 0, 4),
                small_file("overlap.info", overlapping, 0, 2),
                small_file("goggame.script", third, 1, 0),
            ],
        };
        let root = std::env::temp_dir().join(format!(
            "ludomere-depot-small-files-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut fetched = 0;
        materialize(&manifest, &root, |chunk| {
            fetched += 1;
            decode_chunk(
                if chunk.md5 == container_chunk.md5 {
                    &container_bytes
                } else {
                    &other_bytes
                },
                chunk,
            )
        })
        .unwrap();
        assert_eq!(fetched, 2);
        assert_eq!(fs::read(root.join("goggame.info")).unwrap(), b"info");
        assert_eq!(fs::read(root.join("overlap.info")).unwrap(), b"foha");
        assert_eq!(fs::read(root.join("goggame.hashdb")).unwrap(), b"hashdb");
        assert_eq!(fs::read(root.join("goggame.script")).unwrap(), b"script");
        assert!(!root.join(".ludomere-small-files-0.part").exists());
        assert!(!root.join(".ludomere-small-files-1.part").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn journal_resumes_verified_chunks_and_rebuilds_corruption() {
        let (first, first_encoded) = encoded(b"first");
        let (second, second_encoded) = encoded(b"second");
        let manifest = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![DepotEntry::File(DepotFile {
                small_file: None,
                path: "game.dat".into(),
                size: 11,
                executable: false,
                support: false,
                md5: Some(format!("{:x}", md5::compute(b"firstsecond"))),
                sha256: None,
                chunks: vec![first.clone(), second.clone()],
            })],
        };
        let root = std::env::temp_dir().join(format!(
            "ludomere-depot-journal-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut checks = 0;
        let error = materialize_journaled(
            &manifest,
            &root,
            |chunk| {
                decode_chunk(
                    if chunk.md5 == first.md5 {
                        &first_encoded
                    } else {
                        &second_encoded
                    },
                    chunk,
                )
            },
            || {
                checks += 1;
                checks == 3
            },
        )
        .unwrap_err();
        assert!(error.downcast_ref::<DepotCancelled>().is_some());
        assert!(root.join(JOURNAL).is_file());
        assert!(root.join("game.dat.ludomere.part").is_file());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        assert_eq!(journal_staged_bytes(&manifest, &root).unwrap(), 11);
        assert!(
            pending_chunks(&manifest, &root, &root.join(JOURNAL), &HashSet::new())
                .unwrap()
                .is_empty()
        );

        let mut fetched = Vec::new();
        materialize_journaled(
            &manifest,
            &root,
            |chunk| {
                fetched.push(chunk.md5.clone());
                decode_chunk(
                    if chunk.md5 == first.md5 {
                        &first_encoded
                    } else {
                        &second_encoded
                    },
                    chunk,
                )
            },
            || false,
        )
        .unwrap();
        assert!(fetched.is_empty());
        assert_eq!(fs::read(root.join("game.dat")).unwrap(), b"firstsecond");
        assert_eq!(journal_staged_bytes(&manifest, &root).unwrap(), 11);
        finish_journal(&root.join(JOURNAL)).unwrap();
        assert!(!root.join(JOURNAL).exists());

        fs::remove_dir_all(&root).unwrap();
        let mut checks = 0;
        let _ = materialize_journaled(
            &manifest,
            &root,
            |chunk| {
                decode_chunk(
                    if chunk.md5 == first.md5 {
                        &first_encoded
                    } else {
                        &second_encoded
                    },
                    chunk,
                )
            },
            || {
                checks += 1;
                checks == 3
            },
        );
        let mut part = fs::OpenOptions::new()
            .write(true)
            .open(root.join("game.dat.ludomere.part"))
            .unwrap();
        part.seek(SeekFrom::Start(first.size)).unwrap();
        part.write_all(b"X").unwrap();
        drop(part);
        assert!(journal_staged_bytes(&manifest, &root).is_err());
        assert_eq!(
            pending_chunks(&manifest, &root, &root.join(JOURNAL), &HashSet::new()).unwrap(),
            vec![second.clone()]
        );
        let mut fetched = 0;
        materialize_journaled(
            &manifest,
            &root,
            |chunk| {
                fetched += 1;
                decode_chunk(
                    if chunk.md5 == first.md5 {
                        &first_encoded
                    } else {
                        &second_encoded
                    },
                    chunk,
                )
            },
            || false,
        )
        .unwrap();
        assert_eq!(fetched, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resume_reuses_a_completed_staged_file() {
        let (first, first_encoded) = encoded(b"first");
        let (second_a, second_a_encoded) = encoded(b"second-a");
        let (second_b, second_b_encoded) = encoded(b"second-b");
        let manifest = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![
                DepotEntry::File(DepotFile {
                    small_file: None,
                    path: "first.dat".into(),
                    size: 5,
                    executable: false,
                    support: false,
                    md5: Some(format!("{:x}", md5::compute(b"first"))),
                    sha256: None,
                    chunks: vec![first.clone()],
                }),
                DepotEntry::File(DepotFile {
                    small_file: None,
                    path: "second.dat".into(),
                    size: 16,
                    executable: false,
                    support: false,
                    md5: Some(format!("{:x}", md5::compute(b"second-asecond-b"))),
                    sha256: None,
                    chunks: vec![second_a.clone(), second_b.clone()],
                }),
            ],
        };
        let root = std::env::temp_dir().join(format!(
            "ludomere-depot-multifile-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bytes = |chunk: &DepotChunk| {
            if chunk.md5 == first.md5 {
                &first_encoded
            } else if chunk.md5 == second_a.md5 {
                &second_a_encoded
            } else {
                &second_b_encoded
            }
        };
        let mut checks = 0;
        let error = materialize_journaled(
            &manifest,
            &root,
            |chunk| decode_chunk(bytes(chunk), chunk),
            || {
                checks += 1;
                checks == 5
            },
        )
        .unwrap_err();
        assert!(error.downcast_ref::<DepotCancelled>().is_some());
        assert!(root.join("first.dat").is_file());
        assert_eq!(journal_staged_bytes(&manifest, &root).unwrap(), 5);

        let mut fetched = Vec::new();
        materialize_journaled(
            &manifest,
            &root,
            |chunk| {
                fetched.push(chunk.md5.clone());
                decode_chunk(bytes(chunk), chunk)
            },
            || false,
        )
        .unwrap();
        assert_eq!(fetched, [second_a.md5, second_b.md5]);
        assert_eq!(fs::read(root.join("first.dat")).unwrap(), b"first");
        assert_eq!(journal_staged_bytes(&manifest, &root).unwrap(), 21);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_byte_accounting_distinguishes_missing_corrupt_and_overflow() {
        let root = std::env::temp_dir().join(format!(
            "ludomere-depot-staged-bytes-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manifest = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: Vec::new(),
        };
        assert_eq!(journal_staged_bytes(&manifest, &root).unwrap(), 0);
        assert!(checked_staged_add(u64::MAX, 1).is_err());
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(JOURNAL), br#"{"version":99}"#).unwrap();
        assert!(journal_staged_bytes(&manifest, &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn journal_is_private_and_debug_redacts_secure_links() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "ludomere-depot-journal-mode-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manifest = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: Vec::new(),
        };
        let journal_path = root.join(JOURNAL);
        prepare_root(&root, &journal_path).unwrap();
        let temporary = root.join(format!("{JOURNAL}.tmp"));
        fs::write(&temporary, b"old").unwrap();
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o666)).unwrap();
        write_journal(
            &journal_path,
            &Journal {
                version: 1,
                manifest_identity: manifest.identity(),
                container_chunks: Vec::new(),
                files: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            fs::metadata(root.join(JOURNAL))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let links = SecureLinks {
            urls: vec![SecureEndpoint {
                url_format: "https://secret".into(),
                parameters: HashMap::from([("token".into(), "secret".into())]),
            }],
        };
        let debug = format!("{links:?}");
        assert!(!debug.contains("secret"));
        fs::remove_dir_all(root).unwrap();
    }
}
