use std::{collections::HashMap, io::Read};

use anyhow::{Context, Result, bail};
use flate2::read::ZlibDecoder;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const MAX_EXPANDED: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepotManifest {
    pub generation: u64,
    pub entries: Vec<DepotEntry>,
    pub small_files_containers: Vec<SmallFilesContainer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmallFilesContainer {
    pub chunks: Vec<DepotChunk>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepotEntry {
    Directory { path: String },
    File(DepotFile),
    Link { path: String, target: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepotFile {
    pub path: String,
    pub size: u64,
    pub executable: bool,
    pub support: bool,
    pub md5: Option<String>,
    pub sha256: Option<String>,
    pub chunks: Vec<DepotChunk>,
    pub small_file: Option<SmallFileRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmallFileRef {
    pub container_index: usize,
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepotChunk {
    pub compressed_md5: String,
    pub compressed_size: u64,
    pub md5: String,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepotTotals {
    pub compressed: u64,
    pub uncompressed: u64,
}

impl DepotManifest {
    pub fn canonical_json(&self) -> Result<String> {
        let chunks = |chunks: &[DepotChunk]| {
            chunks
                .iter()
                .map(|chunk| {
                    serde_json::json!({
                        "compressedMd5": chunk.compressed_md5,
                        "compressedSize": chunk.compressed_size,
                        "md5": chunk.md5,
                        "size": chunk.size,
                    })
                })
                .collect::<Vec<_>>()
        };
        let items = self
            .entries
            .iter()
            .map(|entry| {
                Ok(match entry {
                    DepotEntry::Directory { path } => {
                        serde_json::json!({"type":"DepotDirectory","path":path})
                    }
                    DepotEntry::Link { path, target } => {
                        serde_json::json!({"type":"DepotLink","path":path,"target":target})
                    }
                    DepotEntry::File(file) => {
                        let mut flags = Vec::new();
                        if file.executable {
                            flags.push("executable");
                        }
                        if file.support {
                            flags.push("support");
                        }
                        let mut value = serde_json::json!({
                            "type":"DepotFile","path":file.path,"flags":flags,
                            "chunks":chunks(&file.chunks),
                        });
                        let object = value.as_object_mut().unwrap();
                        if let Some(md5) = &file.md5 {
                            object.insert("md5".into(), serde_json::json!(md5));
                        }
                        if let Some(sha256) = &file.sha256 {
                            object.insert("sha256".into(), serde_json::json!(sha256));
                        }
                        if let Some(reference) = file.small_file {
                            if reference.container_index != 0 {
                                bail!(
                                    "wire depot manifest has an invalid small-files container index"
                                );
                            }
                            object.insert(
                            "sfcRef".into(),
                            serde_json::json!({"offset":reference.offset,"size":reference.size}),
                        );
                        }
                        value
                    }
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut depot = serde_json::json!({"items":items});
        if let Some(container) = self.small_files_containers.first() {
            depot.as_object_mut().unwrap().insert(
                "smallFilesContainer".into(),
                serde_json::json!({"chunks":chunks(&container.chunks)}),
            );
        }
        if self.small_files_containers.len() > 1 {
            bail!("wire depot manifest cannot encode multiple small-files containers");
        }
        Ok(serde_json::to_string(
            &serde_json::json!({"version":self.generation,"depot":depot}),
        )?)
    }

    pub fn split_support(&self) -> Result<(Self, Self)> {
        let payload =
            self.filtered(|entry| !matches!(entry, DepotEntry::File(file) if file.support))?;
        let support =
            self.filtered(|entry| matches!(entry, DepotEntry::File(file) if file.support))?;
        Ok((payload, support))
    }

    fn filtered(&self, keep: impl Fn(&DepotEntry) -> bool) -> Result<Self> {
        let mut entries = self
            .entries
            .iter()
            .filter(|entry| keep(entry))
            .cloned()
            .collect::<Vec<_>>();
        let used = entries
            .iter()
            .filter_map(|entry| match entry {
                DepotEntry::File(file) => {
                    file.small_file.map(|reference| reference.container_index)
                }
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        let remap = used
            .iter()
            .enumerate()
            .map(|(new, old)| (*old, new))
            .collect::<HashMap<_, _>>();
        let small_files_containers = self
            .small_files_containers
            .iter()
            .enumerate()
            .filter(|(index, _)| used.contains(index))
            .map(|(_, container)| container.clone())
            .collect();
        for entry in &mut entries {
            if let DepotEntry::File(file) = entry
                && let Some(reference) = &mut file.small_file
            {
                reference.container_index = *remap
                    .get(&reference.container_index)
                    .context("filtered manifest has an invalid small-files reference")?;
            }
        }
        Ok(Self {
            generation: self.generation,
            entries,
            small_files_containers,
        })
    }

    pub fn totals(&self) -> Result<DepotTotals> {
        if self.generation != 2 {
            bail!("unsupported depot manifest generation {}", self.generation);
        }
        let mut totals = DepotTotals {
            compressed: 0,
            uncompressed: 0,
        };
        let mut network_chunks = HashMap::new();
        for container in &self.small_files_containers {
            for chunk in &container.chunks {
                add_network_chunk(&mut totals, &mut network_chunks, chunk)?;
            }
        }
        for entry in &self.entries {
            let DepotEntry::File(file) = entry else {
                continue;
            };
            let mut file_size = 0_u64;
            for chunk in &file.chunks {
                if chunk.size == 0 || chunk.compressed_size == 0 {
                    bail!("chunk sizes must be nonzero");
                }
                file_size = file_size
                    .checked_add(chunk.size)
                    .context("file chunk sizes overflow")?;
                if file.small_file.is_none() {
                    add_network_chunk(&mut totals, &mut network_chunks, chunk)?;
                }
            }
            if file_size != file.size {
                bail!("file chunk total does not match file size");
            }
            totals.uncompressed = totals
                .uncompressed
                .checked_add(file.size)
                .context("manifest uncompressed size overflows")?;
        }
        Ok(totals)
    }

    pub fn identity(&self) -> String {
        fn field(hash: &mut Sha256, bytes: &[u8]) {
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        fn text(hash: &mut Sha256, value: &str) {
            field(hash, value.as_bytes());
        }
        fn optional(hash: &mut Sha256, value: Option<&str>) {
            hash.update([u8::from(value.is_some())]);
            if let Some(value) = value {
                text(hash, value);
            }
        }

        let mut hash = Sha256::new();
        hash.update(self.generation.to_be_bytes());
        hash.update((self.entries.len() as u64).to_be_bytes());
        for entry in &self.entries {
            match entry {
                DepotEntry::Directory { path } => {
                    hash.update([0]);
                    text(&mut hash, path);
                }
                DepotEntry::File(file) => {
                    hash.update([1]);
                    text(&mut hash, &file.path);
                    hash.update(file.size.to_be_bytes());
                    hash.update([u8::from(file.executable)]);
                    hash.update([u8::from(file.support)]);
                    optional(&mut hash, file.md5.as_deref());
                    optional(&mut hash, file.sha256.as_deref());
                    hash.update((file.chunks.len() as u64).to_be_bytes());
                    for chunk in &file.chunks {
                        text(&mut hash, &chunk.compressed_md5);
                        hash.update(chunk.compressed_size.to_be_bytes());
                        text(&mut hash, &chunk.md5);
                        hash.update(chunk.size.to_be_bytes());
                    }
                    hash.update([u8::from(file.small_file.is_some())]);
                    if let Some(reference) = file.small_file {
                        hash.update(reference.offset.to_be_bytes());
                        hash.update(reference.size.to_be_bytes());
                    }
                }
                DepotEntry::Link { path, target } => {
                    hash.update([2]);
                    text(&mut hash, path);
                    text(&mut hash, target);
                }
            }
        }
        hash.update((self.small_files_containers.len() as u64).to_be_bytes());
        for container in &self.small_files_containers {
            hash.update((container.chunks.len() as u64).to_be_bytes());
            for chunk in &container.chunks {
                text(&mut hash, &chunk.compressed_md5);
                hash.update(chunk.compressed_size.to_be_bytes());
                text(&mut hash, &chunk.md5);
                hash.update(chunk.size.to_be_bytes());
            }
        }
        format!("sha256:{:x}", hash.finalize())
    }
}

fn add_network_chunk<'a>(
    totals: &mut DepotTotals,
    chunks: &mut HashMap<&'a str, (u64, &'a str, u64)>,
    chunk: &'a DepotChunk,
) -> Result<()> {
    let metadata = (chunk.compressed_size, chunk.md5.as_str(), chunk.size);
    match chunks.insert(chunk.compressed_md5.as_str(), metadata) {
        Some(previous) if previous != metadata => {
            bail!("manifest reuses a compressed chunk hash with inconsistent metadata")
        }
        Some(_) => {}
        None => {
            totals.compressed = totals
                .compressed
                .checked_add(chunk.compressed_size)
                .context("manifest compressed size overflows")?;
        }
    }
    Ok(())
}

pub fn parse(bytes: &[u8]) -> Result<DepotManifest> {
    let json = if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'{') {
        if bytes.len() as u64 > MAX_EXPANDED {
            bail!("depot manifest exceeds 64 MiB");
        }
        bytes.to_vec()
    } else {
        let mut decoder = ZlibDecoder::new(bytes).take(MAX_EXPANDED + 1);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .context("decompress depot manifest")?;
        if out.len() as u64 > MAX_EXPANDED {
            bail!("expanded depot manifest exceeds 64 MiB");
        }
        out
    };
    let root: Value = serde_json::from_slice(&json).context("parse depot manifest JSON")?;
    let manifest = parse_value(&root)?;
    manifest.totals()?;
    Ok(manifest)
}

fn parse_value(root: &Value) -> Result<DepotManifest> {
    let generation = number(root, "version")?;
    if generation != 2 {
        bail!("unsupported depot manifest generation {generation}");
    }
    let depot = root.get("depot").context("depot manifest has no depot")?;
    let entries = depot
        .get("items")
        .and_then(Value::as_array)
        .context("depot manifest has no entries array")?;
    let mut parsed = Vec::with_capacity(entries.len());
    let mut paths = HashMap::new();
    for entry in entries {
        let object = entry.as_object().context("depot entry is not an object")?;
        let kind = string(object, "type")?;
        let raw_path = string(object, "path")?;
        let path = normalize_path(if kind == "DepotDirectory" {
            raw_path.trim_end_matches(['/', '\\'])
        } else {
            raw_path
        })?;
        let folded = path.to_ascii_lowercase();
        if let Some(previous) = paths.insert(folded, path.clone()) {
            bail!("colliding depot paths {previous:?} and {path:?}");
        }
        parsed.push(match kind {
            "DepotDirectory" => DepotEntry::Directory { path },
            "DepotLink" => {
                let target = normalize_link(&path, string(object, "target")?)?;
                DepotEntry::Link { path, target }
            }
            "DepotFile" => DepotEntry::File(parse_file(object, &path)?),
            kind => bail!("unsupported depot entry kind {kind:?}"),
        });
    }
    let small_files_containers = depot
        .get("smallFilesContainer")
        .map(parse_small_files_container)
        .transpose()?;
    let small_files_containers = small_files_containers.into_iter().collect::<Vec<_>>();
    validate_small_file_refs(&parsed, &small_files_containers)?;
    Ok(DepotManifest {
        generation,
        entries: parsed,
        small_files_containers,
    })
}

fn parse_file(object: &Map<String, Value>, path: &str) -> Result<DepotFile> {
    let flags = object
        .get("flags")
        .map(|flags| flags.as_array().context("file flags are not an array"))
        .transpose()?;
    let has_flag =
        |name| flags.is_some_and(|flags| flags.iter().any(|flag| flag.as_str() == Some(name)));
    let executable = has_flag("executable");
    let support = has_flag("support");
    let md5 = optional_digest(object, "md5", 32)?;
    let sha256 = optional_digest(object, "sha256", 64)?;
    let raw_chunks = object
        .get("chunks")
        .and_then(Value::as_array)
        .context("file has no chunks array")?;
    let mut chunks = Vec::with_capacity(raw_chunks.len());
    let mut total = 0u64;
    for raw in raw_chunks {
        let chunk = raw.as_object().context("chunk is not an object")?;
        let size = number_map(chunk, "size")?;
        let compressed_size = number_map(chunk, "compressedSize")?;
        if size == 0 || compressed_size == 0 {
            bail!("chunk sizes must be nonzero");
        }
        total = total.checked_add(size).context("chunk sizes overflow")?;
        chunks.push(DepotChunk {
            compressed_md5: digest(chunk, "compressedMd5", 32)?,
            compressed_size,
            md5: digest(chunk, "md5", 32)?,
            size,
        });
    }
    let size = total;
    let small_file = object
        .get("sfcRef")
        .map(|value| {
            let reference = value.as_object().context("sfcRef is not an object")?;
            let offset = number_map(reference, "offset")?;
            let size = number_map(reference, "size")?;
            if size == 0 || size != total {
                bail!("sfcRef size does not match file size");
            }
            Ok(SmallFileRef {
                container_index: 0,
                offset,
                size,
            })
        })
        .transpose()?;
    Ok(DepotFile {
        path: path.into(),
        size,
        executable,
        support,
        md5,
        sha256,
        chunks,
        small_file,
    })
}

fn parse_small_files_container(value: &Value) -> Result<SmallFilesContainer> {
    let chunks = value
        .get("chunks")
        .and_then(Value::as_array)
        .context("smallFilesContainer has no chunks array")?
        .iter()
        .map(|value| parse_chunk(value.as_object().context("chunk is not an object")?))
        .collect::<Result<Vec<_>>>()?;
    if chunks.is_empty() {
        bail!("smallFilesContainer has no chunks");
    }
    Ok(SmallFilesContainer { chunks })
}

fn parse_chunk(chunk: &Map<String, Value>) -> Result<DepotChunk> {
    let size = number_map(chunk, "size")?;
    let compressed_size = number_map(chunk, "compressedSize")?;
    if size == 0 || compressed_size == 0 {
        bail!("chunk sizes must be nonzero");
    }
    Ok(DepotChunk {
        compressed_md5: digest(chunk, "compressedMd5", 32)?,
        compressed_size,
        md5: digest(chunk, "md5", 32)?,
        size,
    })
}

fn validate_small_file_refs(
    entries: &[DepotEntry],
    containers: &[SmallFilesContainer],
) -> Result<()> {
    let container_sizes = containers
        .iter()
        .map(|container| {
            container.chunks.iter().try_fold(0_u64, |sum, chunk| {
                sum.checked_add(chunk.size)
                    .context("smallFilesContainer size overflows")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for file in entries.iter().filter_map(|entry| match entry {
        DepotEntry::File(file) => Some(file),
        _ => None,
    }) {
        let Some(reference) = file.small_file else {
            continue;
        };
        let end = reference
            .offset
            .checked_add(reference.size)
            .context("sfcRef range overflows")?;
        if container_sizes
            .get(reference.container_index)
            .is_none_or(|size| end > *size)
        {
            bail!("sfcRef is outside smallFilesContainer");
        }
    }
    Ok(())
}

fn normalize_path(path: &str) -> Result<String> {
    if path.is_empty()
        || path.contains('\0')
        || path.starts_with(['/', '\\'])
        || path.as_bytes().get(1) == Some(&b':')
    {
        bail!("unsafe depot path {path:?}");
    }
    let path = path.replace('\\', "/");
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            bail!("unsafe depot path {path:?}");
        }
    }
    Ok(path)
}

fn normalize_link(path: &str, target: &str) -> Result<String> {
    if target.is_empty()
        || target.contains('\0')
        || target.starts_with(['/', '\\'])
        || target.as_bytes().get(1) == Some(&b':')
    {
        bail!("unsafe link target {target:?}");
    }
    let target = target.replace('\\', "/");
    let mut depth = path.split('/').count() - 1;
    for part in target.split('/') {
        match part {
            "" | "." => bail!("unsafe link target {target:?}"),
            ".." if depth == 0 => bail!("link target escapes installation root"),
            ".." => depth -= 1,
            _ => depth += 1,
        }
    }
    Ok(target)
}

pub(crate) fn validate_link(path: &str, target: &str) -> Result<()> {
    normalize_path(path)?;
    let normalized = normalize_link(path, target)?;
    if normalized != target {
        bail!("link target is not normalized");
    }
    Ok(())
}

fn number(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("missing or invalid {key}"))
}

fn number_map(value: &Map<String, Value>, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("missing or invalid {key}"))
}

fn string<'a>(value: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing or invalid {key}"))
}

fn optional_digest(value: &Map<String, Value>, key: &str, len: usize) -> Result<Option<String>> {
    value.get(key).map(|_| digest(value, key, len)).transpose()
}

fn digest(value: &Map<String, Value>, key: &str, len: usize) -> Result<String> {
    let digest = string(value, key)?;
    if digest.len() != len || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid {key} digest");
    }
    Ok(digest.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    const MD5: &str = "0123456789abcdef0123456789abcdef";
    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn manifest(entries: &str) -> Vec<u8> {
        format!(r#"{{"version":2,"depot":{{"items":[{entries}]}}}}"#).into_bytes()
    }

    fn file(path: &str, chunks: &str) -> String {
        format!(r#"{{"type":"DepotFile","path":"{path}","flags":[],"chunks":[{chunks}]}}"#)
    }

    fn chunk(size: u64) -> String {
        let digest = format!("{size:032x}");
        format!(
            r#"{{"compressedMd5":"{digest}","compressedSize":{size},"md5":"{digest}","size":{size}}}"#
        )
    }

    #[test]
    fn parses_raw_and_zlib_with_ordered_chunks_and_metadata() {
        let file = format!(
            r#"{{"type":"DepotFile","path":"bin\\game","flags":["executable","support"],"md5":"{MD5}","sha256":"{SHA}","chunks":[{},{}]}}"#,
            chunk(1),
            chunk(2)
        );
        let raw = manifest(&format!(
            r#"{{"type":"DepotDirectory","path":"bin\\"}},{file},{{"type":"DepotFile","path":"empty","chunks":[]}},{{"type":"DepotLink","path":"current","target":"bin\\game"}}"#
        ));
        let parsed = parse(&raw).unwrap();
        let DepotEntry::File(file) = &parsed.entries[1] else {
            panic!()
        };
        assert!(file.executable);
        assert!(file.support);
        assert_eq!(
            file.chunks
                .iter()
                .map(|chunk| chunk.size)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(file.md5.as_deref(), Some(MD5));
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw).unwrap();
        let compressed = parse(&encoder.finish().unwrap()).unwrap();
        assert_eq!(compressed, parsed);
        assert_eq!(compressed.identity(), parsed.identity());
    }

    #[test]
    fn separates_support_files_from_publishable_payload() {
        let raw = manifest(&format!(
            r#"{},{{"type":"DepotFile","path":"app\\config.ini","flags":["support"],"chunks":[{}]}}"#,
            file("game.exe", &chunk(1)),
            chunk(2)
        ));
        let (payload, support) = parse(&raw).unwrap().split_support().unwrap();
        assert_eq!(payload.entries.len(), 1);
        assert_eq!(support.entries.len(), 1);
        assert!(matches!(&payload.entries[0], DepotEntry::File(file) if !file.support));
        assert!(matches!(&support.entries[0], DepotEntry::File(file) if file.support));
    }

    #[test]
    fn identity_covers_entry_order_and_manifest_metadata() {
        let base = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![
                DepotEntry::File(DepotFile {
                    small_file: None,
                    path: "game".into(),
                    size: 1,
                    executable: false,
                    support: false,
                    md5: Some(MD5.into()),
                    sha256: Some(SHA.into()),
                    chunks: vec![DepotChunk {
                        compressed_md5: MD5.into(),
                        compressed_size: 2,
                        md5: MD5.into(),
                        size: 1,
                    }],
                }),
                DepotEntry::Link {
                    path: "current".into(),
                    target: "game".into(),
                },
            ],
        };
        let identity = base.identity();
        let mut variants = Vec::new();
        let mut changed = base.clone();
        changed.entries.swap(0, 1);
        variants.push(changed);
        let mut changed = base.clone();
        let DepotEntry::File(file) = &mut changed.entries[0] else {
            unreachable!()
        };
        file.executable = true;
        variants.push(changed);
        let mut changed = base.clone();
        let DepotEntry::File(file) = &mut changed.entries[0] else {
            unreachable!()
        };
        file.chunks[0].size = 2;
        variants.push(changed);
        let mut changed = base.clone();
        let DepotEntry::Link { target, .. } = &mut changed.entries[1] else {
            unreachable!()
        };
        *target = "other".into();
        variants.push(changed);
        let mut changed = base.clone();
        changed.entries[0] = DepotEntry::Directory {
            path: "game".into(),
        };
        variants.push(changed);

        for changed in variants {
            assert_ne!(changed.identity(), identity);
        }
    }

    #[test]
    fn rejects_malformed_and_unsupported_input() {
        assert!(parse(b"{").is_err());
        assert!(parse(br#"{"version":1,"depot":{"items":[]}}"#).is_err());
        assert!(parse(&manifest(r#"{"type":"DepotDevice","path":"x"}"#)).is_err());
    }

    #[test]
    fn rejects_unsafe_paths_and_links() {
        for path in ["", "/root", "../x", "a/./b", "a/../b", "C:\\x", "a\0b"] {
            let json = serde_json::json!({"version":2,"depot":{"items":[{"type":"DepotDirectory","path":path}]}});
            assert!(
                parse(&serde_json::to_vec(&json).unwrap()).is_err(),
                "{path:?}"
            );
        }
        assert!(
            parse(&manifest(
                r#"{"type":"DepotLink","path":"a/link","target":"../../x"}"#
            ))
            .is_err()
        );
        assert!(
            parse(&manifest(
                r#"{"type":"DepotLink","path":"a/link","target":"../x"}"#
            ))
            .is_ok()
        );
    }

    #[test]
    fn rejects_duplicate_and_case_fold_collisions() {
        for entries in [
            r#"{"type":"DepotDirectory","path":"a"},{"type":"DepotFile","path":"a","chunks":[]}"#,
            r#"{"type":"DepotDirectory","path":"Game"},{"type":"DepotDirectory","path":"game"}"#,
        ] {
            assert!(parse(&manifest(entries)).is_err());
        }
    }

    #[test]
    fn rejects_bad_hashes_and_chunk_sizes() {
        let zero =
            format!(r#"{{"compressedMd5":"{MD5}","compressedSize":0,"md5":"{MD5}","size":1}}"#);
        assert!(parse(&manifest(&file("x", &zero))).is_err());
        let bad = format!(r#"{{"compressedMd5":"x","compressedSize":1,"md5":"{MD5}","size":1}}"#);
        assert!(parse(&manifest(&file("x", &bad))).is_err());
        let huge = format!("{},{}", chunk(u64::MAX), chunk(1));
        assert!(parse(&manifest(&file("x", &huge))).is_err());
    }

    #[test]
    fn parses_and_validates_small_file_references() {
        let item_chunk = chunk(2);
        let container_chunk = chunk(4);
        let json = format!(
            r#"{{"version":2,"depot":{{"items":[
                {{"type":"DepotFile","path":"a.info","chunks":[{item_chunk}],"sfcRef":{{"offset":0,"size":2}}}},
                {{"type":"DepotFile","path":"b.info","chunks":[{item_chunk}],"sfcRef":{{"offset":2,"size":2}}}}
            ],"smallFilesContainer":{{"chunks":[{container_chunk}]}}}}}}"#
        );
        let parsed = parse(json.as_bytes()).unwrap();
        assert_eq!(parsed.small_files_containers[0].chunks.len(), 1);
        let DepotEntry::File(file) = &parsed.entries[1] else {
            panic!()
        };
        assert_eq!(
            file.small_file,
            Some(SmallFileRef {
                container_index: 0,
                offset: 2,
                size: 2,
            })
        );

        assert!(
            parse(
                json.replace("\"offset\":2,\"size\":2", "\"offset\":1,\"size\":2")
                    .as_bytes()
            )
            .is_ok()
        );
        for bad in [
            json.replace(
                ",\"smallFilesContainer\":{\"chunks\":[",
                ",\"unused\":{\"chunks\":[",
            ),
            json.replace("\"offset\":2,\"size\":2", "\"offset\":4,\"size\":2"),
        ] {
            assert!(parse(bad.as_bytes()).is_err());
        }
    }

    #[test]
    fn rejects_oversized_expansion() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder
            .write_all(&vec![b' '; MAX_EXPANDED as usize + 1])
            .unwrap();
        assert!(parse(&encoder.finish().unwrap()).is_err());
    }

    #[test]
    fn checked_totals_reject_inconsistent_and_overflowing_manifests() {
        let chunk = DepotChunk {
            compressed_md5: MD5.into(),
            compressed_size: u64::MAX,
            md5: MD5.into(),
            size: 1,
        };
        let file = |path: &str, size, chunks| {
            DepotEntry::File(DepotFile {
                small_file: None,
                path: path.into(),
                size,
                executable: false,
                support: false,
                md5: None,
                sha256: None,
                chunks,
            })
        };
        let inconsistent = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![file("a", 2, vec![chunk.clone()])],
        };
        assert!(
            inconsistent
                .totals()
                .unwrap_err()
                .to_string()
                .contains("file chunk total")
        );
        let mut other = chunk.clone();
        other.compressed_md5 = "f".repeat(32);
        let overflow = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![file("a", 1, vec![chunk]), file("b", 1, vec![other])],
        };
        assert!(
            overflow
                .totals()
                .unwrap_err()
                .to_string()
                .contains("compressed size overflows")
        );
        let huge = DepotChunk {
            compressed_md5: MD5.into(),
            compressed_size: 1,
            md5: MD5.into(),
            size: u64::MAX,
        };
        let one = DepotChunk {
            compressed_md5: "f".repeat(32),
            compressed_size: 1,
            md5: MD5.into(),
            size: 1,
        };
        let overflow = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![file("a", u64::MAX, vec![huge]), file("b", 1, vec![one])],
        };
        assert!(
            overflow
                .totals()
                .unwrap_err()
                .to_string()
                .contains("uncompressed size overflows")
        );
        assert!(
            DepotManifest {
                small_files_containers: Vec::new(),
                generation: 1,
                entries: Vec::new()
            }
            .totals()
            .is_err()
        );
    }
}
