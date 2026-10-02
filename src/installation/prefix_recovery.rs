//! Explicit managed-prefix rebuilding; game setup remains a separate reviewed operation.
use super::{dependency_setup, marker::InstallationMarker, recovery};
use crate::domain::InstalledGame;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    version: u32,
    directory: PathBuf,
    marker: InstallationMarker,
    original: Option<(u64, u64)>,
    backup: Option<PathBuf>,
    fresh: Option<(u64, u64)>,
    fresh_staging: Option<PathBuf>,
    initialized: bool,
    process: Option<dependency_setup::SetupProcessGuard>,
}

pub struct PrefixRebuildPlan {
    pub prefix: PathBuf,
    pub backup: Option<PathBuf>,
    pub setup_required: bool,
    game: InstalledGame,
    marker: InstallationMarker,
    identity: Option<(u64, u64)>,
    payload_identity: (u64, u64),
    session: u64,
    generation: u64,
}

pub struct PrefixRebuildResult {
    pub prefix: PathBuf,
    pub backup: Option<PathBuf>,
}

#[derive(Debug)]
pub(super) struct Pending(pub bool);
impl std::fmt::Display for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0 { "The rebuilt prefix still needs game setup. Choose Finish recovery before playing." } else { "Prefix rebuilding is incomplete. Choose Recover prefix to retry; the retained backup is preserved." })
    }
}
impl std::error::Error for Pending {}

fn prefix(directory: &Path) -> Result<PathBuf> {
    let slug = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid game directory")?;
    crate::compatibility::validate_slug(slug)?;
    ensure!(slug != ".ludomere", "Invalid game directory");
    Ok(crate::compatibility::prefix_path(
        directory.parent().context("Missing library")?,
        slug,
    ))
}

fn record_path(directory: &Path) -> Result<PathBuf> {
    let _ = prefix(directory)?;
    Ok(directory
        .parent()
        .unwrap()
        .join(".ludomere/staging")
        .join(format!(
            "{}.prefix-rebuild.json",
            directory.file_name().unwrap().to_string_lossy()
        )))
}

fn identity(path: &Path) -> Result<Option<(u64, u64)>> {
    match recovery::open_directory(path) {
        Ok(file) => {
            let metadata = file.metadata()?;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() },
                "The prefix or game directory is not owned by this user"
            );
            Ok(Some((metadata.dev(), metadata.ino())))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("Cannot safely inspect prefix recovery paths"),
    }
}

fn load(directory: &Path) -> Result<Option<Record>> {
    recovery::read_json(&record_path(directory)?)?
        .map(|value| {
            let record: Record = serde_json::from_value(value)?;
            ensure!(
                record.version == 1
                    && record.directory == directory
                    && record.marker.slug == directory.file_name().unwrap().to_string_lossy(),
                "Prefix recovery record does not match this installation"
            );
            ensure!(
                record.backup.is_some() == record.original.is_some()
                    && (!record.initialized || record.fresh.is_some()),
                "Incomplete prefix recovery identity"
            );
            if let Some(backup) = &record.backup {
                ensure!(
                    backup.parent()
                        == Some(
                            directory
                                .parent()
                                .unwrap()
                                .join(".ludomere/prefix-backups")
                                .as_path()
                        )
                        && backup.file_name().is_some_and(|name| name
                            .to_string_lossy()
                            .starts_with(&format!("{}-prefix-", record.marker.slug))),
                    "Invalid prefix backup location"
                );
            }
            if let Some(staging) = &record.fresh_staging {
                ensure!(
                    staging.parent() == prefix(directory)?.parent()
                        && staging.file_name().is_some_and(|name| name
                            .to_string_lossy()
                            .starts_with(&format!(".{}-rebuild-", record.marker.slug))),
                    "Invalid staged prefix location"
                );
            }
            if let Some(guard) = &record.process {
                dependency_setup::ensure_process_quiescent(guard)?;
            }
            Ok(record)
        })
        .transpose()
}

pub(super) fn ensure_quiescent(directory: &Path) -> Result<()> {
    load(directory).map(|_| ())
}

pub(super) fn ensure_ready(game: &InstalledGame) -> Result<()> {
    if let Some(record) = load(&game.installation_directory)? {
        ensure!(
            record.marker.product_id == game.product_id,
            "Prefix recovery belongs to another game"
        );
        return Err(Pending(record.initialized).into());
    }
    Ok(())
}

fn create_child(parent: &File, name: &str) -> Result<File> {
    let name_c = std::ffi::CString::new(name)?;
    if unsafe { libc::mkdirat(parent.as_raw_fd(), name_c.as_ptr(), 0o700) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(error.into());
        }
    }
    let child = recovery::open_child(parent, std::ffi::OsStr::new(name), true, false)?;
    parent.sync_all()?;
    Ok(child)
}

fn persist(record: &Record, replace: bool) -> Result<()> {
    let library = recovery::open_directory(record.directory.parent().context("Missing library")?)?;
    let control = create_child(&library, ".ludomere")?;
    let staging = create_child(&control, "staging")?;
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", staging.as_raw_fd()));
    let mut temporary = tempfile::NamedTempFile::new_in(&anchored)?;
    temporary.write_all(&serde_json::to_vec(record)?)?;
    temporary.as_file().sync_all()?;
    let target = anchored.join(record_path(&record.directory)?.file_name().unwrap());
    if replace {
        temporary.persist(target)?;
    } else {
        temporary.persist_noclobber(target)?;
    }
    staging.sync_all()?;
    Ok(())
}

fn validate_owner(path: &Path, slug: &str) -> Result<()> {
    let Some(owner) = recovery::read_json(&path.join(".ludomere-managed.json"))? else {
        // Called only after the installed marker proves this exact managed prefix.
        // Cloud-save downloads can create drive_c before Wine initialization.
        ensure!(
            identity(path)?.is_some() && identity(&path.join("drive_c"))?.is_some(),
            "The existing prefix has no ownership record or attributable incomplete drive_c; it was not changed"
        );
        for name in ["dosdevices", "system.reg", "user.reg", "userdef.reg"] {
            match fs::symlink_metadata(path.join(name)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).context(
                        "Cannot inspect the incomplete prefix; restore access before recovery",
                    );
                }
                Ok(_) => anyhow::bail!(
                    "The existing prefix contains Wine structure but has no trustworthy Ludomere ownership record; it was not changed"
                ),
            }
        }
        return Ok(());
    };
    ensure!(
        owner.get("schema_version").and_then(|value| value.as_u64()) == Some(1)
            && owner
                .get("managed_by_ludomere")
                .and_then(|value| value.as_bool())
                == Some(true)
            && owner.get("slug").and_then(|value| value.as_str()) == Some(slug),
        "The existing prefix ownership does not match this game"
    );
    Ok(())
}

fn damaged(path: &Path) -> Result<bool> {
    let mut missing = false;
    for (name, directory) in [
        ("dosdevices", true),
        ("drive_c", true),
        ("system.reg", false),
        ("user.reg", false),
        ("userdef.reg", false),
    ] {
        match fs::symlink_metadata(path.join(name)) {
            Ok(metadata) => {
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "Prefix structure contains a symlink that cannot be safely attributed to this game"
                );
                missing |= if directory {
                    !metadata.is_dir()
                } else {
                    !metadata.is_file()
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(error) => {
                return Err(error)
                    .context("Cannot inspect the prefix; restore access before recovery");
            }
        }
    }
    Ok(missing)
}

/// Worker-only preview. Missing/corrupt structure is separate from permission or ownership failure.
pub fn prepare_prefix_rebuild(game: &InstalledGame) -> Result<PrefixRebuildPlan> {
    super::validate_game_library(
        &crate::storage::read_config()?,
        &game.library_id,
        &game.installation_directory,
    )?;
    let payload_identity =
        identity(&game.installation_directory)?.context("Installed game directory is missing")?;
    let marker: InstallationMarker = serde_json::from_value(
        recovery::read_json(&super::marker::marker_path(&game.installation_directory))?
            .context("A managed installation marker is required")?,
    )?;
    marker.validate()?;
    let compatibility = marker
        .compatibility
        .as_ref()
        .context("Native games do not use a Windows prefix")?;
    ensure!(
        marker.product_id == game.product_id
            && Some(marker.slug.as_str())
                == game
                    .installation_directory
                    .file_name()
                    .and_then(|name| name.to_str())
            && marker.base.version == game.installed_version
            && marker.base.revision_id == game.installer_revision_id
            && compatibility.managed_by_ludomere
            && compatibility.prefix_slug == marker.slug
            && game
                .compatibility
                .as_ref()
                .is_some_and(|saved| saved.prefix_slug == marker.slug),
        "The installation or managed-prefix identity changed; reopen this action"
    );
    let prefix = prefix(&game.installation_directory)?;
    recovery::validate_prefix_locations(&prefix)?;
    let current = identity(&prefix)?;
    let record = load(&game.installation_directory)?;
    let (backup, setup_required) = if let Some(record) = record {
        ensure!(
            same_source(&record.marker, &marker),
            "The installed source changed since prefix recovery began"
        );
        if let Some(fresh) = record.fresh {
            ensure!(
                current == Some(fresh)
                    || (current.is_none()
                        && record
                            .fresh_staging
                            .as_ref()
                            .is_some_and(|path| identity(path).ok().flatten() == Some(fresh))),
                "The rebuilt prefix changed unexpectedly; no files were changed"
            );
        } else if current.is_some() {
            ensure!(
                current == record.original
                    && record
                        .backup
                        .as_ref()
                        .is_none_or(|path| identity(path).ok().flatten().is_none()),
                "An unexpected prefix replaced the saved recovery target"
            );
        }
        (record.backup, record.initialized)
    } else {
        if current.is_some() {
            validate_owner(&prefix, &marker.slug)?;
            ensure!(
                damaged(&prefix)?,
                "The managed prefix is structurally complete; rebuilding is not needed for this error"
            );
        }
        let backup = current.map(|_| {
            game.installation_directory
                .parent()
                .unwrap()
                .join(".ludomere/prefix-backups")
                .join(format!(
                    "{}-prefix-{}-{}",
                    marker.slug,
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ))
        });
        (backup, false)
    };
    Ok(PrefixRebuildPlan {
        prefix,
        backup,
        setup_required,
        game: game.clone(),
        marker,
        identity: current,
        payload_identity,
        session: crate::online::account_session(),
        generation: recovery::generation(game.product_id),
    })
}

fn save_original(record: &Record) -> Result<()> {
    let Some(backup) = &record.backup else {
        return Ok(());
    };
    let path = prefix(&record.directory)?;
    if identity(backup)? == record.original {
        ensure!(
            identity(&path)?.is_none() || identity(&path)? == record.fresh,
            "An unexpected prefix appeared after the backup"
        );
        return Ok(());
    }
    ensure!(
        identity(backup)?.is_none() && identity(&path)? == record.original,
        "Original prefix or backup identity changed"
    );
    let parent = recovery::open_directory(path.parent().unwrap())?;
    let control = recovery::open_directory(
        record
            .directory
            .parent()
            .unwrap()
            .join(".ludomere")
            .as_path(),
    )?;
    let backups = create_child(&control, "prefix-backups")?;
    ensure!(
        parent.metadata()?.dev() == record.original.unwrap().0
            && backups.metadata()?.dev() == record.original.unwrap().0,
        "Cannot safely rename a mounted prefix or move it across filesystems"
    );
    let name = std::ffi::CString::new(path.file_name().unwrap().as_bytes())?;
    let target = std::ffi::CString::new(backup.file_name().unwrap().as_bytes())?;
    if unsafe {
        libc::renameat2(
            parent.as_raw_fd(),
            name.as_ptr(),
            backups.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    parent.sync_all()?;
    backups.sync_all()?;
    ensure!(
        identity(backup)? == record.original,
        "Prefix backup identity could not be verified"
    );
    Ok(())
}

fn stage_fresh(record: &mut Record) -> Result<()> {
    let path = prefix(&record.directory)?;
    if record.fresh.is_none() {
        ensure!(
            identity(&path)?.is_none(),
            "An unrecorded prefix exists; the original backup remains untouched"
        );
        let library = recovery::open_directory(record.directory.parent().unwrap())?;
        let control = create_child(&library, ".ludomere")?;
        let prefixes = create_child(&control, "compatibility")?;
        let anchored = PathBuf::from(format!("/proc/self/fd/{}", prefixes.as_raw_fd()));
        let staged = tempfile::Builder::new()
            .prefix(&format!(".{}-rebuild-", record.marker.slug))
            .tempdir_in(&anchored)?;
        crate::compatibility::write_ownership(staged.path(), &record.marker.slug)?;
        File::open(staged.path().join(".ludomere-managed.json"))?.sync_all()?;
        File::open(staged.path())?.sync_all()?;
        prefixes.sync_all()?;
        let metadata = fs::metadata(staged.path())?;
        record.fresh = Some((metadata.dev(), metadata.ino()));
        record.fresh_staging = Some(
            path.parent()
                .unwrap()
                .join(staged.path().file_name().unwrap()),
        );
        let _ = staged.keep();
        persist(record, true)?;
    }
    if identity(&path)?.is_none() {
        let staging = record
            .fresh_staging
            .as_ref()
            .context("The recorded new prefix is missing")?;
        ensure!(
            identity(staging)? == record.fresh,
            "Staged prefix identity changed"
        );
        let parent = recovery::open_directory(path.parent().unwrap())?;
        let source = std::ffi::CString::new(staging.file_name().unwrap().as_bytes())?;
        let target = std::ffi::CString::new(path.file_name().unwrap().as_bytes())?;
        if unsafe {
            libc::renameat2(
                parent.as_raw_fd(),
                source.as_ptr(),
                parent.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        parent.sync_all()?;
    }
    ensure!(
        identity(&path)? == record.fresh,
        "The new prefix identity changed"
    );
    Ok(())
}

pub fn rebuild_prefix(
    plan: PrefixRebuildPlan,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(&str),
) -> Result<PrefixRebuildResult> {
    let _activity = crate::profile_reset::begin_activity("rebuilding Windows prefix")?;
    ensure!(
        crate::online::account_session() == plan.session
            && recovery::current(plan.game.product_id, plan.generation),
        "Account or game operation changed; reopen prefix recovery"
    );
    let _reservation = recovery::Reservation::reserve(&[plan.game.product_id])?;
    let _permit = crate::operation_gate::try_acquire().map_err(|_| {
        anyhow::anyhow!("Finish active downloads or installations before rebuilding this prefix")
    })?;
    ensure!(
        !super::manager::recovery_busy(&[plan.game.product_id]),
        "Finish or abandon the pending game operation before rebuilding its prefix"
    );
    let current = prepare_prefix_rebuild(&plan.game)?;
    ensure!(
        current.marker == plan.marker
            && current.payload_identity == plan.payload_identity
            && current.identity == plan.identity,
        "The installation or prefix changed since confirmation"
    );
    let stopped =
        || cancelled.load(Ordering::Acquire) || crate::online::account_session() != plan.session;
    ensure!(
        !stopped(),
        "Prefix recovery cancelled before changing files"
    );
    if current.setup_required {
        return Ok(PrefixRebuildResult {
            prefix: plan.prefix,
            backup: current.backup,
        });
    }
    crate::compatibility::preflight_windows(Some(plan.game.product_id))?;
    let backend = crate::compatibility::backend_for_game(plan.game.product_id)?;
    let mut record = load(&plan.game.installation_directory)?.unwrap_or(Record {
        version: 1,
        directory: plan.game.installation_directory.clone(),
        marker: plan.marker.clone(),
        original: plan.identity,
        backup: plan.backup.clone(),
        fresh: None,
        fresh_staging: None,
        initialized: false,
        process: None,
    });
    let result = (|| -> Result<()> {
        if load(&record.directory)?.is_none() {
            persist(&record, false)?;
        }
        ensure!(
            !stopped(),
            "Prefix recovery cancelled before saving the old prefix"
        );
        progress("Backing up your previous Windows setup…");
        crate::online::with_account_session(plan.session, || save_original(&record))?;
        crate::online::with_account_session(plan.session, || stage_fresh(&mut record))?;
        ensure!(
            !stopped(),
            "Prefix recovery cancelled; the backup is retained"
        );
        progress("Creating a new Windows environment…");
        let log = super::executor::installation_log_path(plan.game.product_id)?;
        backend.rebuild_prefix_controlled(
            crate::compatibility::InitializePrefixRequest {
                library_id: plan.game.library_id.clone(),
                library: plan
                    .game
                    .installation_directory
                    .parent()
                    .unwrap()
                    .to_owned(),
                slug: plan.marker.slug.clone(),
                profile: plan.marker.compatibility.as_ref().unwrap().profile.clone(),
                log_path: log,
            },
            |command, log| {
                let result = dependency_setup::run_guarded(
                    &stopped,
                    "Prefix rebuilding",
                    log,
                    |guard| {
                        record.process = guard;
                        persist(&record, true)
                    },
                    || {
                        Ok(crate::compatibility::CompatibilityProcess::spawn(
                            command, log,
                        )?)
                    },
                );
                // UMU creates the prefix before the initialization command exits. Accept only
                // a drained nonzero exit with all structural postconditions, never spawn,
                // cancellation, persistence or process-inspection failures.
                if result
                    .as_ref()
                    .is_err_and(|error| error.is::<dependency_setup::UnsuccessfulExit>())
                    && !stopped()
                    && crate::compatibility::validate_prefix_structure(&plan.prefix).is_ok()
                {
                    Ok(())
                } else {
                    result
                }
            },
        )?;
        ensure!(
            !stopped() && identity(&plan.prefix)? == record.fresh,
            "Prefix rebuilding was interrupted; finish recovery before playing"
        );
        crate::compatibility::validate_prefix_structure(&plan.prefix)?;
        record.initialized = true;
        persist(&record, true)?;
        Ok(())
    })();
    result.with_context(|| match &record.backup { Some(path) if identity(path).ok().flatten() == record.original => format!("Prefix recovery is incomplete. The original prefix backup is retained at {}. Reopen recovery to retry; the game payload was not removed", path.display()), _ => "Prefix recovery is incomplete. Reopen recovery to retry; the original prefix and game payload were not deleted".into() })?;
    progress("Windows environment created. Continue setup to install required game components.");
    Ok(PrefixRebuildResult {
        prefix: plan.prefix,
        backup: record.backup,
    })
}

pub(super) struct SetupTicket(Record, u64);

pub(super) fn setup_ticket(
    directory: &Path,
    target: &InstallationMarker,
    repair: bool,
) -> Result<Option<SetupTicket>> {
    let Some(record) = load(directory)? else {
        return Ok(None);
    };
    ensure!(
        repair && record.initialized && record.fresh == identity(&prefix(directory)?)?,
        "Finish prefix rebuilding, then use Repair for the installed version before changing this game"
    );
    ensure!(
        same_source(&record.marker, target),
        "Prefix recovery requires the installed source and version, not an update. For offline repair, select that version and all previously installed DLC."
    );
    crate::compatibility::validate_prefix_structure(&prefix(directory)?)?;
    Ok(Some(SetupTicket(record, crate::online::account_session())))
}

fn same_source(left: &InstallationMarker, right: &InstallationMarker) -> bool {
    left.product_id == right.product_id
        && left.slug == right.slug
        && left.source == right.source
        && left.base.operating_system == right.base.operating_system
        && left.base.language == right.base.language
        && left.base.version == right.base.version
        && left.base.revision_id == right.base.revision_id
        && match (&left.galaxy_depot, &right.galaxy_depot) {
            (Some(a), Some(b)) => {
                a.build_id == b.build_id
                    && a.repository_id == b.repository_id
                    && a.branch == b.branch
                    && a.language == b.language
                    && a.architecture == b.architecture
                    && a.depots == b.depots
                    && a.dlc == b.dlc
            }
            (None, None) => {
                left.dlc
                    .iter()
                    .map(|dlc| (dlc.product_id, dlc.version.as_deref(), dlc.revision_id))
                    .collect::<std::collections::BTreeSet<_>>()
                    == right
                        .dlc
                        .iter()
                        .map(|dlc| (dlc.product_id, dlc.version.as_deref(), dlc.revision_id))
                        .collect::<std::collections::BTreeSet<_>>()
            }
            _ => false,
        }
}

pub(super) fn setup_completed(
    ticket: Option<SetupTicket>,
    marker: &InstallationMarker,
    publish_marker: bool,
) -> Result<()> {
    let Some(SetupTicket(expected, session)) = ticket else {
        return Ok(());
    };
    ensure!(
        crate::online::account_session() == session,
        "Account changed before prefix setup completion"
    );
    let current = load(&expected.directory)?
        .context("Prefix recovery record disappeared before setup completed")?;
    ensure!(
        current.fresh == expected.fresh
            && current.marker == expected.marker
            && same_source(&expected.marker, marker)
            && identity(&prefix(&expected.directory)?)? == expected.fresh,
        "Prefix or source changed during setup; recovery remains pending"
    );
    crate::compatibility::validate_prefix_structure(&prefix(&expected.directory)?)?;
    let path = record_path(&expected.directory)?;
    let parent = recovery::open_directory(path.parent().unwrap())?;
    let name = std::ffi::CString::new(path.file_name().unwrap().as_bytes())?;
    crate::online::with_account_session(session, || {
        if publish_marker {
            super::marker::write(marker, &expected.directory)?;
        }
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        parent.sync_all()?;
        Ok(())
    })
}

pub(super) fn run_setup(
    ticket: &SetupTicket,
    cancelled: &AtomicBool,
    log: &Path,
    spawn: impl FnOnce() -> Result<crate::compatibility::CompatibilityProcess>,
) -> Result<()> {
    let mut record = load(&ticket.0.directory)?.context("Prefix recovery record disappeared")?;
    ensure!(
        record.fresh == ticket.0.fresh && record.marker == ticket.0.marker,
        "Prefix recovery changed before setup"
    );
    dependency_setup::run_guarded(
        &|| cancelled.load(Ordering::Acquire) || crate::online::account_session() != ticket.1,
        "Game setup for rebuilt prefix",
        log,
        |guard| {
            record.process = guard;
            persist(&record, true)
        },
        spawn,
    )
}

pub(super) fn retire_after_uninstall(directory: &Path, product_id: i64) -> Result<()> {
    let Some(record) = load(directory)? else {
        return Ok(());
    };
    ensure!(
        record.marker.product_id == product_id && identity(&prefix(directory)?)?.is_none(),
        "Prefix recovery cannot be retired before this game's prefix removal"
    );
    recovery::remove_control_file(&record_path(directory)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_launch_failure_offers_scaffold_recovery_and_reports_unsafe_rejections() {
        const CHILD: &str = "LUDOMERE_PREFIX_OFFER_FIXTURE";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "installation::prefix_recovery::tests::actual_launch_failure_offers_scaffold_recovery_and_reports_unsafe_rejections", "--nocapture"])
                .env(CHILD, "1");
            for key in [
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR",
                "TMPDIR",
            ] {
                let path = root.path().join(key);
                fs::create_dir(&path).unwrap();
                command.env(key, path);
            }
            assert!(command.status().unwrap().success());
            return;
        }
        let (root, record, game) = fixture();
        crate::config::Config {
            game_libraries: vec![crate::config::GameLibrary {
                id: game.library_id.clone(),
                name: "Private fixture".into(),
                path: root.path().to_owned(),
                default: true,
            }],
            ..Default::default()
        }
        .save()
        .unwrap();
        let live = prefix(&game.installation_directory).unwrap();
        fs::remove_file(live.join(".ludomere-managed.json")).unwrap();
        fs::create_dir(live.join("drive_c")).unwrap();
        // This is the actual backend error, with no runner/helper needed.
        assert!(matches!(
            crate::compatibility::configure_library_drive(&live, root.path()),
            Err(crate::compatibility::CompatibilityFailure::PrefixCorrupt(_))
        ));
        let launch = |game: InstalledGame| {
            let id = game.product_id;
            let event = super::super::launcher::launch_game(game)
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while super::super::launcher::is_game_running(id)
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!super::super::launcher::is_game_running(id));
            event
        };
        match launch(game.clone()) {
            super::super::launcher::LaunchEvent::PrefixRecoveryRequired {
                message,
                setup_required,
                ..
            } => {
                assert!(message.contains("PrefixCorrupt"));
                assert!(!setup_required);
            }
            event => panic!("expected real launch to offer recovery, got {event:?}"),
        }
        let rejected = |game: InstalledGame, reason: &str| match launch(game) {
            super::super::launcher::LaunchEvent::Failed(message) => {
                assert!(message.contains("PrefixCorrupt"), "{message}");
                assert!(
                    message.contains("Prefix recovery is unavailable"),
                    "{message}"
                );
                assert!(message.contains(reason), "{message}");
            }
            event => panic!("expected actionable refusal, got {event:?}"),
        };
        let mut mismatched = game.clone();
        mismatched.installed_version = Some("different".into());
        rejected(mismatched, "identity changed");
        fs::write(live.join("system.reg"), b"inert fixture").unwrap();
        rejected(game.clone(), "no trustworthy Ludomere ownership");
        fs::remove_file(live.join("system.reg")).unwrap();
        fs::write(live.join(".ludomere-managed.json"), b"not JSON").unwrap();
        rejected(game.clone(), "Reading game recovery metadata");
        fs::remove_file(live.join(".ludomere-managed.json")).unwrap();
        crate::compatibility::write_ownership(&live, "wrong-game").unwrap();
        rejected(game.clone(), "ownership does not match");
        fs::remove_file(live.join(".ludomere-managed.json")).unwrap();
        fs::remove_dir(live.join("drive_c")).unwrap();
        std::os::unix::fs::symlink(root.path(), live.join("drive_c")).unwrap();
        rejected(game.clone(), "Prefix recovery is unavailable");
        fs::remove_file(live.join("drive_c")).unwrap();
        fs::create_dir(live.join("drive_c")).unwrap();
        assert!(prepare_prefix_rebuild(&game).is_ok());
        persist(&record, false).unwrap();
        save_original(&record).unwrap();
        assert!(!live.exists());
        assert_eq!(
            fs::read(record.backup.unwrap().join("save-sentinel")).unwrap(),
            b"preserve old saves"
        );
        assert!(record.directory.join("game.exe").is_file());
    }

    fn fixture() -> (tempfile::TempDir, Record, InstalledGame) {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("game");
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("game.exe"),
            b"inert game payload; never execute",
        )
        .unwrap();
        let marker = InstallationMarker {
            schema_version: 2,
            product_id: 217,
            slug: "game".into(),
            base: super::super::marker::InstalledComponent {
                operating_system: Some("windows".into()),
                language: Some("en-US".into()),
                version: Some("1".into()),
                revision_id: Some(3),
                installed_at: 1,
            },
            dlc: Vec::new(),
            compatibility: Some(super::super::marker::InstalledCompatibility {
                backend: crate::compatibility::CompatibilityBackendKind::Umu,
                managed_by_ludomere: true,
                prefix_slug: "game".into(),
                profile: crate::compatibility::UmuProfile::fallback(),
            }),
            source: crate::domain::InstallationSource::OfflineInstaller,
            galaxy_depot: None,
            launch: None,
            dependencies: Vec::new(),
        };
        super::super::marker::write(&marker, &directory).unwrap();
        let prefix = prefix(&directory).unwrap();
        fs::create_dir_all(&prefix).unwrap();
        crate::compatibility::write_ownership(&prefix, "game").unwrap();
        fs::write(prefix.join("save-sentinel"), b"preserve old saves").unwrap();
        let game = super::super::marker::game_from_marker(
            &marker,
            "private".into(),
            directory.clone(),
            Some(directory.join("game.exe")),
        );
        let record = Record {
            version: 1,
            directory,
            marker,
            original: identity(&prefix).unwrap(),
            backup: Some(
                root.path()
                    .join(".ludomere/prefix-backups/game-prefix-test"),
            ),
            fresh: None,
            fresh_staging: None,
            initialized: false,
            process: None,
        };
        (root, record, game)
    }

    fn finish_structure(record: &mut Record) {
        let prefix = prefix(&record.directory).unwrap();
        for name in ["drive_c", "dosdevices"] {
            fs::create_dir(prefix.join(name)).unwrap();
        }
        for name in ["system.reg", "user.reg", "userdef.reg"] {
            fs::write(prefix.join(name), "inert registry").unwrap();
        }
        record.initialized = true;
        persist(record, true).unwrap();
    }

    #[test]
    fn backup_and_staged_publication_resume_without_touching_original_saves_or_payload() {
        let (_root, mut record, game) = fixture();
        let original_marker =
            fs::read(super::super::marker::marker_path(&record.directory)).unwrap();
        persist(&record, false).unwrap();
        save_original(&record).unwrap();
        // The prearmed record is sufficient after a crash immediately following rename.
        record = load(&record.directory).unwrap().unwrap();
        save_original(&record).unwrap();
        stage_fresh(&mut record).unwrap();
        let fresh = record.fresh;
        let live = prefix(&record.directory).unwrap();
        // Recreate the other interruption boundary: staged identity persisted, not published.
        fs::rename(&live, record.fresh_staging.as_ref().unwrap()).unwrap();
        record = load(&record.directory).unwrap().unwrap();
        stage_fresh(&mut record).unwrap();
        assert_eq!(identity(&live).unwrap(), fresh);
        assert!(ensure_ready(&game).unwrap_err().is::<Pending>());
        assert_eq!(
            fs::read(record.backup.as_ref().unwrap().join("save-sentinel")).unwrap(),
            b"preserve old saves"
        );
        assert_eq!(
            fs::read(record.directory.join("game.exe")).unwrap(),
            b"inert game payload; never execute"
        );
        assert_eq!(
            fs::read(super::super::marker::marker_path(&record.directory)).unwrap(),
            original_marker
        );
    }

    #[test]
    fn zero_dependency_prefix_stays_pending_until_matching_setup_and_rejects_changed_prefix() {
        let (_root, mut record, game) = fixture();
        persist(&record, false).unwrap();
        save_original(&record).unwrap();
        stage_fresh(&mut record).unwrap();
        finish_structure(&mut record);
        assert!(record.marker.dependencies.is_empty());
        assert!(
            ensure_ready(&game)
                .unwrap_err()
                .downcast_ref::<Pending>()
                .unwrap()
                .0
        );
        assert!(setup_ticket(&record.directory, &record.marker, false).is_err());
        let mut update = record.marker.clone();
        update.base.version = Some("2".into());
        assert!(setup_ticket(&record.directory, &update, true).is_err());
        let ticket = setup_ticket(&record.directory, &record.marker, true).unwrap();
        let live = prefix(&record.directory).unwrap();
        let held = live.with_file_name("held-new-prefix");
        fs::rename(&live, &held).unwrap();
        fs::create_dir(&live).unwrap();
        assert!(setup_completed(ticket, &record.marker, false).is_err());
        assert!(record_path(&record.directory).unwrap().exists());
        fs::remove_dir(&live).unwrap();
        fs::rename(held, live).unwrap();
        let ticket = setup_ticket(&record.directory, &record.marker, true).unwrap();
        setup_completed(ticket, &record.marker, false).unwrap();
        ensure_ready(&game).unwrap();
        assert!(record.backup.unwrap().join("save-sentinel").is_file());
    }

    #[test]
    fn pending_setup_arms_guard_before_spawn_and_refuses_unknown_process_after_restart() {
        let (_root, mut record, _game) = fixture();
        persist(&record, false).unwrap();
        save_original(&record).unwrap();
        stage_fresh(&mut record).unwrap();
        finish_structure(&mut record);
        let ticket = setup_ticket(&record.directory, &record.marker, true)
            .unwrap()
            .unwrap();
        let result = run_setup(
            &ticket,
            &AtomicBool::new(false),
            &record.directory.join("inert.log"),
            || {
                let value = recovery::read_json(&record_path(&record.directory)?)
                    .unwrap()
                    .unwrap();
                assert!(value.get("process").unwrap().is_object());
                assert!(value.pointer("/process/group").unwrap().is_null());
                anyhow::bail!("synthetic spawn failure; no child executed")
            },
        );
        assert!(result.is_err());
        assert!(load(&record.directory).unwrap().unwrap().process.is_none());
        record.process = Some(dependency_setup::SetupProcessGuard {
            boot: dependency_setup::boot_identity().unwrap(),
            group: None,
        });
        persist(&record, true).unwrap();
        assert!(
            ensure_quiescent(&record.directory)
                .unwrap_err()
                .to_string()
                .contains("Reboot")
        );
        assert!(retire_after_uninstall(&record.directory, record.marker.product_id).is_err());
    }

    #[test]
    fn successful_uninstall_retires_only_matching_pending_record_and_keeps_backup() {
        let (_root, mut record, _game) = fixture();
        persist(&record, false).unwrap();
        save_original(&record).unwrap();
        stage_fresh(&mut record).unwrap();
        finish_structure(&mut record);
        assert!(retire_after_uninstall(&record.directory, record.marker.product_id).is_err());
        fs::remove_dir_all(prefix(&record.directory).unwrap()).unwrap();
        assert!(retire_after_uninstall(&record.directory, 999).is_err());
        retire_after_uninstall(&record.directory, record.marker.product_id).unwrap();
        assert!(load(&record.directory).unwrap().is_none());
        assert!(record.backup.unwrap().join("save-sentinel").is_file());
    }

    #[test]
    fn ownership_symlinks_bad_metadata_and_replaced_prefix_fail_closed() {
        let (_root, record, _game) = fixture();
        let live = prefix(&record.directory).unwrap();
        assert!(damaged(&live).unwrap());
        validate_owner(&live, "game").unwrap();
        assert!(validate_owner(&live, "another-game").is_err());
        std::os::unix::fs::symlink("/", live.join("dosdevices")).unwrap();
        assert!(damaged(&live).is_err());
        persist(&record, false).unwrap();
        let held = live.with_file_name("held-old-prefix");
        fs::rename(&live, held).unwrap();
        fs::create_dir(&live).unwrap();
        assert!(save_original(&record).is_err());
        let path = record_path(&record.directory).unwrap();
        fs::write(&path, b"not JSON").unwrap();
        assert!(load(&record.directory).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(record.directory.join("game.exe"), &path).unwrap();
        assert!(load(&record.directory).is_err());
    }

    #[test]
    fn offline_recovery_requires_all_installed_dlc_independent_of_order() {
        let (_root, mut record, game) = fixture();
        let mut original = record.marker.clone();
        original.dlc = vec![
            super::super::marker::InstalledDlc {
                product_id: 1,
                version: Some("1".into()),
                revision_id: Some(11),
                installed_at: 1,
            },
            super::super::marker::InstalledDlc {
                product_id: 2,
                version: Some("2".into()),
                revision_id: Some(22),
                installed_at: 2,
            },
        ];
        let mut repaired = original.clone();
        repaired.dlc.reverse();
        repaired.dlc[0].installed_at = 99;
        assert!(same_source(&original, &repaired));
        repaired.dlc[0].revision_id = Some(23);
        assert!(!same_source(&original, &repaired));
        repaired.dlc.remove(0);
        assert!(!same_source(&original, &repaired));

        record.marker = original.clone();
        super::super::marker::write(&original, &record.directory).unwrap();
        let marker_path = super::super::marker::marker_path(&record.directory);
        let original_bytes = fs::read(&marker_path).unwrap();
        persist(&record, false).unwrap();
        save_original(&record).unwrap();
        stage_fresh(&mut record).unwrap();
        finish_structure(&mut record);
        // A failed attempt after the base or only one DLC must leave the complete
        // installed source available for another explicit repair attempt.
        for completed_dlcs in [0, 1] {
            let ticket = setup_ticket(&record.directory, &original, true).unwrap();
            let mut incomplete = original.clone();
            incomplete.dlc.truncate(completed_dlcs);
            assert!(setup_completed(ticket, &incomplete, true).is_err());
            assert_eq!(fs::read(&marker_path).unwrap(), original_bytes);
            assert!(ensure_ready(&game).unwrap_err().is::<Pending>());
            assert!(setup_ticket(&record.directory, &original, true).is_ok());
        }
        let ticket = setup_ticket(&record.directory, &original, true).unwrap();
        let mut completed = original;
        completed.dlc.reverse();
        completed.base.installed_at = 99;
        setup_completed(ticket, &completed, true).unwrap();
        assert_eq!(
            super::super::marker::load(&record.directory).unwrap(),
            Some(completed)
        );
        ensure_ready(&game).unwrap();
        assert!(record.backup.unwrap().join("save-sentinel").is_file());
    }
}
