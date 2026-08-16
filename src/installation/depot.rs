use super::marker::{self, InstallationMarker};
use crate::{
    domain::InstallationSource,
    gog::depot_manifest::{DepotEntry, DepotManifest},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

const STAGING_OVERHEAD: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepotDiskSpaceError {
    pub required: u64,
    pub available: u64,
}

impl std::fmt::Display for DepotDiskSpaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "depot operation requires {} bytes but only {} are available",
            self.required, self.available
        )
    }
}

impl std::error::Error for DepotDiskSpaceError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepotOperationKind {
    Install,
    Update,
    BranchSwitch,
    Repair,
}

pub struct DepotInstallPlan<'a> {
    pub operation: DepotOperationKind,
    pub target: PathBuf,
    pub target_manifest: &'a DepotManifest,
    pub current_manifest: Option<&'a DepotManifest>,
    pub target_marker: InstallationMarker,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CommitFailure {
    #[default]
    None,
    AfterRemovals,
    AfterMove(usize),
    BeforeMarker,
}

impl DepotInstallPlan<'_> {
    pub fn validate(&self) -> Result<()> {
        self.target_marker.validate()?;
        reject_marker_symlink(&self.target)?;
        if fs::symlink_metadata(&self.target)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            bail!("depot installation target cannot be a symlink");
        }
        if self.target_marker.source != InstallationSource::GalaxyDepot {
            bail!("depot operation requires Galaxy depot target provenance");
        }
        let provenance = self.target_marker.galaxy_depot.as_ref().unwrap();
        if provenance.build_id.is_empty()
            || provenance.repository_id.is_empty()
            || provenance.manifest_fingerprint.is_empty()
        {
            bail!("depot target provenance is incomplete");
        }
        validate_manifest(self.target_manifest)?;
        if provenance.manifest_fingerprint != self.target_manifest.identity() {
            bail!("target depot manifest does not match marker provenance");
        }
        match self.operation {
            DepotOperationKind::Install => {
                if self.current_manifest.is_some() {
                    bail!("fresh depot install cannot have a current manifest");
                }
                if self.target.exists() && !self.target.is_dir() {
                    bail!("depot install target is not a directory");
                }
            }
            _ => {
                let current = self
                    .current_manifest
                    .context("depot update, branch switch, or repair requires current manifest")?;
                validate_manifest(current)?;
                let existing = marker::load(&self.target)?
                    .context("depot operation requires an existing installation marker")?;
                if existing.source != InstallationSource::GalaxyDepot {
                    bail!("offline-managed installations cannot use depot operations");
                }
                if existing.product_id != self.target_marker.product_id {
                    bail!("target marker product does not match installed product");
                }
                let provenance = existing.galaxy_depot.as_ref().unwrap();
                if provenance.build_id.is_empty()
                    || provenance.repository_id.is_empty()
                    || provenance.manifest_fingerprint.is_empty()
                {
                    bail!("installed depot provenance is incomplete");
                }
                if provenance.manifest_fingerprint != current.identity() {
                    bail!("current depot manifest does not match installed provenance");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
fn execute<F>(plan: &DepotInstallPlan<'_>, fetch: F) -> Result<()>
where
    F: FnMut(&crate::gog::depot_manifest::DepotChunk) -> Result<Vec<u8>>,
{
    execute_inner(plan, None, fetch, || false, false)
}

#[cfg(test)]
fn execute_inner<F, C>(
    plan: &DepotInstallPlan<'_>,
    staging: Option<&Path>,
    mut fetch: F,
    cancelled: C,
    fail_before_marker: bool,
) -> Result<()>
where
    F: FnMut(&crate::gog::depot_manifest::DepotChunk) -> Result<Vec<u8>>,
    C: FnMut() -> bool,
{
    execute_streamed_inner(
        plan,
        staging,
        (&BTreeSet::new(), &std::collections::HashSet::new()),
        |chunks, output, completed| {
            for (index, job) in chunks.iter().enumerate() {
                let mut writer =
                    crate::download::depot::FileRegionWriter::new(output.try_clone()?, job.offset);
                std::io::Write::write_all(&mut writer, &fetch(job.chunk)?)?;
                completed(index)?;
            }
            Ok(())
        },
        cancelled,
        || Ok(()),
        if fail_before_marker {
            CommitFailure::BeforeMarker
        } else {
            CommitFailure::None
        },
    )
}

pub(crate) fn execute_streamed_forward<F, C, P>(
    plan: &DepotInstallPlan<'_>,
    staging: &Path,
    forced_remove_paths: &BTreeSet<String>,
    trusted_files: &std::collections::HashSet<String>,
    fetch: F,
    cancelled: C,
    before_commit: P,
) -> Result<()>
where
    F: FnMut(
        &[crate::download::depot::ChunkWrite<'_>],
        &std::fs::File,
        &mut dyn FnMut(usize) -> Result<()>,
    ) -> Result<()>,
    C: FnMut() -> bool,
    P: FnMut() -> Result<()>,
{
    for path in forced_remove_paths {
        checked_join(Path::new("root"), path)?;
    }
    execute_streamed_inner(
        plan,
        Some(staging),
        (forced_remove_paths, trusted_files),
        fetch,
        cancelled,
        before_commit,
        CommitFailure::None,
    )
}

#[cfg(test)]
fn execute_controlled<F, C>(
    plan: &DepotInstallPlan<'_>,
    staging: &Path,
    mut fetch: F,
    cancelled: C,
) -> Result<()>
where
    F: FnMut(&crate::gog::depot_manifest::DepotChunk) -> Result<Vec<u8>>,
    C: FnMut() -> bool,
{
    execute_streamed_forward(
        plan,
        staging,
        &BTreeSet::new(),
        &std::collections::HashSet::new(),
        |chunks, output, completed| {
            for (index, job) in chunks.iter().enumerate() {
                let mut writer =
                    crate::download::depot::FileRegionWriter::new(output.try_clone()?, job.offset);
                std::io::Write::write_all(&mut writer, &fetch(job.chunk)?)?;
                completed(index)?;
            }
            Ok(())
        },
        cancelled,
        || Ok(()),
    )?;
    crate::download::depot::finish_journal(staging)
}

fn execute_streamed_inner<F, C, P>(
    plan: &DepotInstallPlan<'_>,
    supplied_staging: Option<&Path>,
    paths: (&BTreeSet<String>, &std::collections::HashSet<String>),
    fetch: F,
    mut cancelled: C,
    mut before_commit: P,
    failure: CommitFailure,
) -> Result<()>
where
    F: FnMut(
        &[crate::download::depot::ChunkWrite<'_>],
        &std::fs::File,
        &mut dyn FnMut(usize) -> Result<()>,
    ) -> Result<()>,
    C: FnMut() -> bool,
    P: FnMut() -> Result<()>,
{
    let (forced_remove_paths, trusted_files) = paths;
    plan.validate()?;
    let generated = plan.target.with_extension("ludomere-depot.json");
    let journal = supplied_staging.unwrap_or(&generated);
    let library = if supplied_staging.is_some() {
        validate_staging(&plan.target, journal)?;
        let library = journal
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .context("controlled depot journal has no library root")?;
        disk_preflight(plan.target_manifest, library, &plan.target, journal)?;
        ensure_operation_staging(library, journal)?;
        Some(library)
    } else {
        None
    };
    let result = (|| {
        preflight_live_tree(plan, forced_remove_paths)?;
        crate::download::depot::materialize_streamed_controlled(
            plan.target_manifest,
            &plan.target,
            journal,
            trusted_files,
            fetch,
            &mut cancelled,
        )
        .context("materializing depot build")?;
        if cancelled() {
            return Err(crate::download::depot::DepotCancelled.into());
        }
        before_commit()?;
        commit(plan, forced_remove_paths, failure)
    })();
    if supplied_staging.is_none() {
        let _ = fs::remove_file(journal);
    }
    let _ = library;
    result
}

pub fn operation_staging_path(
    library_root: &Path,
    destination: &Path,
    slug: &str,
    _operation_id: &str,
) -> Result<PathBuf> {
    validate_identifier(slug, "slug")?;
    reject_symlink_components(library_root)?;
    let relative = destination
        .strip_prefix(library_root)
        .context("depot destination is outside its library")?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || relative
            .components()
            .next()
            .is_some_and(|part| part.as_os_str() == ".ludomere")
    {
        bail!("depot destination must be strictly inside the library payload tree");
    }
    reject_symlink_components(
        destination
            .parent()
            .context("depot destination has no parent")?,
    )?;
    if fs::symlink_metadata(destination).is_ok_and(|value| value.file_type().is_symlink()) {
        bail!("depot destination is a symlink");
    }
    let control = library_root.join(".ludomere");
    let base = control.join("staging");
    for path in [&control, &base] {
        if fs::symlink_metadata(path).is_ok_and(|value| value.file_type().is_symlink()) {
            bail!("depot control path is a symlink");
        }
    }
    let journal = base.join(format!("{slug}.json"));
    if fs::symlink_metadata(&journal).is_ok_and(|value| value.file_type().is_symlink()) {
        bail!("depot operation journal is a symlink");
    }
    Ok(journal)
}

fn ensure_operation_staging(library_root: &Path, staging: &Path) -> Result<()> {
    let base = library_root.join(".ludomere/staging");
    fs::create_dir_all(&base)?;
    reject_symlink_components(&base)?;
    if staging.parent() != Some(base.as_path())
        || staging.extension().and_then(|v| v.to_str()) != Some("json")
    {
        bail!("invalid controlled depot journal path");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata(&base)?.dev() != fs::metadata(library_root)?.dev() {
            bail!("depot staging and library are on different filesystems");
        }
    }
    Ok(())
}

pub fn disk_preflight(
    manifest: &DepotManifest,
    library_root: &Path,
    target: &Path,
    journal: &Path,
) -> Result<u64> {
    disk_preflight_with(manifest, target, journal, || {
        fs2::available_space(library_root)
    })
}

pub fn inspect_abandoned_depot_staging(
    library: &Path,
    destination: &Path,
    slug: &str,
    operation_id: &str,
) -> Result<Option<u64>> {
    let path = operation_staging_path(library, destination, slug, operation_id)?;
    if !path.exists() {
        return Ok(None);
    }
    reject_symlink_components(&path)?;
    Ok(Some(fs::metadata(path)?.len()))
}

pub fn delete_abandoned_depot_staging(
    library: &Path,
    destination: &Path,
    slug: &str,
    operation_id: &str,
) -> Result<bool> {
    let path = operation_staging_path(library, destination, slug, operation_id)?;
    if !path.exists() {
        return Ok(false);
    }
    reject_symlink_components(&path)?;
    fs::remove_file(path)?;
    Ok(true)
}

pub fn removed_dlc_marker_paths(product_ids: &[i64]) -> Result<BTreeSet<String>> {
    product_ids
        .iter()
        .map(|id| {
            if *id <= 0 {
                bail!("invalid removed DLC product ID");
            }
            Ok(format!("goggame-{id}.info"))
        })
        .collect()
}

fn disk_preflight_with<F>(
    manifest: &DepotManifest,
    target: &Path,
    journal: &Path,
    available: F,
) -> Result<u64>
where
    F: FnOnce() -> std::io::Result<u64>,
{
    let total = manifest.totals()?.uncompressed;
    let staged =
        crate::download::depot::journal_staged_bytes_at(manifest, target, journal).unwrap_or(0);
    let required = checked_disk_requirement(total, staged)?;
    let available = available()?;
    if available < required {
        return Err(DepotDiskSpaceError {
            required,
            available,
        }
        .into());
    }
    Ok(required)
}

fn checked_disk_requirement(total: u64, staged: u64) -> Result<u64> {
    total
        .checked_sub(staged)
        .context("staged bytes exceed target manifest")?
        .checked_add(STAGING_OVERHEAD)
        .context("depot staging requirement overflows")
}

fn validate_identifier(value: &str, name: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 96
        || value.starts_with('.')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        bail!("invalid depot {name}");
    }
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component.as_os_str());
        if fs::symlink_metadata(&cursor).is_ok_and(|value| value.file_type().is_symlink()) {
            bail!("depot path crosses a symlink");
        }
    }
    Ok(())
}

fn reject_marker_symlink(target: &Path) -> Result<()> {
    let directory = target.join(crate::identity::MARKER_DIRECTORY);
    if fs::symlink_metadata(directory).is_ok_and(|value| value.file_type().is_symlink()) {
        bail!("installation marker directory cannot be a symlink");
    }
    Ok(())
}

fn validate_staging(target: &Path, staging: &Path) -> Result<()> {
    if staging == target
        || staging.extension().and_then(|value| value.to_str()) != Some("json")
        || staging.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("staging"))
        || staging
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            != Some(std::ffi::OsStr::new(".ludomere"))
        || fs::symlink_metadata(staging).is_ok_and(|value| value.file_type().is_symlink())
    {
        bail!("depot staging path is outside the controlled operation layout");
    }
    reject_symlink_components(staging)?;
    Ok(())
}

fn commit(
    plan: &DepotInstallPlan<'_>,
    forced_remove_paths: &BTreeSet<String>,
    failure: CommitFailure,
) -> Result<()> {
    reject_marker_symlink(&plan.target)?;
    fs::create_dir_all(&plan.target)?;
    let current = plan
        .current_manifest
        .map(managed_leaves)
        .unwrap_or_default();
    let target = managed_leaves(plan.target_manifest);
    let removed = current
        .difference(&target)
        .cloned()
        .chain(
            forced_remove_paths
                .iter()
                .filter(|path| !target.contains(*path))
                .cloned(),
        )
        .collect::<BTreeSet<_>>();
    for relative in &removed {
        let live = checked_join(&plan.target, relative)?;
        if fs::symlink_metadata(&live).is_ok() {
            remove_path(&live)?;
        }
    }
    if failure == CommitFailure::AfterRemovals {
        bail!("injected depot commit failure after removals");
    }
    for (index, relative) in target.iter().enumerate() {
        if failure == CommitFailure::AfterMove(index) {
            bail!("injected depot commit failure after published file");
        }
        checked_join(&plan.target, relative)?;
    }
    create_manifest_directories(plan.target_manifest, &plan.target)?;
    if failure == CommitFailure::BeforeMarker {
        bail!("injected depot commit failure");
    }
    marker::write(&plan.target_marker, &plan.target).context("publishing depot marker")?;
    cleanup_removed_dirs(plan.current_manifest, plan.target_manifest, &plan.target);
    Ok(())
}

fn preflight_live_tree(
    plan: &DepotInstallPlan<'_>,
    forced_remove_paths: &BTreeSet<String>,
) -> Result<()> {
    for relative in managed_paths(plan.target_manifest)
        .into_iter()
        .chain(plan.current_manifest.into_iter().flat_map(managed_paths))
        .chain(forced_remove_paths.iter().cloned())
    {
        let mut cursor = plan.target.clone();
        for component in Path::new(&relative).components() {
            cursor.push(component.as_os_str());
            if cursor != plan.target.join(&relative)
                && fs::symlink_metadata(&cursor).is_ok_and(|value| value.file_type().is_symlink())
            {
                bail!("managed path crosses a live symlink");
            }
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &DepotManifest) -> Result<()> {
    for entry in &manifest.entries {
        let path = match entry {
            DepotEntry::Directory { path }
            | DepotEntry::File(crate::gog::depot_manifest::DepotFile { path, .. }) => path,
            DepotEntry::Link { path, target } => {
                crate::gog::depot_manifest::validate_link(path, target)?;
                path
            }
        };
        checked_join(Path::new("root"), path)?;
    }
    Ok(())
}

fn managed_paths(manifest: &DepotManifest) -> Vec<String> {
    manifest
        .entries
        .iter()
        .map(|entry| match entry {
            DepotEntry::Directory { path }
            | DepotEntry::Link { path, .. }
            | DepotEntry::File(crate::gog::depot_manifest::DepotFile { path, .. }) => path.clone(),
        })
        .collect()
}

fn managed_leaves(manifest: &DepotManifest) -> BTreeSet<String> {
    manifest
        .entries
        .iter()
        .filter_map(|entry| match entry {
            DepotEntry::File(file) => Some(file.path.clone()),
            DepotEntry::Link { path, .. } => Some(path.clone()),
            DepotEntry::Directory { .. } => None,
        })
        .collect()
}

fn create_manifest_directories(manifest: &DepotManifest, root: &Path) -> Result<()> {
    for entry in &manifest.entries {
        if let DepotEntry::Directory { path } = entry {
            fs::create_dir_all(checked_join(root, path)?)?;
        }
    }
    Ok(())
}

fn cleanup_removed_dirs(current: Option<&DepotManifest>, target: &DepotManifest, root: &Path) {
    let retained = managed_paths(target).into_iter().collect::<BTreeSet<_>>();
    if let Some(current) = current {
        let mut directories = current
            .entries
            .iter()
            .filter_map(|entry| match entry {
                DepotEntry::Directory { path } if !retained.contains(path) => Some(path),
                _ => None,
            })
            .collect::<Vec<_>>();
        directories.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
        for relative in directories {
            let _ = fs::remove_dir(root.join(relative));
        }
    }
}

fn checked_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe depot transaction path");
    }
    Ok(root.join(path))
}

fn remove_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)?
        }
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{
            GalaxyDepotDlcProvenance, GalaxyDepotIdentity, GalaxyDepotProvenance,
            InstallationSource,
        },
        gog::depot_manifest::{DepotChunk, DepotFile},
        installation::marker::{InstalledComponent, InstalledDlc},
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp(name: &str) -> PathBuf {
        let value = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("ludomere-depot-transaction-{name}-{value}"))
    }

    fn chunk(value: &[u8]) -> DepotChunk {
        DepotChunk {
            compressed_md5: format!("{:x}", md5::compute(value)),
            compressed_size: 1,
            md5: format!("{:x}", md5::compute(value)),
            size: value.len() as u64,
        }
    }

    fn manifest(files: &[(&str, &[u8])]) -> DepotManifest {
        DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: files
                .iter()
                .map(|(path, value)| {
                    DepotEntry::File(DepotFile {
                        small_file: None,
                        path: (*path).into(),
                        size: value.len() as u64,
                        executable: path.ends_with(".sh"),
                        support: false,
                        md5: Some(format!("{:x}", md5::compute(value))),
                        sha256: None,
                        chunks: vec![chunk(value)],
                    })
                })
                .collect(),
        }
    }

    fn marker(build: &str, manifest: &DepotManifest) -> InstallationMarker {
        InstallationMarker {
            schema_version: 1,
            product_id: 7,
            slug: "game".into(),
            base: InstalledComponent {
                operating_system: Some("linux".into()),
                language: Some("en".into()),
                version: Some(build.into()),
                revision_id: None,
                installed_at: 1,
            },
            dlc: vec![InstalledDlc {
                product_id: 8,
                version: Some(build.into()),
                revision_id: None,
                installed_at: 1,
            }],
            compatibility: None,
            source: InstallationSource::GalaxyDepot,
            galaxy_depot: Some(GalaxyDepotProvenance {
                build_id: build.into(),
                repository_id: "repo".into(),
                manifest_fingerprint: manifest.identity(),
                branch: None,
                language: Some("en".into()),
                architecture: Some("x86_64".into()),
                depots: vec![GalaxyDepotIdentity {
                    depot_id: "base".into(),
                    manifest_id: format!("base-{build}"),
                }],
                dlc: vec![GalaxyDepotDlcProvenance {
                    product_id: 8,
                    depots: Vec::new(),
                    has_payload: false,
                    entitlement_only_marker: true,
                }],
            }),
            launch: None,
            dependencies: Vec::new(),
        }
    }

    fn fetch<'a>(
        files: &'a [(&'a str, &'a [u8])],
    ) -> impl FnMut(&DepotChunk) -> Result<Vec<u8>> + 'a {
        move |wanted| {
            files
                .iter()
                .map(|(_, value)| *value)
                .find(|value| format!("{:x}", md5::compute(value)) == wanted.md5)
                .map(Vec::from)
                .context("missing synthetic chunk")
        }
    }

    fn run(
        kind: DepotOperationKind,
        root: &Path,
        current: Option<&DepotManifest>,
        target: &DepotManifest,
        target_files: &[(&str, &[u8])],
        build: &str,
    ) -> Result<()> {
        execute(
            &DepotInstallPlan {
                operation: kind,
                target: root.into(),
                target_manifest: target,
                current_manifest: current,
                target_marker: marker(build, target),
            },
            fetch(target_files),
        )
    }

    #[test]
    fn fresh_install_overwrites_managed_paths_and_publishes_marker() {
        let root = temp("install");
        let files = [("bin/game.sh", b"new".as_slice())];
        let target = manifest(&files);
        run(
            DepotOperationKind::Install,
            &root,
            None,
            &target,
            &files,
            "2",
        )
        .unwrap();
        assert_eq!(fs::read(root.join("bin/game.sh")).unwrap(), b"new");
        assert_eq!(marker::load(&root).unwrap(), Some(marker("2", &target)));

        let collision = temp("collision");
        fs::create_dir_all(&collision).unwrap();
        fs::write(collision.join("mod.txt"), b"keep").unwrap();
        run(
            DepotOperationKind::Install,
            &collision,
            None,
            &target,
            &files,
            "2",
        )
        .unwrap();
        assert_eq!(fs::read(collision.join("mod.txt")).unwrap(), b"keep");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(collision);
    }

    #[test]
    fn update_branch_switch_and_repair_replace_only_managed_files() {
        let root = temp("lifecycle");
        let old_files = [
            ("game.dat", b"old".as_slice()),
            ("removed.dat", b"gone".as_slice()),
        ];
        let new_files = [
            ("game.dat", b"new".as_slice()),
            ("added.dat", b"added".as_slice()),
        ];
        let old = manifest(&old_files);
        let new = manifest(&new_files);
        run(
            DepotOperationKind::Install,
            &root,
            None,
            &old,
            &old_files,
            "1",
        )
        .unwrap();
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/user.cfg"), b"keep").unwrap();
        run(
            DepotOperationKind::Update,
            &root,
            Some(&old),
            &new,
            &new_files,
            "2",
        )
        .unwrap();
        assert_eq!(fs::read(root.join("game.dat")).unwrap(), b"new");
        assert!(!root.join("removed.dat").exists());
        assert_eq!(fs::read(root.join("mods/user.cfg")).unwrap(), b"keep");
        run(
            DepotOperationKind::BranchSwitch,
            &root,
            Some(&new),
            &old,
            &old_files,
            "1",
        )
        .unwrap();
        assert_eq!(fs::read(root.join("game.dat")).unwrap(), b"old");
        fs::write(root.join("game.dat"), b"corrupt").unwrap();
        run(
            DepotOperationKind::Repair,
            &root,
            Some(&old),
            &old,
            &old_files,
            "1",
        )
        .unwrap();
        assert_eq!(fs::read(root.join("game.dat")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("mods/user.cfg")).unwrap(), b"keep");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_offline_marker_and_materialization_failure_is_non_mutating() {
        let root = temp("failure");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("game.dat"), b"old").unwrap();
        let files = [("game.dat", b"new".as_slice())];
        let current = manifest(&[("game.dat", b"old".as_slice())]);
        let target = manifest(&files);
        let mut offline = marker("1", &current);
        offline.source = InstallationSource::OfflineInstaller;
        offline.galaxy_depot = None;
        marker::write(&offline, &root).unwrap();
        assert!(
            run(
                DepotOperationKind::Update,
                &root,
                Some(&current),
                &target,
                &files,
                "2"
            )
            .is_err()
        );
        assert_eq!(fs::read(root.join("game.dat")).unwrap(), b"old");

        marker::write(&marker("1", &current), &root).unwrap();
        assert!(
            execute(
                &DepotInstallPlan {
                    operation: DepotOperationKind::Update,
                    target: root.clone(),
                    target_manifest: &target,
                    current_manifest: Some(&current),
                    target_marker: marker("2", &target),
                },
                |_| bail!("fetch failed")
            )
            .is_err()
        );
        assert_eq!(fs::read(root.join("game.dat")).unwrap(), b"old");
        assert_eq!(marker::load(&root).unwrap(), Some(marker("1", &current)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_failure_leaves_forward_payload_with_old_marker() {
        let root = temp("forward-failure");
        let old_files = [("game.dat", b"old".as_slice())];
        let new_files = [("game.dat", b"new".as_slice())];
        let old = manifest(&old_files);
        let new = manifest(&new_files);
        run(
            DepotOperationKind::Install,
            &root,
            None,
            &old,
            &old_files,
            "1",
        )
        .unwrap();
        let plan = DepotInstallPlan {
            operation: DepotOperationKind::Update,
            target: root.clone(),
            target_manifest: &new,
            current_manifest: Some(&old),
            target_marker: marker("2", &new),
        };
        assert!(execute_inner(&plan, None, fetch(&new_files), || false, true).is_err());
        assert_eq!(fs::read(root.join("game.dat")).unwrap(), b"new");
        let restored = marker::load(&root).unwrap().unwrap();
        assert_eq!(restored, marker("1", &old));
        assert_eq!(restored.base.revision_id, None);
        assert_eq!(restored.galaxy_depot.unwrap().dlc.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn preserves_executable_and_link_behavior_and_rejects_escape() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp("links");
        let files = [("bin/game.sh", b"run".as_slice())];
        let mut target = manifest(&files);
        target.entries.push(DepotEntry::Link {
            path: "game".into(),
            target: "bin/game.sh".into(),
        });
        run(
            DepotOperationKind::Install,
            &root,
            None,
            &target,
            &files,
            "1",
        )
        .unwrap();
        assert_ne!(
            fs::metadata(root.join("bin/game.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
        assert_eq!(
            fs::read_link(root.join("game")).unwrap(),
            Path::new("bin/game.sh")
        );

        let unsafe_manifest = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![DepotEntry::Directory {
                path: "../escape".into(),
            }],
        };
        assert!(
            DepotInstallPlan {
                operation: DepotOperationKind::Install,
                target: temp("escape"),
                target_manifest: &unsafe_manifest,
                current_manifest: None,
                target_marker: marker("1", &unsafe_manifest),
            }
            .validate()
            .is_err()
        );
        for target in ["../../escape", "/escape", "bad\\target", "bad\0target"] {
            let unsafe_link = DepotManifest {
                small_files_containers: Vec::new(),
                generation: 2,
                entries: vec![DepotEntry::Link {
                    path: "dir/link".into(),
                    target: target.into(),
                }],
            };
            assert!(
                DepotInstallPlan {
                    operation: DepotOperationKind::Install,
                    target: temp("unsafe-link"),
                    target_manifest: &unsafe_link,
                    current_manifest: None,
                    target_marker: marker("1", &unsafe_link),
                }
                .validate()
                .is_err(),
                "{target:?}"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_manifest_fingerprint_mismatches_before_mutation() {
        let root = temp("fingerprints");
        let old_files = [("game.dat", b"old".as_slice())];
        let new_files = [("game.dat", b"new".as_slice())];
        let old = manifest(&old_files);
        let new = manifest(&new_files);

        let mut wrong_target = marker("1", &old);
        wrong_target
            .galaxy_depot
            .as_mut()
            .unwrap()
            .manifest_fingerprint = "sha256:wrong".into();
        assert!(
            DepotInstallPlan {
                operation: DepotOperationKind::Install,
                target: root.clone(),
                target_manifest: &old,
                current_manifest: None,
                target_marker: wrong_target,
            }
            .validate()
            .is_err()
        );
        assert!(!root.exists());

        run(
            DepotOperationKind::Install,
            &root,
            None,
            &old,
            &old_files,
            "1",
        )
        .unwrap();
        let mut installed = marker::load(&root).unwrap().unwrap();
        installed
            .galaxy_depot
            .as_mut()
            .unwrap()
            .manifest_fingerprint = "sha256:wrong".into();
        marker::write(&installed, &root).unwrap();
        for operation in [
            DepotOperationKind::Update,
            DepotOperationKind::BranchSwitch,
            DepotOperationKind::Repair,
        ] {
            let target = if operation == DepotOperationKind::Repair {
                &old
            } else {
                &new
            };
            assert!(
                DepotInstallPlan {
                    operation,
                    target: root.clone(),
                    target_manifest: target,
                    current_manifest: Some(&old),
                    target_marker: marker("2", target),
                }
                .validate()
                .is_err()
            );
        }
        assert_eq!(fs::read(root.join("game.dat")).unwrap(), b"old");
        assert_eq!(marker::load(&root).unwrap(), Some(installed));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn destructively_removes_directory_at_managed_leaf() {
        let root = temp("removed-leaf-directory");
        let old_files = [("removed", b"old".as_slice())];
        let old = manifest(&old_files);
        let new = manifest(&[]);
        run(
            DepotOperationKind::Install,
            &root,
            None,
            &old,
            &old_files,
            "1",
        )
        .unwrap();
        fs::remove_file(root.join("removed")).unwrap();
        fs::create_dir(root.join("removed")).unwrap();
        fs::write(root.join("removed/user.txt"), b"keep").unwrap();
        let before = marker::load(&root).unwrap();

        assert!(
            run(
                DepotOperationKind::Update,
                &root,
                Some(&old),
                &new,
                &[],
                "2"
            )
            .is_ok()
        );
        assert!(!root.join("removed").exists());
        assert_ne!(marker::load(&root).unwrap(), before);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn controlled_execution_cancels_before_commit() {
        let library = temp("controlled-cancel");
        fs::create_dir_all(&library).unwrap();
        let root = library.join("game");
        let staging = operation_staging_path(&library, &root, "game", "op1").unwrap();
        let files = [("game.dat", b"new".as_slice())];
        let target = manifest(&files);
        let plan = DepotInstallPlan {
            operation: DepotOperationKind::Install,
            target: root.clone(),
            target_manifest: &target,
            current_manifest: None,
            target_marker: marker("1", &target),
        };
        let mut checks = 0;
        let error = execute_controlled(&plan, &staging, fetch(&files), || {
            checks += 1;
            checks == 3
        })
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::download::depot::DepotCancelled>()
                .is_some()
        );
        assert!(root.exists());
        assert!(staging.exists());
        let _ = fs::remove_dir_all(library);
    }

    #[test]
    fn controlled_execution_rejects_unsafe_staging() {
        let library = temp("controlled-staging");
        fs::create_dir_all(&library).unwrap();
        let root = library.join("game");
        let outside = root.parent().unwrap().join("nested").join("stage");
        let target = manifest(&[]);
        let plan = DepotInstallPlan {
            operation: DepotOperationKind::Install,
            target: root.clone(),
            target_manifest: &target,
            current_manifest: None,
            target_marker: marker("1", &target),
        };
        assert!(execute_controlled(&plan, &outside, fetch(&[]), || false).is_err());
        let staging = operation_staging_path(&library, &root, "game", "op1").unwrap();
        fs::create_dir_all(&staging).unwrap();
        assert!(execute_controlled(&plan, &staging, fetch(&[]), || false).is_err());
        fs::remove_dir(&staging).unwrap();
        assert!(execute_controlled(&plan, &staging, fetch(&[]), || false).is_ok());
        assert!(!staging.exists());
        let _ = fs::remove_dir_all(library);
    }

    #[test]
    fn derives_only_library_owned_staging_paths() {
        let library = temp("layout");
        fs::create_dir_all(&library).unwrap();
        let destination = library.join("grim_dawn");
        assert_eq!(
            operation_staging_path(&library, &destination, "grim_dawn", "a1b2c3").unwrap(),
            library.join(".ludomere/staging/grim_dawn.json")
        );
        assert!(!library.join(".ludomere").exists());
        for (destination, slug, operation) in [
            (library.join(".ludomere/game"), "game", "op"),
            (library.with_file_name("outside"), "game", "op"),
            (library.join("game"), "../game", "op"),
        ] {
            assert!(operation_staging_path(&library, &destination, slug, operation).is_err());
        }
        fs::create_dir_all(library.join(".ludomere/compatibility/sentinel")).unwrap();
        assert!(library.join(".ludomere/compatibility/sentinel").is_dir());
        let _ = fs::remove_dir_all(library);
    }

    #[test]
    fn disk_preflight_is_checked_and_non_mutating() {
        let library = temp("disk");
        fs::create_dir_all(&library).unwrap();
        let staging = library.join(".ludomere/staging/game.json");
        let destination = library.join("game");
        let target = manifest(&[("game.dat", b"payload".as_slice())]);
        let error = disk_preflight_with(&target, &destination, &staging, || Ok(0)).unwrap_err();
        let error = error.downcast_ref::<DepotDiskSpaceError>().unwrap();
        assert_eq!(error.available, 0);
        assert_eq!(error.required, STAGING_OVERHEAD + 7);
        assert!(!library.join(".ludomere").exists());
        assert_eq!(
            disk_preflight_with(&target, &destination, &staging, || Ok(u64::MAX)).unwrap(),
            STAGING_OVERHEAD + 7
        );
        let huge = DepotManifest {
            small_files_containers: Vec::new(),
            generation: 2,
            entries: vec![DepotEntry::File(DepotFile {
                small_file: None,
                path: "huge".into(),
                size: u64::MAX,
                executable: false,
                support: false,
                md5: None,
                sha256: None,
                chunks: vec![DepotChunk {
                    compressed_md5: "0".repeat(32),
                    compressed_size: 1,
                    md5: "0".repeat(32),
                    size: u64::MAX,
                }],
            })],
        };
        assert!(disk_preflight_with(&huge, &destination, &staging, || Ok(u64::MAX)).is_err());
        assert_eq!(
            checked_disk_requirement(100, 40).unwrap(),
            STAGING_OVERHEAD + 60
        );
        assert!(checked_disk_requirement(40, 41).is_err());
        assert!(checked_disk_requirement(u64::MAX, 0).is_err());
        let _ = fs::remove_dir_all(library);
    }

    #[test]
    fn abandoned_cleanup_is_exact_and_preserves_control_siblings() {
        let library = temp("abandoned");
        fs::create_dir_all(library.join(".ludomere/staging")).unwrap();
        fs::write(library.join(".ludomere/staging/game.json"), b"abc").unwrap();
        fs::create_dir_all(library.join(".ludomere/staging/other-op")).unwrap();
        fs::create_dir_all(library.join(".ludomere/compatibility/sentinel")).unwrap();
        let destination = library.join("game");
        assert_eq!(
            inspect_abandoned_depot_staging(&library, &destination, "game", "op").unwrap(),
            Some(3)
        );
        assert!(delete_abandoned_depot_staging(&library, &destination, "game", "op").unwrap());
        assert!(library.join(".ludomere/staging/other-op").is_dir());
        assert!(library.join(".ludomere/compatibility/sentinel").is_dir());
        assert_eq!(
            removed_dlc_marker_paths(&[42]).unwrap(),
            BTreeSet::from(["goggame-42.info".into()])
        );
        assert!(removed_dlc_marker_paths(&[0]).is_err());
        let _ = fs::remove_dir_all(library);
    }

    #[cfg(unix)]
    #[test]
    fn repair_replaces_symlink_and_nonempty_directory_blockers() {
        let root = temp("repair-blockers");
        let files = [
            ("link", b"one".as_slice()),
            ("directory", b"two".as_slice()),
        ];
        let target = manifest(&files);
        run(
            DepotOperationKind::Install,
            &root,
            None,
            &target,
            &files,
            "1",
        )
        .unwrap();
        fs::remove_file(root.join("link")).unwrap();
        let outside = temp("outside-sentinel");
        fs::write(&outside, b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        fs::remove_file(root.join("directory")).unwrap();
        fs::create_dir(root.join("directory")).unwrap();
        fs::write(root.join("directory/unknown"), b"remove").unwrap();
        fs::write(root.join("unknown-sibling"), b"keep").unwrap();
        run(
            DepotOperationKind::Repair,
            &root,
            Some(&target),
            &target,
            &files,
            "1",
        )
        .unwrap();
        assert_eq!(fs::read(root.join("link")).unwrap(), b"one");
        assert_eq!(fs::read(root.join("directory")).unwrap(), b"two");
        assert_eq!(fs::read(root.join("unknown-sibling")).unwrap(), b"keep");
        assert_eq!(fs::read(&outside).unwrap(), b"outside");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn controlled_failures_roll_forward_at_every_commit_boundary() {
        for failure in [
            CommitFailure::AfterRemovals,
            CommitFailure::AfterMove(0),
            CommitFailure::AfterMove(1),
            CommitFailure::AfterMove(2),
            CommitFailure::BeforeMarker,
        ] {
            let library = temp("forward-boundary");
            fs::create_dir_all(&library).unwrap();
            let root = library.join("game");
            let old_files = [("old.dat", b"old".as_slice())];
            let new_files = [
                ("a.dat", b"a".as_slice()),
                ("b.dat", b"b".as_slice()),
                ("c.dat", b"c".as_slice()),
            ];
            let old = manifest(&old_files);
            let new = manifest(&new_files);
            run(
                DepotOperationKind::Install,
                &root,
                None,
                &old,
                &old_files,
                "1",
            )
            .unwrap();
            fs::write(root.join("unknown"), b"keep").unwrap();
            let staging = operation_staging_path(&library, &root, "game", "op").unwrap();
            let plan = DepotInstallPlan {
                operation: DepotOperationKind::Update,
                target: root.clone(),
                target_manifest: &new,
                current_manifest: Some(&old),
                target_marker: marker("2", &new),
            };
            let mut first_fetch = fetch(&new_files);
            assert!(
                execute_streamed_inner(
                    &plan,
                    Some(&staging),
                    (&BTreeSet::new(), &std::collections::HashSet::new()),
                    |chunks, output, completed| {
                        for (index, job) in chunks.iter().enumerate() {
                            let mut writer = crate::download::depot::FileRegionWriter::new(
                                output.try_clone()?,
                                job.offset,
                            );
                            std::io::Write::write_all(&mut writer, &first_fetch(job.chunk)?)?;
                            completed(index)?;
                        }
                        Ok(())
                    },
                    || false,
                    || Ok(()),
                    failure,
                )
                .is_err()
            );
            assert!(staging.is_file());
            execute_controlled(&plan, &staging, fetch(&new_files), || false).unwrap();
            for (path, bytes) in new_files {
                assert_eq!(fs::read(root.join(path)).unwrap(), bytes);
            }
            assert!(!root.join("old.dat").exists());
            assert_eq!(fs::read(root.join("unknown")).unwrap(), b"keep");
            assert_eq!(marker::load(&root).unwrap(), Some(marker("2", &new)));
            assert!(!staging.exists());
            assert!(!library.join(".transaction-backup").exists());
            let _ = fs::remove_dir_all(library);
        }
    }
}
