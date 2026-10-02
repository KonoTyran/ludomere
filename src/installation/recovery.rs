//! Explicit, per-game recovery of incomplete installations.
use anyhow::{Context, Result, ensure};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        LazyLock, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct Admissions {
    blocked: HashSet<i64>,
    generations: HashMap<i64, u64>,
}
static ADMISSIONS: LazyLock<Mutex<Admissions>> =
    LazyLock::new(|| Mutex::new(Admissions::default()));
pub(crate) struct Admission {
    _guard: MutexGuard<'static, Admissions>,
}
pub(crate) fn generation(id: i64) -> u64 {
    *ADMISSIONS
        .lock()
        .unwrap()
        .generations
        .get(&id)
        .unwrap_or(&0)
}
pub(crate) fn try_admit(id: i64) -> Result<Admission> {
    let guard = ADMISSIONS
        .try_lock()
        .map_err(|_| anyhow::anyhow!("Game operations are busy; retry Play shortly"))?;
    ensure!(
        !guard.blocked.contains(&id),
        "This game's files are being reset; wait for recovery to finish"
    );
    Ok(Admission { _guard: guard })
}
pub(crate) fn admit_generation(id: i64, generation: u64) -> Result<Admission> {
    let guard = ADMISSIONS.lock().unwrap();
    ensure!(
        !guard.blocked.contains(&id) && *guard.generations.get(&id).unwrap_or(&0) == generation,
        "This download predates the game's recovery; start it again explicitly"
    );
    Ok(Admission { _guard: guard })
}
pub(crate) fn current(id: i64, generation: u64) -> bool {
    let guard = ADMISSIONS.lock().unwrap();
    !guard.blocked.contains(&id) && *guard.generations.get(&id).unwrap_or(&0) == generation
}
pub(crate) fn admit_stamps(stamps: &[(i64, u64)]) -> Result<Admission> {
    let guard = ADMISSIONS.lock().unwrap();
    ensure!(
        stamps
            .iter()
            .all(|(id, generation)| !guard.blocked.contains(id)
                && *guard.generations.get(id).unwrap_or(&0) == *generation),
        "Game recovery invalidated this pending download; start it again explicitly"
    );
    Ok(Admission { _guard: guard })
}
pub(crate) struct Reservation(Vec<i64>);
impl Reservation {
    pub(crate) fn reserve(ids: &[i64]) -> Result<Self> {
        let mut guard = ADMISSIONS.lock().unwrap();
        ensure!(
            ids.iter().all(|id| !guard.blocked.contains(id)),
            "Recovery is already running for this game"
        );
        ensure!(
            ids.iter().all(|id| !super::is_game_running(*id)),
            "Stop the running game before resetting its files"
        );
        for id in ids {
            guard.blocked.insert(*id);
            *guard.generations.entry(*id).or_default() += 1;
        }
        Ok(Self(ids.to_vec()))
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut guard = ADMISSIONS.lock().unwrap();
        for id in &self.0 {
            guard.blocked.remove(id);
        }
    }
}

pub enum UninstallPreparation {
    Normal(crate::domain::InstalledGame),
    Recovery(GameResetPlan),
}
#[derive(Clone)]
pub struct GameResetPlan {
    pub product_id: i64,
    pub directories: Vec<PathBuf>,
    pub prefixes: Vec<PathBuf>,
    pub downloaded_files: usize,
    pub downloaded_bytes: u64,
    ids: Vec<i64>,
    config: crate::config::Config,
    slug: String,
    identities: Vec<Option<(u64, u64)>>,
    prefix_identities: Vec<Option<(u64, u64)>>,
    prefix_checks: Vec<bool>,
    explicit_directory: bool,
    session: u64,
}
#[derive(Default)]
pub struct GameResetResult {
    pub removed_directories: usize,
    pub removed_prefixes: usize,
    pub retained_downloads: usize,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReadiness {
    RestartRequired,
    Ready,
}

#[derive(Debug)]
pub struct DamagedOperationRecord;
impl std::fmt::Display for DamagedOperationRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("This game's operation record is damaged. Choose Prepare recovery, then restart the computer before retrying; no game files were removed")
    }
}
impl std::error::Error for DamagedOperationRecord {}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RebootRecovery {
    version: u32,
    product_id: i64,
    directory: PathBuf,
    root_identity: (u64, u64),
    directory_identity: (u64, u64),
    journal_identity: (u64, u64),
    journal_hash: String,
    boot: String,
}

/// Explicit recovery preparation only. A damaged process journal cannot prove
/// quiescence on this boot. Preserve it and require a new boot before quarantine;
/// payload deletion still needs a separate, freshly reviewed reset confirmation.
pub fn prepare_game_directory_recovery(
    config: &crate::config::Config,
    product_id: i64,
    slug: &str,
    library_id: &str,
) -> Result<RecoveryReadiness> {
    prepare_directory_recovery_at_boot(
        config,
        product_id,
        slug,
        library_id,
        &super::dependency_setup::boot_identity()?,
    )
}

fn prepare_directory_recovery_at_boot(
    config: &crate::config::Config,
    product_id: i64,
    slug: &str,
    library_id: &str,
    boot: &str,
) -> Result<RecoveryReadiness> {
    use sha2::{Digest, Sha256};
    use std::{
        io::{Read, Write},
        os::{fd::AsRawFd, unix::fs::MetadataExt},
    };
    let _activity = crate::profile_reset::begin_activity("preparing damaged game recovery")?;
    let _reservation = Reservation::reserve(&[product_id])?;
    let _permit = crate::operation_gate::try_acquire()
        .context("Pause active operations before preparing recovery")?;
    ensure!(
        !super::manager::recovery_busy(&[product_id]),
        "Wait for this game's operations to stop before preparing recovery"
    );
    let session = crate::online::account_session();
    let game = crate::state::StateStore::open()?
        .cached_product_game(product_id)?
        .context("Load this game's library information before preparing recovery")?;
    ensure!(
        product_id > 0 && game.slug == slug,
        "The game identity changed; reopen recovery"
    );
    let library = config
        .game_libraries
        .iter()
        .find(|library| library.id == library_id)
        .context("The selected Game Files library is no longer configured")?;
    let directory = library.path.join(slug);
    crate::storage::validate_game_directory_location(config, &directory)?;
    super::prefix_recovery::ensure_quiescent(&directory)?;
    let mut ids = vec![product_id];
    ids.extend(
        game.dlcs
            .iter()
            .filter(|dlc| dlc.owned)
            .map(|dlc| dlc.product_id),
    );
    ensure_no_foreign_game(&directory, product_id, &ids)?;
    let root = open_directory(&library.path)?;
    let payload = open_child(&root, std::ffi::OsStr::new(slug), true, true)?;
    let root_metadata = root.metadata()?;
    let payload_metadata = payload.metadata()?;
    let root_identity = (root_metadata.dev(), root_metadata.ino());
    let directory_identity = (payload_metadata.dev(), payload_metadata.ino());
    let path = super::operation_journal::path(&library.path, slug)?;
    let parent = open_directory(path.parent().unwrap())?;
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()));
    let barrier_name = format!("{slug}.reboot-recovery.json");
    let prior = read_json(&path.with_file_name(&barrier_name))?
        .map(serde_json::from_value::<RebootRecovery>)
        .transpose()?;
    if let Some(prior) = &prior {
        ensure!(
            prior.version == 1
                && prior.product_id == product_id
                && prior.journal_hash.len() == 64
                && prior.journal_hash.bytes().all(|b| b.is_ascii_hexdigit())
                && prior.boot.len() == 36
                && prior.boot.bytes().enumerate().all(|(index, value)| {
                    if [8, 13, 18, 23].contains(&index) {
                        value == b'-'
                    } else {
                        value.is_ascii_hexdigit()
                    }
                }),
            "The recovery target changed. No files were changed; reopen recovery and review the directory"
        );
    }
    let journal = match open_child(&parent, path.file_name().unwrap(), false, false) {
        Ok(file) => Some(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && prior.is_some() => None,
        Err(error) => return Err(error).context(
            "There is no damaged operation record to prepare; use the normal game recovery action",
        ),
    };
    let read_bytes = |mut file: std::fs::File| -> Result<Vec<u8>> {
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.nlink() == 1 && metadata.len() <= 64 * 1024 * 1024,
            "Unsafe or oversized operation record; it was not changed"
        );
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            "Operation record grew beyond the safety limit"
        );
        Ok(bytes)
    };
    let Some(journal) = journal else {
        let prior = prior.unwrap();
        ensure!(
            prior.directory == directory
                && prior.root_identity == root_identity
                && prior.directory_identity == directory_identity,
            "The recovery target changed; review the new directory before resetting it"
        );
        ensure!(
            prior.boot != boot,
            "Restart the computer before retrying recovery"
        );
        let backup = open_child(
            &parent,
            std::ffi::OsStr::new(&format!(
                "{slug}.operation-quarantine-{}.json",
                prior.journal_hash
            )),
            false,
            false,
        )?;
        ensure!(
            format!("{:x}", Sha256::digest(read_bytes(backup)?)) == prior.journal_hash,
            "The operation recovery copy changed; no files were removed"
        );
        return Ok(RecoveryReadiness::Ready);
    };
    let metadata = journal.metadata()?;
    let journal_identity = (metadata.dev(), metadata.ino());
    let bytes = read_bytes(journal)?;
    let journal_hash = format!("{:x}", Sha256::digest(&bytes));
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if let Some(id) = value
            .pointer("/record/product_id")
            .and_then(serde_json::Value::as_i64)
        {
            ensure!(
                id == product_id,
                "Another game's operation owns this record"
            );
        }
        if let Some(version) = value.get("version").and_then(serde_json::Value::as_u64) {
            ensure!(
                version == 1,
                "Unsupported operation record version; use a compatible Ludomere version"
            );
        }
    }
    if let Ok(operation) =
        serde_json::from_slice::<super::operation_journal::OperationJournal>(&bytes)
    {
        let plan = match operation {
            super::operation_journal::OperationJournal::Depot { record, .. } => {
                ensure!(
                    record.destination == directory,
                    "The operation belongs to a different directory"
                );
                record.plan_json
            }
            super::operation_journal::OperationJournal::Offline { record, .. } => record.plan_json,
        };
        ensure!(
            serde_json::from_str::<serde_json::Value>(&plan).is_err(),
            "This operation record is readable. Use Resume or normal recovery; restart the computer first if its setup process is uncertain"
        );
    }
    let replace_barrier = prior.is_some();
    let prior = prior.filter(|prior| {
        prior.directory == directory
            && prior.root_identity == root_identity
            && prior.directory_identity == directory_identity
            && prior.journal_identity == journal_identity
            && prior.journal_hash == journal_hash
    });
    let Some(prior) = prior else {
        let record = RebootRecovery {
            version: 1,
            product_id,
            directory,
            root_identity,
            directory_identity,
            journal_identity,
            journal_hash,
            boot: boot.into(),
        };
        let mut temporary = tempfile::NamedTempFile::new_in(&anchored)?;
        serde_json::to_writer(&mut temporary, &record)?;
        temporary.as_file().sync_all()?;
        crate::online::with_account_session(session, || {
            ensure!(
                read_config()?.game_libraries == config.game_libraries,
                "Game libraries changed; reopen recovery"
            );
            if replace_barrier {
                temporary.persist(anchored.join(&barrier_name))?;
            } else {
                temporary.persist_noclobber(anchored.join(&barrier_name))?;
            }
            parent.sync_all()?;
            Ok(())
        })?;
        return Ok(RecoveryReadiness::RestartRequired);
    };
    if prior.boot == boot {
        return Ok(RecoveryReadiness::RestartRequired);
    }
    let backup_name = format!("{slug}.operation-quarantine-{journal_hash}.json");
    match open_child(&parent, std::ffi::OsStr::new(&backup_name), false, false) {
        Ok(backup) => ensure!(
            read_bytes(backup)? == bytes,
            "The operation recovery copy does not match"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut temporary = tempfile::NamedTempFile::new_in(&anchored)?;
            temporary.write_all(&bytes)?;
            temporary.as_file().sync_all()?;
            temporary.persist_noclobber(anchored.join(&backup_name))?;
            parent.sync_all()?;
        }
        Err(error) => return Err(error.into()),
    }
    let current = open_child(&parent, path.file_name().unwrap(), false, false)?;
    let metadata = current.metadata()?;
    ensure!(
        (metadata.dev(), metadata.ino()) == journal_identity && read_bytes(current)? == bytes,
        "The operation changed during recovery; it was retained"
    );
    crate::online::with_account_session(session, || {
        ensure!(
            read_config()?.game_libraries == config.game_libraries,
            "Game libraries changed; reopen recovery"
        );
        unlink(&parent, path.file_name().unwrap(), false)?;
        parent.sync_all()?;
        Ok(())
    })?;
    Ok(RecoveryReadiness::Ready)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Receipt {
    version: u32,
    product_id: i64,
    directory: PathBuf,
    identity: Option<(u64, u64)>,
    #[serde(default)]
    prefix_identity: Option<(u64, u64)>,
    #[serde(default)]
    setup_process: Option<super::dependency_setup::SetupProcessGuard>,
}

pub(super) fn receipt_path(directory: &Path) -> Result<PathBuf> {
    Ok(directory
        .parent()
        .context("Game has no library")?
        .join(".ludomere/staging")
        .join(format!(
            "{}.recovery.json",
            directory
                .file_name()
                .and_then(|name| name.to_str())
                .context("Invalid game name")?
        )))
}

fn receipt(directory: &Path, product_id: i64) -> Result<Option<Receipt>> {
    read_json(&receipt_path(directory)?)?
        .map(|value| {
            let receipt: Receipt = serde_json::from_value(value)?;
            ensure!(
                receipt.version == 1
                    && receipt.product_id == product_id
                    && receipt.directory == directory,
                "Recovery record does not match this game; no files were removed"
            );
            Ok(receipt)
        })
        .transpose()
}

pub(crate) fn pending(directory: &Path, product_id: i64) -> Result<bool> {
    Ok(receipt(directory, product_id)?.is_some())
}

// Broken JSON is not ownership evidence. Filesystem errors remain errors, not absence.
fn ensure_no_foreign_game(directory: &Path, product_id: i64, ids: &[i64]) -> Result<()> {
    if let Some(marker) = optional_metadata(&super::marker::marker_path(directory))? {
        let marker_id = marker
            .get("product_id")
            .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()));
        ensure!(
            marker_id.is_none_or(|id| id == product_id),
            "The installation marker belongs to another game"
        );
    }
    for (index, entry) in std::fs::read_dir(directory)?.enumerate() {
        ensure!(
            index < 100_000,
            "Game metadata inspection exceeds its bounded entry limit"
        );
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_prefix("goggame-"))
            .and_then(|name| name.strip_suffix(".info"))
            .and_then(|name| name.parse::<i64>().ok())
        else {
            continue;
        };
        ensure!(
            ids.contains(&id),
            "This directory contains metadata for another game; choose that game's own recovery action"
        );
    }
    Ok(())
}

fn optional_metadata(path: &Path) -> Result<Option<serde_json::Value>> {
    match read_json(path) {
        Err(error) if error.downcast_ref::<serde_json::Error>().is_some() => Ok(None),
        result => result,
    }
}

fn valid_download_destination(
    destination: &Path,
    directory: &Path,
    artifacts: &[crate::domain::RemoteArtifact],
) -> bool {
    if artifacts.is_empty() {
        return false;
    }
    let Ok(relative) = destination.strip_prefix(directory) else {
        return false;
    };
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return false;
    }
    let expected = crate::download::destination(Path::new("/"), "game", None, &[&artifacts[0]]);
    let expected = expected.strip_prefix("/game").unwrap();
    relative == expected
        || relative
            .strip_prefix("dlc")
            .ok()
            .and_then(|path| {
                let mut parts = path.components();
                parts.next()?;
                Some(parts.as_path() == expected)
            })
            .unwrap_or(false)
}

fn write_receipt(
    directory: &Path,
    product_id: i64,
    identity: Option<(u64, u64)>,
    prefix_identity: Option<(u64, u64)>,
) -> Result<()> {
    if let Some(existing) = receipt(directory, product_id)? {
        ensure!(
            existing.identity == identity || identity.is_none(),
            "Recovery directory identity changed"
        );
        ensure!(
            prefix_identity.is_none() || existing.prefix_identity == prefix_identity,
            "Recovery prefix identity changed; no prefix files were removed"
        );
        return Ok(());
    }
    persist_receipt(
        &Receipt {
            version: 1,
            product_id,
            directory: directory.to_owned(),
            identity,
            prefix_identity,
            setup_process: None,
        },
        false,
    )
}

fn persist_receipt(receipt: &Receipt, replace: bool) -> Result<()> {
    use std::{
        io::Write,
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
    };
    let directory = &receipt.directory;
    let path = receipt_path(directory)?;
    let mut parent = open_directory(directory.parent().context("Missing library")?)?;
    for name in [".ludomere", "staging"] {
        let name_c = std::ffi::CString::new(name)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name_c.as_ptr(), 0o700) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
        }
        parent = open_child(&parent, std::ffi::OsStr::new(name), true, false)?;
    }
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()));
    let mut temporary = tempfile::NamedTempFile::new_in(&anchored)?;
    temporary.write_all(&serde_json::to_vec(receipt)?)?;
    temporary.as_file().sync_all()?;
    let name = path.file_name().context("Missing recovery filename")?;
    ensure!(!name.as_bytes().is_empty(), "Missing recovery filename");
    if replace {
        temporary.persist(anchored.join(name))?;
    } else {
        temporary.persist_noclobber(anchored.join(name))?;
    }
    parent.sync_all()?;
    Ok(())
}

pub(super) fn run_prefix_uninstaller(
    directory: &Path,
    product_id: i64,
    cancelled: &AtomicBool,
    log: &Path,
    spawn: impl FnOnce() -> Result<crate::compatibility::CompatibilityProcess>,
) -> Result<()> {
    let mut record =
        receipt(directory, product_id)?.context("Uninstall recovery record is missing")?;
    if let Some(guard) = &record.setup_process {
        super::dependency_setup::ensure_process_quiescent(guard)?;
    }
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "Uninstallation cancelled before starting"
    );
    record.setup_process = Some(super::dependency_setup::SetupProcessGuard {
        boot: super::dependency_setup::boot_identity()?,
        group: None,
    });
    persist_receipt(&record, true)?;
    let mut process = match spawn() {
        Ok(process) => process,
        Err(error) => {
            record.setup_process = None;
            persist_receipt(&record, true)?;
            return Err(error);
        }
    };
    record.setup_process.as_mut().unwrap().group = Some(process.group_id());
    if let Err(error) = persist_receipt(&record, true) {
        process.stop().context(
            "Uninstaller identity could not be saved or drained; reboot before recovery",
        )?;
        record.setup_process = None;
        persist_receipt(&record, true)?;
        return Err(error);
    }
    let result = super::dependency_setup::wait_process(
        &mut process,
        &|| cancelled.load(Ordering::Acquire),
        "Windows game uninstaller",
        log,
    );
    if !matches!(process.group_running(), Ok(false)) {
        process
            .stop()
            .context("The uninstaller could not be drained; recovery remains blocked")?;
    }
    record.setup_process = None;
    persist_receipt(&record, true)?;
    result
}

fn managed_prefix_path(directory: &Path) -> Result<PathBuf> {
    let slug = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid game folder")?;
    crate::compatibility::validate_slug(slug)?;
    ensure!(slug != ".ludomere", "Invalid game folder");
    Ok(crate::compatibility::prefix_path(
        directory.parent().context("Missing game library")?,
        slug,
    ))
}

fn read_config() -> Result<crate::config::Config> {
    use std::{io::Read, os::unix::fs::MetadataExt};
    let path = crate::config::Config::path();
    let opened = (|| -> std::io::Result<std::fs::File> {
        let parent = open_directory(path.parent().unwrap())?;
        open_child(&parent, path.file_name().unwrap(), false, false)
    })();
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(crate::config::Config::default());
        }
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.nlink() == 1,
        "Unsafe configuration file"
    );
    let mut text = String::new();
    file.by_ref()
        .take(4 * 1024 * 1024 + 1)
        .read_to_string(&mut text)?;
    ensure!(
        text.len() <= 4 * 1024 * 1024,
        "Configuration exceeds its safety limit"
    );
    Ok(toml::from_str(&text)?)
}

pub(super) fn validate_prefix_locations(prefix: &Path) -> Result<()> {
    let config = read_config()?;
    let preferences = crate::compatibility::proton_preferences()?;
    let store = crate::state::StateStore::open()?;
    let mut protected = vec![
        crate::identity::config_root(),
        crate::identity::data_root(),
        crate::identity::cache_root(),
        config.download_directory.clone(),
    ];
    protected.extend(
        crate::config::LibraryKind::ALL
            .into_iter()
            .flat_map(|kind| config.libraries(kind))
            .map(|library| library.path.clone()),
    );
    protected.extend(preferences.default);
    protected.extend(preferences.overrides.into_values());
    protected.extend(
        store
            .managed_files()?
            .into_iter()
            .filter(|file| file.present)
            .map(|file| file.path),
    );
    protected.extend(
        store
            .download_jobs()?
            .into_iter()
            .map(|job| job.destination),
    );
    ensure!(
        !protected.iter().any(|path| path.starts_with(prefix)
            || std::fs::canonicalize(path).is_ok_and(|path| path.starts_with(prefix))),
        "The managed prefix contains a protected profile, library, Proton or download location. Move that location before uninstalling; no prefix files were removed"
    );
    Ok(())
}

fn prefix_identity(
    directory: &Path,
    product_id: i64,
    windows_proof: bool,
    native: bool,
) -> Result<Option<(u64, u64)>> {
    use std::os::unix::fs::MetadataExt;
    if native {
        return Ok(None);
    }
    let prefix = managed_prefix_path(directory)?;
    let opened = match open_directory(&prefix) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).context("Cannot safely inspect this game's managed prefix");
        }
    };
    let metadata = opened.metadata()?;
    validate_prefix_locations(&prefix)?;
    let identity = (metadata.dev(), metadata.ino());
    let prior = receipt(directory, product_id)?;
    if let Some(prior) = &prior {
        ensure!(
            prior.prefix_identity == Some(identity),
            "The managed prefix changed or was not part of the previous removal; no prefix files were removed"
        );
    }
    let owner = read_json(&prefix.join(".ludomere-managed.json"))?;
    if let Some(owner) = &owner {
        ensure!(
            owner
                .get("schema_version")
                .and_then(serde_json::Value::as_u64)
                == Some(1)
                && owner
                    .get("managed_by_ludomere")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && owner.get("slug").and_then(serde_json::Value::as_str)
                    == directory.file_name().and_then(|name| name.to_str()),
            "Managed prefix ownership does not match this game; no prefix files were removed"
        );
    }
    ensure!(
        owner.is_some() || windows_proof || prior.is_some(),
        "Cannot verify this game's managed prefix; no prefix files were removed"
    );
    Ok(Some(identity))
}

fn marker_prefix_identity(
    directory: &Path,
    product_id: i64,
    marker: &super::marker::InstallationMarker,
) -> Result<Option<(u64, u64)>> {
    ensure!(
        marker.product_id == product_id
            && directory.file_name().and_then(|name| name.to_str()) == Some(marker.slug.as_str()),
        "Installation identity changed; reopen Uninstall"
    );
    let Some(compatibility) = &marker.compatibility else {
        return Ok(None);
    };
    ensure!(
        compatibility.managed_by_ludomere && compatibility.prefix_slug == marker.slug,
        "The prefix is not owned exclusively by this game; no prefix files were removed"
    );
    prefix_identity(directory, product_id, true, false)
}

/// Worker-only preview: use the same marker and prefix ownership checks as execution.
pub fn uninstall_prefix(game: &crate::domain::InstalledGame) -> Result<Option<PathBuf>> {
    super::validate_game_library(
        &read_config()?,
        &game.library_id,
        &game.installation_directory,
    )?;
    let marker = super::marker::load(&game.installation_directory)?
        .context("Installation marker is missing; reopen Uninstall for recovery")?;
    marker_prefix_identity(&game.installation_directory, game.product_id, &marker)?
        .map(|_| managed_prefix_path(&game.installation_directory))
        .transpose()
}

pub(super) fn begin_uninstall_prefix(
    directory: &Path,
    product_id: i64,
    marker: &super::marker::InstallationMarker,
) -> Result<Option<(u64, u64)>> {
    use std::os::unix::fs::MetadataExt;
    let prefix = marker_prefix_identity(directory, product_id, marker)?;
    if marker.compatibility.is_some() {
        if let Some(guard) = receipt(directory, product_id)?.and_then(|record| record.setup_process)
        {
            super::dependency_setup::ensure_process_quiescent(&guard)?;
        }
        let metadata = open_directory(directory)?.metadata()?;
        write_receipt(
            directory,
            product_id,
            Some((metadata.dev(), metadata.ino())),
            prefix,
        )?;
    }
    Ok(prefix)
}

pub(super) fn remove_prefix(
    directory: &Path,
    expected: Option<(u64, u64)>,
    cancelled: &AtomicBool,
    session: u64,
) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let Some(expected) = expected else {
        return Ok(false);
    };
    ensure!(
        !cancelled.load(Ordering::Relaxed) && crate::online::account_session() == session,
        "Prefix removal cancelled; review and retry Uninstall"
    );
    let prefix = managed_prefix_path(directory)?;
    validate_prefix_locations(&prefix)?;
    let parent = match open_directory(prefix.parent().unwrap()) {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let opened = match open_child(&parent, prefix.file_name().unwrap(), true, true) {
        Ok(opened) => opened,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let metadata = opened.metadata()?;
    ensure!(
        expected == (metadata.dev(), metadata.ino()),
        "The managed prefix was replaced; no prefix files were removed"
    );
    clear_directory(&opened, &prefix, &[], cancelled, session)?;
    unlink(&parent, prefix.file_name().unwrap(), true)?;
    Ok(true)
}

pub fn prepare_uninstall(
    config: &crate::config::Config,
    product_id: i64,
    slug: &str,
) -> Result<UninstallPreparation> {
    prepare_uninstall_inner(config, product_id, slug, None)
}

/// Preview an exact, explicitly selected directory for deletion. The caller must
/// show its full path and warn that unrecognized files and contained saves are
/// included before calling reset_game. Prefixes are deliberately retained.
pub fn prepare_game_directory_reset(
    config: &crate::config::Config,
    product_id: i64,
    slug: &str,
    library_id: &str,
) -> Result<GameResetPlan> {
    let game = crate::state::StateStore::open()?
        .cached_product_game(product_id)?
        .context("Load this game's library information before preparing recovery")?;
    ensure!(
        product_id > 0 && game.slug == slug,
        "The selected game identity changed; reopen recovery"
    );
    ensure!(
        config
            .game_libraries
            .iter()
            .any(|library| library.id == library_id),
        "The selected Game Files library is no longer configured"
    );
    match prepare_uninstall_inner(config, product_id, slug, Some(library_id))? {
        UninstallPreparation::Recovery(plan) => Ok(plan),
        UninstallPreparation::Normal(_) => unreachable!("explicit reset always needs confirmation"),
    }
}

fn prepare_uninstall_inner(
    config: &crate::config::Config,
    product_id: i64,
    slug: &str,
    exact_library: Option<&str>,
) -> Result<UninstallPreparation> {
    use std::os::unix::fs::MetadataExt;
    crate::compatibility::validate_slug(slug)?;
    let store = crate::state::StateStore::open()?;
    let mut ids = vec![product_id];
    if let Some(game) = store.cached_product_game(product_id)? {
        ensure!(
            game.slug == slug,
            "The game identity changed; reopen Uninstall"
        );
        ids.extend(
            game.dlcs
                .iter()
                .filter(|dlc| dlc.owned)
                .map(|dlc| dlc.product_id),
        );
    }
    let jobs = store.download_jobs()?;
    let managed = store.managed_files()?;
    let active = super::manager::recovery_busy(&ids)
        || jobs.iter().any(|job| {
            ids.contains(&job.product_id) && job.state != crate::state::DownloadState::Complete
        });
    let mut directories = Vec::new();
    let mut identities = Vec::new();
    let mut prefixes = Vec::new();
    let mut prefix_identities = Vec::new();
    let mut prefix_checks = Vec::new();
    let mut installed = None;
    for library in &config.game_libraries {
        if exact_library.is_some_and(|id| id != library.id) {
            continue;
        }
        let directory = library.path.join(slug);
        // An unrelated unavailable library must not prevent managing this game's valid copy.
        if directory.try_exists()?
            || crate::compatibility::prefix_path(&library.path, slug).try_exists()?
            || super::operation_journal::path(&library.path, slug)?.try_exists()?
        {
            super::prefix_recovery::ensure_quiescent(&directory)?;
            crate::storage::validate_game_directory_location(config, &directory)?;
        } else {
            continue;
        }
        ensure!(
            slug != ".ludomere",
            "The library control directory cannot be reset as a game"
        );
        ensure!(
            ![
                crate::identity::config_root(),
                crate::identity::data_root(),
                crate::identity::cache_root()
            ]
            .iter()
            .any(|protected| protected.starts_with(&directory)),
            "This game folder contains Ludomere profile data; move the game before recovery"
        );
        ensure!(
            !crate::config::LibraryKind::ALL
                .into_iter()
                .flat_map(|kind| config.libraries(kind))
                .any(|other| other.path.starts_with(&directory)),
            "A configured library is inside this game's directory; move that library before recovery"
        );
        let journal = super::operation_journal::path(&library.path, slug)?;
        let operation = read_json_bounded(&journal, 64 * 1024 * 1024)
            .map_err(|error| {
                if error.downcast_ref::<serde_json::Error>().is_some() {
                    error.context(DamagedOperationRecord)
                } else {
                    error
                }
            })?
            .map(serde_json::from_value::<super::operation_journal::OperationJournal>)
            .transpose()
            .context(DamagedOperationRecord)?;
        let operation_matches = match &operation {
            Some(super::operation_journal::OperationJournal::Offline { record, .. }) => {
                if record.product_id != product_id {
                    false
                } else {
                    let plan: serde_json::Value =
                        serde_json::from_str(&record.plan_json).context(DamagedOperationRecord)?;
                    let target = plan
                        .pointer("/game/installation_directory")
                        .or_else(|| plan.get("installation_directory"));
                    ensure!(
                        target.and_then(serde_json::Value::as_str).map(Path::new)
                            == Some(directory.as_path()),
                        "Operation destination does not match this game folder"
                    );
                    true
                }
            }
            Some(super::operation_journal::OperationJournal::Depot { record, .. }) => {
                if record.product_id != product_id {
                    false
                } else {
                    let plan: serde_json::Value =
                        serde_json::from_str(&record.plan_json).context(DamagedOperationRecord)?;
                    ensure!(
                        plan.get("destination")
                            .and_then(serde_json::Value::as_str)
                            .map(Path::new)
                            == Some(directory.as_path()),
                        "Operation destination does not match this game folder"
                    );
                    true
                }
            }
            None => false,
        };
        ensure!(
            operation.is_none() || operation_matches,
            "Another game's operation owns this folder"
        );
        let metadata = match std::fs::symlink_metadata(&directory) {
            Ok(value) => Some(value),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(metadata) = &metadata {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Game directory is not a regular directory"
            );
        }
        let marker = optional_metadata(&super::marker::marker_path(&directory))?;
        if let Some(marker) = &marker {
            let marker_id = marker
                .get("product_id")
                .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()));
            ensure!(
                marker_id == Some(product_id) || (exact_library.is_some() && marker_id.is_none()),
                "Installation marker belongs to another game"
            );
        }
        if exact_library.is_some() && metadata.is_some() {
            ensure_no_foreign_game(&directory, product_id, &ids)?;
        }
        let info_matches = if metadata.is_some() && marker.is_none() && !operation_matches {
            optional_metadata(&directory.join(format!("goggame-{product_id}.info")))?.is_some_and(
                |info| {
                    info.get("gameId").and_then(serde_json::Value::as_str)
                        == Some(product_id.to_string().as_str())
                },
            )
        } else {
            false
        };
        let download_matches = jobs.iter().any(|job| {
            ids.contains(&job.product_id)
                && valid_download_destination(&job.destination, &directory, &job.artifacts)
                && !job.artifacts.is_empty()
        }) || managed.iter().any(|file| {
            ids.contains(&file.product_id)
                && file.matched
                && file.present
                && file.path.parent().is_some_and(|parent| {
                    let mut relative = PathBuf::from(file.kind.as_str());
                    if let Some(os) = &file.operating_system {
                        relative.push(os);
                    }
                    if let Some(language) = &file.language {
                        relative.push(language);
                    }
                    file.product_id == product_id && parent == directory.join(relative)
                })
        });
        let prior = receipt(&directory, product_id)?;
        if let (Some(prior), Some(metadata)) = (&prior, &metadata) {
            ensure!(
                prior.identity == Some((metadata.dev(), metadata.ino())),
                "The game directory was replaced after recovery; no files were removed"
            );
        }
        if metadata.is_none() && !operation_matches && prior.is_none() {
            continue;
        }
        ensure!(
            marker.is_some()
                || operation_matches
                || info_matches
                || download_matches
                || prior.is_some()
                || (exact_library.is_some() && metadata.is_some()),
            "Cannot verify ownership of {}; no files were removed",
            directory.display()
        );
        let operation_plan = operation
            .as_ref()
            .map(|operation| match operation {
                super::operation_journal::OperationJournal::Offline { record, .. } => {
                    serde_json::from_str::<serde_json::Value>(&record.plan_json)
                }
                super::operation_journal::OperationJournal::Depot { record, .. } => {
                    serde_json::from_str::<serde_json::Value>(&record.plan_json)
                }
            })
            .transpose()?;
        let compatibility = marker
            .as_ref()
            .and_then(|marker| marker.get("compatibility"))
            .filter(|value| !value.is_null())
            .or_else(|| {
                operation_plan
                    .as_ref()
                    .and_then(|plan| {
                        plan.pointer("/target_marker/compatibility")
                            .or_else(|| plan.pointer("/game/compatibility"))
                    })
                    .filter(|value| !value.is_null())
            });
        if let Some(compatibility) = compatibility.filter(|_| exact_library.is_none()) {
            ensure!(
                compatibility
                    .get("prefix_slug")
                    .and_then(serde_json::Value::as_str)
                    == Some(slug)
                    && compatibility
                        .get("managed_by_ludomere")
                        .and_then(serde_json::Value::as_bool)
                        != Some(false),
                "The prefix is not owned exclusively by this game; no prefix files were removed"
            );
        }
        let platform = operation_plan
            .as_ref()
            .and_then(|plan| {
                plan.pointer("/target_marker/base/operating_system")
                    .or_else(|| plan.pointer("/game/installer_operating_system"))
            })
            .or_else(|| {
                marker
                    .as_ref()
                    .and_then(|marker| marker.pointer("/base/operating_system"))
            });
        let native = exact_library.is_some()
            || (compatibility.is_none()
                && platform
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|os| os != "windows"));
        let prefix = prefix_identity(&directory, product_id, compatibility.is_some(), native)?;
        if prefix.is_some() {
            prefixes.push(managed_prefix_path(&directory)?);
        }
        if exact_library.is_none()
            && !active
            && operation.is_none()
            && prior.is_none()
            && marker.is_some()
            && marker.as_ref().is_some_and(|value| {
                serde_json::from_value::<super::marker::InstallationMarker>(value.clone())
                    .is_ok_and(|marker| {
                        marker.compatibility.is_none()
                            || marker.source == crate::domain::InstallationSource::GalaxyDepot
                            || prefix.is_some()
                    })
            })
        {
            let games = super::reconcile_installed_products(
                &store,
                std::slice::from_ref(library),
                &[(product_id, slug.to_owned())],
                &HashMap::new(),
            )?;
            if let Some(game) = games.into_iter().next() {
                installed = Some(game);
            }
        }
        directories.push(directory);
        identities.push(metadata.map(|metadata| (metadata.dev(), metadata.ino())));
        prefix_identities.push(prefix);
        prefix_checks.push(!native);
    }
    if !active
        && directories.len() == 1
        && let Some(installed) = installed
    {
        return Ok(UninstallPreparation::Normal(installed));
    }
    let downloads = crate::download::managed_downloads(product_id)?;
    ensure!(
        !directories.is_empty() || jobs.iter().any(|job| ids.contains(&job.product_id)),
        "There are no local installation files or downloads to reset"
    );
    Ok(UninstallPreparation::Recovery(GameResetPlan {
        product_id,
        directories,
        prefixes,
        downloaded_files: downloads.count(),
        downloaded_bytes: downloads.bytes(),
        ids,
        config: config.clone(),
        slug: slug.to_owned(),
        identities,
        prefix_identities,
        prefix_checks,
        explicit_directory: exact_library.is_some(),
        session: crate::online::account_session(),
    }))
}

pub(super) fn read_json(path: &Path) -> Result<Option<serde_json::Value>> {
    read_json_bounded(path, 4 * 1024 * 1024)
}

fn read_json_bounded(path: &Path, limit: usize) -> Result<Option<serde_json::Value>> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let opened = (|| -> std::io::Result<std::fs::File> {
        let parent = open_directory(
            path.parent()
                .ok_or_else(|| std::io::Error::other("metadata has no parent"))?,
        )?;
        open_child(
            &parent,
            path.file_name()
                .ok_or_else(|| std::io::Error::other("metadata has no filename"))?,
            false,
            false,
        )
    })();
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.nlink() == 1,
        "Recovery metadata is not a regular file"
    );
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "Recovery metadata exceeds its safety limit"
    );
    Ok(Some(
        serde_json::from_slice(&bytes).context("Reading game recovery metadata")?,
    ))
}

pub(super) fn open_directory(path: &Path) -> std::io::Result<std::fs::File> {
    let mut directory = std::fs::File::open("/")?;
    if !path.is_absolute() {
        return Err(std::io::Error::other(
            "Recovery requires an absolute library path",
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                directory = open_child(&directory, name, true, false)?
            }
            _ => return Err(std::io::Error::other("Unsafe recovery path")),
        }
    }
    Ok(directory)
}

pub(super) fn open_child(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    directory: bool,
    no_mounts: bool,
) -> std::io::Result<std::fs::File> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };
    let name = std::ffi::CString::new(name.as_bytes())?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = if no_mounts {
        #[repr(C)]
        struct OpenHow {
            flags: u64,
            mode: u64,
            resolve: u64,
        }
        let how = OpenHow {
            flags: flags as u64,
            mode: 0,
            resolve: 0x01 | 0x02 | 0x04 | 0x08,
        };
        unsafe {
            libc::syscall(
                libc::SYS_openat2,
                parent.as_raw_fd(),
                name.as_ptr(),
                &how,
                std::mem::size_of::<OpenHow>(),
            ) as i32
        }
    } else {
        unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) }
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }
}

fn unlink(parent: &std::fs::File, name: &std::ffi::OsStr, directory: bool) -> std::io::Result<()> {
    use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
    let name = std::ffi::CString::new(name.as_bytes())?;
    if unsafe {
        libc::unlinkat(
            parent.as_raw_fd(),
            name.as_ptr(),
            if directory { libc::AT_REMOVEDIR } else { 0 },
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn clear_directory(
    directory: &std::fs::File,
    path: &Path,
    protected: &[PathBuf],
    cancelled: &AtomicBool,
    session: u64,
) -> Result<()> {
    use std::os::fd::AsRawFd;
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))? {
        ensure!(
            !cancelled.load(Ordering::Relaxed) && crate::online::account_session() == session,
            "Recovery stopped; some files may already have been removed. Review and retry"
        );
        let entry = entry?;
        let child = path.join(entry.file_name());
        if protected.iter().any(|keep| child.starts_with(keep)) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            let opened = open_child(directory, &entry.file_name(), true, true)?;
            clear_directory(&opened, &child, protected, cancelled, session)?;
            if !protected.iter().any(|keep| keep.starts_with(&child)) {
                unlink(directory, &entry.file_name(), true)?;
            }
        } else {
            unlink(directory, &entry.file_name(), false)?;
        }
    }
    Ok(())
}

pub fn reset_game(
    plan: GameResetPlan,
    remove_downloads: bool,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(&str),
) -> Result<GameResetResult> {
    use std::os::unix::fs::MetadataExt;
    ensure!(
        !cancelled.load(Ordering::Relaxed) && crate::online::account_session() == plan.session,
        "The account changed; reopen Uninstall"
    );
    let _reservation = Reservation::reserve(&plan.ids)?;
    let _activity = crate::profile_reset::begin_activity("game recovery")?;
    let store = crate::state::StateStore::open()?;
    for id in &plan.ids {
        store.clear_download_install_intent(*id)?;
    }
    progress("Stopping this game's downloads and installers…");
    crate::download::quiesce_recovery(&plan.ids, cancelled)?;
    let mut recovery_config = plan.config.clone();
    if plan.explicit_directory {
        recovery_config.game_libraries.retain(|library| {
            plan.directories
                .iter()
                .any(|directory| directory.parent() == Some(library.path.as_path()))
        });
    }
    super::manager::quiesce_recovery(&plan.ids, cancelled, &recovery_config, &plan.slug)?;
    ensure!(
        !cancelled.load(Ordering::Relaxed) && crate::online::account_session() == plan.session,
        "Recovery cancelled before deleting game files"
    );
    let permit = crate::operation_gate::try_acquire()
        .context("Another game's operation is using files. Pause it, then retry this recovery")?;
    ensure!(
        crate::config::Config::path().try_exists()?,
        "Configuration changed; reopen Uninstall"
    );
    let current = read_config()?;
    ensure!(
        current.game_libraries == plan.config.game_libraries
            && current.offline_libraries == plan.config.offline_libraries
            && current.extras_libraries == plan.config.extras_libraries
            && current.download_directory == plan.config.download_directory,
        "Library or download locations changed; reopen Uninstall"
    );
    let mut protected = Vec::new();
    for root in &plan.directories {
        crate::storage::validate_game_directory_location(&current, root)?;
    }
    protected.extend(
        [
            crate::config::LibraryKind::OfflineInstallers,
            crate::config::LibraryKind::Extras,
        ]
        .into_iter()
        .flat_map(|kind| current.libraries(kind))
        .map(|library| library.path.clone()),
    );
    // Download paths stay indexed and in place, even with a shared game/download root.
    for file in store.managed_files()? {
        if file.present {
            protected.push(file.path);
        }
    }
    let jobs = store.download_jobs()?;
    protected.extend(jobs.iter().map(|job| job.destination.clone()));
    for root in &plan.directories {
        ensure!(
            !protected.iter().any(|keep| root.starts_with(keep)),
            "A download destination contains the game directory; separate these locations before recovery"
        );
        ensure!(
            plan.config.download_directory != *root,
            "The game directory is also the configured download root; separate these locations before recovery"
        );
        if plan.config.download_directory != *root
            && plan.config.download_directory.starts_with(root)
        {
            protected.push(plan.config.download_directory.clone());
        }
    }
    let mut result = GameResetResult::default();
    // Revalidate every preview identity before the first destructive operation. A worker may
    // have created a previously absent root while stopping; that needs a new confirmation.
    for (((path, identity), prefix), check_prefix) in plan
        .directories
        .iter()
        .zip(&plan.identities)
        .zip(&plan.prefix_identities)
        .zip(&plan.prefix_checks)
    {
        if let Some(guard) = receipt(path, plan.product_id)?.and_then(|record| record.setup_process)
        {
            super::dependency_setup::ensure_process_quiescent(&guard)?;
        }
        let actual = match open_directory(path) {
            Ok(directory) => {
                let metadata = directory.metadata()?;
                Some((metadata.dev(), metadata.ino()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        ensure!(
            actual.is_none() || actual == *identity,
            "The game directory changed; reopen Uninstall to review it"
        );
        if plan.explicit_directory && actual.is_some() {
            ensure_no_foreign_game(path, plan.product_id, &plan.ids)?;
        }
        let actual_prefix =
            prefix_identity(path, plan.product_id, prefix.is_some(), !check_prefix)?;
        ensure!(
            actual_prefix.is_none() || actual_prefix == *prefix,
            "The managed prefix changed; reopen Uninstall to review it"
        );
        write_receipt(path, plan.product_id, *identity, *prefix)?;
    }
    // Once the receipt is durable, remove runnable operation journals before payload deletion.
    // A crash must leave an explicit recovery offer, never an automatic installation replay.
    for path in &plan.directories {
        let staging = receipt_path(path)?.parent().unwrap().to_owned();
        for name in [
            format!("{}.operation.json", plan.slug),
            format!("{}.json", plan.slug),
            format!("{}.json.support", plan.slug),
        ] {
            remove_control_file(&staging.join(name))?;
        }
    }
    for job in jobs.iter().filter(|job| {
        plan.ids.contains(&job.product_id) && job.state != crate::state::DownloadState::Complete
    }) {
        // Paused records retain any completed-file provenance needed by optional cleanup,
        // but never resume automatically after an interrupted recovery.
        store.save_download_job(&crate::state::DownloadJobUpdate {
            job_id: &job.job_id,
            product_id: job.product_id,
            title: &job.title,
            artifacts: &job.artifacts,
            destination: &job.destination,
            state: crate::state::DownloadState::Paused,
            bytes_downloaded: job.bytes_downloaded,
            total_bytes: job.total_bytes,
            completed_files: &job.completed_files,
            error: Some("Game recovery interrupted this download; remove or explicitly requeue it"),
        })?;
    }
    if plan.prefix_identities.iter().any(Option::is_some) {
        progress("Removing this game's managed prefix and its contained saves/settings…");
    }
    for (path, prefix) in plan.directories.iter().zip(&plan.prefix_identities) {
        match remove_prefix(path, *prefix, cancelled, plan.session) {
            Ok(true) => result.removed_prefixes += 1,
            Ok(false) => {}
            Err(error) => result.failures.push(format!(
                "Managed prefix {} could not be removed: {error}. Review and retry Uninstall",
                managed_prefix_path(path)?.display()
            )),
        }
    }
    if !result.failures.is_empty() {
        result.retained_downloads = crate::download::managed_downloads(plan.product_id)?.count();
        return Ok(result);
    }
    progress("Removing this game's installation files…");
    for (path, identity) in plan.directories.iter().zip(&plan.identities) {
        let removed = (|| -> Result<bool> {
            let parent = open_directory(path.parent().context("Game directory has no library")?)?;
            let directory = match open_child(
                &parent,
                path.file_name().context("Game directory has no name")?,
                true,
                true,
            ) {
                Ok(directory) => directory,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            let metadata = directory.metadata()?;
            ensure!(
                *identity == Some((metadata.dev(), metadata.ino())),
                "The game directory was replaced; reopen Uninstall"
            );
            clear_directory(&directory, path, &protected, cancelled, plan.session)?;
            if !protected.iter().any(|keep| keep.starts_with(path)) {
                unlink(&parent, path.file_name().unwrap(), true)?;
                return Ok(true);
            }
            Ok(false)
        })();
        match removed {
            Ok(true) => result.removed_directories += 1,
            Ok(false) => {}
            Err(error) => result.failures.push(format!("{}: {error}", path.display())),
        }
    }
    if result.failures.is_empty() {
        let game = store.cached_product_game(plan.product_id)?;
        for job in jobs.iter().filter(|job| {
            plan.ids.contains(&job.product_id) && job.state != crate::state::DownloadState::Complete
        }) {
            let child = game
                .as_ref()
                .and_then(|game| {
                    game.dlcs
                        .iter()
                        .find(|dlc| dlc.product_id == job.product_id)
                })
                .map(|dlc| dlc.slug.as_str());
            let staging = crate::download::recovery_staging(job, &plan.slug, child)?;
            match open_directory(&staging) {
                Ok(directory) => {
                    clear_directory(&directory, &staging, &[], cancelled, plan.session)?;
                    unlink(
                        &open_directory(staging.parent().unwrap())?,
                        staging.file_name().unwrap(),
                        true,
                    )?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            if job.completed_files.is_empty() {
                store.delete_download_job(&job.job_id)?;
            }
        }
        super::manager::finish_recovery(&plan.ids);
    }
    drop(permit);
    let downloads = crate::download::managed_downloads(plan.product_id)?;
    result.retained_downloads = downloads.count();
    if remove_downloads && result.failures.is_empty() {
        progress("Removing downloaded installers and extras…");
        match crate::download::delete_managed_downloads(downloads) {
            Ok(cleanup) => {
                result.retained_downloads =
                    result.retained_downloads.saturating_sub(cleanup.deleted);
                result.failures.extend(cleanup.failures);
            }
            Err(error) => result
                .failures
                .push(format!("Game reset, but downloaded files remain: {error}")),
        }
    }
    if result.failures.is_empty() {
        for path in &plan.directories {
            super::prefix_recovery::retire_after_uninstall(path, plan.product_id)?;
            remove_control_file(&receipt_path(path)?)?;
        }
    }
    Ok(result)
}

pub(super) fn remove_control_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let parent = match open_directory(path.parent().context("Missing control parent")?) {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let name = path.file_name().context("Missing control name")?;
    match open_child(&parent, name, false, false) {
        Ok(file) => {
            ensure!(
                file.metadata()?.is_file() && file.metadata()?.nlink() == 1,
                "Unsafe game control file was retained"
            );
            unlink(&parent, name, false)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};

    fn fixture(id: i64) -> (tempfile::TempDir, crate::config::Config, PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path().join("library");
        let directory = library.join(format!("recovery-{id}"));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("goggame-{id}.info")),
            format!(r#"{{"gameId":"{id}"}}"#),
        )
        .unwrap();
        let config = crate::config::Config {
            game_libraries: vec![crate::config::GameLibrary {
                id: "recovery".into(),
                name: "Recovery fixture".into(),
                path: library.clone(),
                default: true,
            }],
            download_directory: library,
            ..Default::default()
        };
        std::fs::create_dir_all(crate::config::Config::path().parent().unwrap()).unwrap();
        config.save().unwrap();
        (temporary, config, directory)
    }

    fn plan(config: &crate::config::Config, id: i64) -> GameResetPlan {
        match prepare_uninstall(config, id, &format!("recovery-{id}")).unwrap() {
            UninstallPreparation::Recovery(plan) => plan,
            UninstallPreparation::Normal(_) => panic!("unexpected normal uninstall"),
        }
    }

    fn known_fixture(id: i64) -> (tempfile::TempDir, crate::config::Config, PathBuf) {
        let fixture = fixture(id);
        crate::state::StateStore::open()
            .unwrap()
            .upsert_normalized_library(&[crate::domain::Game {
                product_id: id,
                slug: format!("recovery-{id}"),
                title: "Synthetic recovery game".into(),
                ..Default::default()
            }])
            .unwrap();
        fixture
    }

    #[test]
    fn explicit_directory_reset_requires_fresh_identity_and_preserves_other_data() {
        let (_temporary, config, directory) = known_fixture(910090);
        let slug = "recovery-910090";
        std::fs::remove_file(directory.join(format!("goggame-{}.info", 910090))).unwrap();
        std::fs::write(directory.join("unrecognized-save"), b"explicitly reviewed").unwrap();
        let marker = super::super::marker::marker_path(&directory);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, b"{}").unwrap();
        let sibling = config.game_libraries[0].path.join("healthy-other-game");
        std::fs::create_dir(&sibling).unwrap();
        std::fs::write(sibling.join("sentinel"), b"keep sibling").unwrap();
        let prefix = crate::compatibility::prefix_path(&config.game_libraries[0].path, slug);
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::write(prefix.join("sentinel"), b"unowned prefix").unwrap();
        assert!(prepare_uninstall(&config, 910090, slug).is_err());
        let preview = prepare_game_directory_reset(&config, 910090, slug, "recovery").unwrap();
        assert!(preview.prefixes.is_empty());
        assert!(
            directory.join("unrecognized-save").exists(),
            "preview never deletes"
        );
        std::fs::write(&marker, br#"{"product_id":999}"#).unwrap();
        assert!(reset_game(preview, false, &AtomicBool::new(false), |_| {}).is_err());
        assert!(directory.join("unrecognized-save").exists());
        std::fs::remove_file(&marker).unwrap();
        let preview = prepare_game_directory_reset(&config, 910090, slug, "recovery").unwrap();
        std::fs::write(directory.join("goggame-999.info"), b"{}").unwrap();
        assert!(reset_game(preview, false, &AtomicBool::new(false), |_| {}).is_err());
        std::fs::remove_file(directory.join("goggame-999.info")).unwrap();
        let preview = prepare_game_directory_reset(&config, 910090, slug, "recovery").unwrap();
        let old = directory.with_extension("old");
        std::fs::rename(&directory, &old).unwrap();
        std::fs::create_dir(&directory).unwrap();
        assert!(reset_game(preview, false, &AtomicBool::new(false), |_| {}).is_err());
        std::fs::remove_dir(&directory).unwrap();
        std::fs::rename(&old, &directory).unwrap();
        let preview = prepare_game_directory_reset(&config, 910090, slug, "recovery").unwrap();
        let result = reset_game(preview, false, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert!(!directory.exists());
        assert_eq!(
            std::fs::read(sibling.join("sentinel")).unwrap(),
            b"keep sibling"
        );
        assert_eq!(
            std::fs::read(prefix.join("sentinel")).unwrap(),
            b"unowned prefix"
        );
    }

    #[test]
    fn exact_reset_ignores_corrupt_same_game_journal_in_another_library() {
        let (temporary, mut config, directory) = known_fixture(910094);
        let other = temporary.path().join("other-library");
        let other_game = other.join("recovery-910094");
        std::fs::create_dir_all(&other_game).unwrap();
        std::fs::write(other_game.join("sentinel"), b"keep other copy").unwrap();
        let journal = super::super::operation_journal::path(&other, "recovery-910094").unwrap();
        std::fs::create_dir_all(journal.parent().unwrap()).unwrap();
        std::fs::write(&journal, b"{corrupt other record").unwrap();
        config.game_libraries.push(crate::config::GameLibrary {
            id: "other".into(),
            name: "Other library".into(),
            path: other,
            default: false,
        });
        config.save().unwrap();
        let preview =
            prepare_game_directory_reset(&config, 910094, "recovery-910094", "recovery").unwrap();
        let result = reset_game(preview, false, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert!(!directory.exists());
        assert_eq!(
            std::fs::read(other_game.join("sentinel")).unwrap(),
            b"keep other copy"
        );
        assert_eq!(std::fs::read(journal).unwrap(), b"{corrupt other record");
    }

    #[test]
    fn damaged_journal_recovery_requires_new_boot_and_restarts_if_evidence_changes() {
        let (_temporary, config, directory) = known_fixture(910091);
        let slug = "recovery-910091";
        let path =
            super::super::operation_journal::path(&config.game_libraries[0].path, slug).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{unfinished").unwrap();
        let marker = super::super::marker::marker_path(&directory);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(marker, b"{}").unwrap();
        let error = prepare_game_directory_reset(&config, 910091, slug, "recovery")
            .err()
            .unwrap();
        assert!(error.downcast_ref::<DamagedOperationRecord>().is_some());
        let prepare = |boot| {
            prepare_directory_recovery_at_boot(&config, 910091, slug, "recovery", boot).unwrap()
        };
        let boot_a = "00000000-0000-0000-0000-000000000001";
        let boot_b = "00000000-0000-0000-0000-000000000002";
        let boot_c = "00000000-0000-0000-0000-000000000003";
        assert_eq!(prepare(boot_a), RecoveryReadiness::RestartRequired);
        assert_eq!(prepare(boot_a), RecoveryReadiness::RestartRequired);
        assert_eq!(std::fs::read(&path).unwrap(), b"{unfinished");
        std::fs::write(&path, b"{changed unfinished").unwrap();
        assert_eq!(
            prepare(boot_b),
            RecoveryReadiness::RestartRequired,
            "changed evidence starts a fresh wait"
        );
        assert_eq!(prepare(boot_b), RecoveryReadiness::RestartRequired);
        assert_eq!(prepare(boot_c), RecoveryReadiness::Ready);
        assert!(!path.exists());
        assert!(directory.exists(), "preparation never deletes the payload");
        let backup = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("operation-quarantine-")
            })
            .unwrap()
            .path();
        assert_eq!(std::fs::read(&backup).unwrap(), b"{changed unfinished");
        assert_eq!(
            prepare(boot_c),
            RecoveryReadiness::Ready,
            "retry after quarantine is idempotent"
        );
        let preview = prepare_game_directory_reset(&config, 910091, slug, "recovery").unwrap();
        let result = reset_game(preview, false, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert!(!directory.exists());
        assert!(backup.exists(), "forensic operation copy remains");
    }

    #[test]
    fn damaged_journal_recovery_rejects_links_other_products_and_future_records() {
        let (temporary, config, directory) = known_fixture(910092);
        let slug = "recovery-910092";
        let path =
            super::super::operation_journal::path(&config.game_libraries[0].path, slug).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let outside = temporary.path().join("outside-record");
        std::fs::write(&outside, b"{broken").unwrap();
        symlink(&outside, &path).unwrap();
        let prepare = || {
            prepare_directory_recovery_at_boot(
                &config,
                910092,
                slug,
                "recovery",
                "00000000-0000-0000-0000-000000000001",
            )
        };
        assert!(prepare().is_err());
        std::fs::remove_file(&path).unwrap();
        for bytes in [
            br#"{"record":{"product_id":999}}"#.as_slice(),
            br#"{"version":2}"#.as_slice(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(prepare().is_err());
        }
        assert!(directory.exists());
        assert_eq!(std::fs::read(outside).unwrap(), b"{broken");
        assert!(
            !path
                .with_file_name(format!("{slug}.reboot-recovery.json"))
                .exists()
        );
    }

    #[test]
    fn valid_large_operation_record_is_not_treated_as_corrupt_during_recovery() {
        let (_temporary, config, directory) = known_fixture(910093);
        let slug = "recovery-910093";
        let path =
            super::super::operation_journal::path(&config.game_libraries[0].path, slug).unwrap();
        super::super::operation_journal::write_offline(&path, &crate::state::InstallationOperationRecord {
            product_id: 910093, operation: "install".into(), state: "failed".into(),
            plan_json: serde_json::json!({"game":{"product_id":910093,"installation_directory":directory,"installer_operating_system":"linux"}}).to_string(),
            message: Some("x".repeat(5 * 1024 * 1024)), percentage: None, queue_position: None, created_at:1, updated_at:1, completed_at:None,
        }).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 4 * 1024 * 1024);
        assert!(prepare_game_directory_reset(&config, 910093, slug, "recovery").is_ok());
    }

    fn windows_marker(id: i64) -> super::super::marker::InstallationMarker {
        serde_json::from_value(serde_json::json!({
            "schema_version":2,"product_id":id,"slug":format!("recovery-{id}"),
            "base":{"operating_system":"windows","installed_at":1},
            "compatibility":{"backend":"umu","managed_by_ludomere":true,
                "prefix_slug":format!("recovery-{id}"),"profile":crate::compatibility::UmuProfile::fallback()}
        })).unwrap()
    }

    #[test]
    fn native_marker_and_native_pending_journal_leave_old_prefix_untouched() {
        for id in [910023, 910024] {
            let (_temporary, config, directory) = fixture(id);
            let prefix = managed_prefix_path(&directory).unwrap();
            std::fs::create_dir_all(&prefix).unwrap();
            crate::compatibility::write_ownership(&prefix, &format!("recovery-{id}")).unwrap();
            std::fs::write(prefix.join("native-sentinel"), b"keep").unwrap();
            if id == 910023 {
                let mut marker = windows_marker(id);
                marker.schema_version = 1;
                marker.compatibility = None;
                marker.base.operating_system = Some("linux".into());
                super::super::marker::write(&marker, &directory).unwrap();
                assert!(
                    marker_prefix_identity(&directory, id, &marker)
                        .unwrap()
                        .is_none()
                );
            } else {
                let journal = super::super::operation_journal::path(
                    &config.game_libraries[0].path,
                    &format!("recovery-{id}"),
                )
                .unwrap();
                super::super::operation_journal::write_offline(&journal, &crate::state::InstallationOperationRecord {
                    product_id: id, operation:"install".into(), state:"failed".into(),
                    plan_json: serde_json::json!({"game":{"product_id":id,"installation_directory":directory,"installer_operating_system":"linux","compatibility":null}}).to_string(),
                    message:None, percentage:None, queue_position:None, created_at:1, updated_at:1, completed_at:None,
                }).unwrap();
            }
            let snapshot = plan(&config, id);
            assert!(snapshot.prefixes.is_empty());
            let result = reset_game(snapshot, false, &AtomicBool::new(false), |_| {}).unwrap();
            assert!(result.failures.is_empty(), "{:?}", result.failures);
            assert_eq!(result.removed_prefixes, 0);
            assert_eq!(
                std::fs::read(prefix.join("native-sentinel")).unwrap(),
                b"keep"
            );
        }
    }

    #[test]
    fn vendor_guard_is_durable_before_spawn_and_blocks_uncertain_prefix_cleanup() {
        let (_temporary, config, directory) = fixture(910025);
        let prefix = managed_prefix_path(&directory).unwrap();
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::write(prefix.join("save"), b"keep until drained").unwrap();
        begin_uninstall_prefix(&directory, 910025, &windows_marker(910025)).unwrap();
        let error = run_prefix_uninstaller(
            &directory,
            910025,
            &AtomicBool::new(false),
            &directory.join("inert.log"),
            || {
                let record = receipt(&directory, 910025).unwrap().unwrap();
                let guard = record.setup_process.as_ref().expect("armed before spawn");
                assert!(guard.group.is_none());
                assert!(super::super::dependency_setup::ensure_process_quiescent(guard).is_err());
                anyhow::bail!("inert spawn failure")
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("inert spawn failure"));
        let mut record = receipt(&directory, 910025).unwrap().unwrap();
        assert!(record.setup_process.is_none());
        let current_boot = super::super::dependency_setup::boot_identity().unwrap();
        for (boot, group) in [
            (current_boot.clone(), None),
            (current_boot.clone(), Some(0)),
            ("invalid".into(), Some(2)),
        ] {
            record.setup_process =
                Some(super::super::dependency_setup::SetupProcessGuard { boot, group });
            persist_receipt(&record, true).unwrap();
            assert!(
                reset_game(
                    plan(&config, 910025),
                    false,
                    &AtomicBool::new(false),
                    |_| {}
                )
                .is_err()
            );
            assert_eq!(
                std::fs::read(prefix.join("save")).unwrap(),
                b"keep until drained"
            );
        }
        let other_boot = if current_boot.starts_with('0') {
            "10000000-0000-0000-0000-000000000000"
        } else {
            "00000000-0000-0000-0000-000000000000"
        };
        record.setup_process = Some(super::super::dependency_setup::SetupProcessGuard {
            boot: other_boot.into(),
            group: None,
        });
        persist_receipt(&record, true).unwrap();
        let result = reset_game(
            plan(&config, 910025),
            false,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert!(result.failures.is_empty());
        assert!(!prefix.exists());
    }

    #[test]
    fn prefix_receipt_survives_vendor_payload_removal_and_partial_cleanup() {
        let (temporary, config, directory) = fixture(910020);
        let prefix = managed_prefix_path(&directory).unwrap();
        std::fs::create_dir_all(prefix.join("drive_c/users/player")).unwrap();
        crate::compatibility::write_ownership(&prefix, "recovery-910020").unwrap();
        let external = temporary.path().join("external-save");
        std::fs::write(&external, b"keep").unwrap();
        symlink(&external, prefix.join("drive_c/users/player/Documents")).unwrap();
        symlink(&config.game_libraries[0].path, prefix.join("l:")).unwrap();
        let other = prefix.with_file_name("other-game");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("keep"), b"other").unwrap();
        let expected = begin_uninstall_prefix(&directory, 910020, &windows_marker(910020)).unwrap();
        assert!(
            receipt(&directory, 910020)
                .unwrap()
                .unwrap()
                .prefix_identity
                .is_some()
        );
        // Simulate a completed vendor removing the payload, followed by cancellation
        // during cleanup. Neither vendor nor helper executables run in this fixture.
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(
            remove_prefix(
                &directory,
                expected,
                &AtomicBool::new(true),
                crate::online::account_session()
            )
            .is_err()
        );
        // A partial deletion may already have removed the ownership file.
        std::fs::remove_file(prefix.join(".ludomere-managed.json")).unwrap();
        let snapshot = plan(&config, 910020);
        assert_eq!(snapshot.prefixes, vec![prefix.clone()]);
        let result = reset_game(snapshot, false, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(result.removed_directories, 0);
        assert_eq!(result.removed_prefixes, 1);
        assert!(!prefix.exists());
        assert!(!receipt_path(&directory).unwrap().exists());
        assert_eq!(std::fs::read(external).unwrap(), b"keep");
        assert_eq!(std::fs::read(other.join("keep")).unwrap(), b"other");
        assert!(config.game_libraries[0].path.is_dir());
    }

    #[test]
    fn prefix_identity_refuses_replacement_foreign_ownership_and_links() {
        let (temporary, config, directory) = fixture(910021);
        let prefix = managed_prefix_path(&directory).unwrap();
        std::fs::create_dir_all(&prefix).unwrap();
        crate::compatibility::write_ownership(&prefix, "other-game").unwrap();
        assert!(begin_uninstall_prefix(&directory, 910021, &windows_marker(910021)).is_err());
        assert!(!receipt_path(&directory).unwrap().exists());
        crate::compatibility::write_ownership(&prefix, "recovery-910021").unwrap();
        let snapshot = plan(&config, 910021);
        let old = prefix.with_file_name("saved-prefix");
        std::fs::rename(&prefix, &old).unwrap();
        std::fs::create_dir(&prefix).unwrap();
        std::fs::write(prefix.join("foreign"), b"keep").unwrap();
        assert!(reset_game(snapshot, false, &AtomicBool::new(false), |_| {}).is_err());
        assert_eq!(std::fs::read(prefix.join("foreign")).unwrap(), b"keep");
        std::fs::remove_dir_all(&prefix).unwrap();
        symlink(temporary.path(), &prefix).unwrap();
        assert!(prefix_identity(&directory, 910021, true, false).is_err());
        assert!(directory.exists());
    }

    #[test]
    fn absent_prefix_and_legacy_receipts_do_not_authorize_new_prefixes() {
        let (_temporary, config, directory) = fixture(910022);
        let snapshot = plan(&config, 910022);
        assert!(snapshot.prefixes.is_empty());
        let metadata = directory.metadata().unwrap();
        write_receipt(
            &directory,
            910022,
            Some((metadata.dev(), metadata.ino())),
            None,
        )
        .unwrap();
        let prefix = managed_prefix_path(&directory).unwrap();
        std::fs::create_dir_all(&prefix).unwrap();
        crate::compatibility::write_ownership(&prefix, "recovery-910022").unwrap();
        assert!(prepare_uninstall(&config, 910022, "recovery-910022").is_err());
        assert!(reset_game(snapshot, false, &AtomicBool::new(false), |_| {}).is_err());
        assert!(prefix.exists());
        std::fs::remove_dir_all(&prefix).unwrap();
        let result = reset_game(
            plan(&config, 910022),
            false,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert!(result.failures.is_empty());
        assert_eq!(result.removed_prefixes, 0);
    }

    #[test]
    fn offline_windows_without_prefix_offers_recovery_instead_of_vendor_execution() {
        let (_temporary, config, directory) = fixture(910026);
        super::super::marker::write(&windows_marker(910026), &directory).unwrap();
        std::fs::write(directory.join("game.exe"), b"inert payload").unwrap();
        std::fs::write(directory.join("unins000.exe"), b"never execute").unwrap();
        let snapshot = plan(&config, 910026);
        assert!(snapshot.prefixes.is_empty());
        let result = reset_game(snapshot, false, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(result.failures.is_empty());
        assert_eq!(result.removed_prefixes, 0);
        assert!(!directory.exists());
    }

    #[test]
    fn prefix_overlap_refuses_protected_downloads_libraries_profiles_and_proton() {
        let (_temporary, mut config, directory) = fixture(910027);
        let prefix = managed_prefix_path(&directory).unwrap();
        std::fs::create_dir_all(&prefix).unwrap();
        crate::compatibility::write_ownership(&prefix, "recovery-910027").unwrap();
        let sentinel = prefix.join("preserve");
        std::fs::write(&sentinel, b"protected").unwrap();
        let original = config.clone();
        config.download_directory = prefix.join("downloads");
        config.save().unwrap();
        assert!(begin_uninstall_prefix(&directory, 910027, &windows_marker(910027)).is_err());
        config = original.clone();
        config.game_libraries.push(crate::config::GameLibrary {
            id: "nested".into(),
            name: "Nested".into(),
            path: prefix.join("library"),
            default: false,
        });
        config.save().unwrap();
        assert!(prepare_uninstall(&config, 910027, "recovery-910027").is_err());
        original.save().unwrap();
        let preference_path = crate::identity::config_root().join("proton.json");
        let saved = std::fs::read(&preference_path).ok();
        std::fs::write(
            &preference_path,
            serde_json::json!({"default":prefix.join("Proton")}).to_string(),
        )
        .unwrap();
        assert!(begin_uninstall_prefix(&directory, 910027, &windows_marker(910027)).is_err());
        match saved {
            Some(bytes) => std::fs::write(&preference_path, bytes).unwrap(),
            None => std::fs::remove_file(&preference_path).unwrap(),
        }
        assert!(validate_prefix_locations(&crate::identity::config_root()).is_err());
        assert!(validate_prefix_locations(&crate::identity::data_root()).is_err());
        assert_eq!(std::fs::read(sentinel).unwrap(), b"protected");
        assert!(!receipt_path(&directory).unwrap().exists());
    }

    #[test]
    fn recovery_deletes_confirmed_residuals_without_following_links_or_erasing_profile() {
        let (temporary, config, directory) = fixture(910001);
        let outside = temporary.path().join("external-save");
        let prefix = config.game_libraries[0]
            .path
            .join(".ludomere/compatibility/recovery-910001");
        std::fs::create_dir_all(&prefix).unwrap();
        crate::compatibility::write_ownership(&prefix, "recovery-910001").unwrap();
        std::fs::write(prefix.join("sentinel"), b"prefix").unwrap();
        std::fs::write(&outside, b"external save").unwrap();
        symlink(&outside, directory.join("linked-save")).unwrap();
        std::fs::write(
            directory.join("untracked-save"),
            b"explicitly authorized residual",
        )
        .unwrap();
        let marker = super::super::marker::marker_path(&directory);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(marker, b"{broken").unwrap();
        let config_before = std::fs::read(crate::config::Config::path()).unwrap();
        assert!(prepare_uninstall(&config, 910001, "recovery-910001").is_ok());
        assert!(directory.join("untracked-save").exists());
        // Correct the known inert metadata before exercising authorized recovery.
        super::super::marker::write(&windows_marker(910001), &directory).unwrap();
        let snapshot = plan(&config, 910001);
        assert!(directory.join("untracked-save").exists()); // Preview is read-only.
        let result = reset_game(snapshot, false, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(result.removed_directories, 1);
        assert!(!directory.exists());
        assert_eq!(std::fs::read(&outside).unwrap(), b"external save");
        assert!(!prefix.exists());
        assert_eq!(result.removed_prefixes, 1);
        assert_eq!(
            std::fs::read(crate::config::Config::path()).unwrap(),
            config_before
        );
        assert!(!receipt_path(&directory).unwrap().exists());
    }

    #[test]
    fn recovery_receipt_preserves_explicit_retry_and_rejects_replaced_roots() {
        let (_temporary, config, directory) = fixture(910002);
        let metadata = directory.metadata().unwrap();
        write_receipt(
            &directory,
            910002,
            Some((metadata.dev(), metadata.ino())),
            None,
        )
        .unwrap();
        std::fs::remove_file(directory.join("goggame-910002.info")).unwrap();
        let snapshot = plan(&config, 910002);
        assert!(reset_game(snapshot.clone(), false, &AtomicBool::new(true), |_| {}).is_err());
        assert!(receipt_path(&directory).unwrap().exists());
        std::fs::rename(&directory, directory.with_extension("old")).unwrap();
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("unrelated"), b"preserve").unwrap();
        assert!(prepare_uninstall(&config, 910002, "recovery-910002").is_err());
        assert!(reset_game(snapshot, false, &AtomicBool::new(false), |_| {}).is_err());
        assert_eq!(
            std::fs::read(directory.join("unrelated")).unwrap(),
            b"preserve"
        );
    }

    #[test]
    fn recovery_metadata_rejects_symlinks_fifo_and_mismatched_identity() {
        let (temporary, _config, directory) = fixture(910003);
        let source = temporary.path().join("source.json");
        std::fs::write(&source, b"{}").unwrap();
        symlink(&source, directory.join("linked.json")).unwrap();
        assert!(read_json(&directory.join("linked.json")).is_err());
        let fifo = directory.join("fifo");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_json(&fifo).is_err());
        write_receipt(&directory, 910003, None, None).unwrap();
        assert!(receipt(&directory, 910004).is_err());
    }

    #[test]
    fn recovery_generation_revokes_popped_and_delayed_work_after_guard_drops() {
        let id = 910004;
        let before = generation(id);
        let reservation = Reservation::reserve(&[id]).unwrap();
        assert!(admit_generation(id, before).is_err());
        drop(reservation);
        assert!(!current(id, before));
        assert!(admit_generation(id, before).is_err());
        assert!(admit_generation(id, generation(id)).is_ok());
    }

    #[test]
    fn protected_download_files_and_cancellation_leave_truthful_partial_state() {
        let (_temporary, _config, directory) = fixture(910005);
        let downloads = directory.join("installer/windows/en");
        std::fs::create_dir_all(&downloads).unwrap();
        let installer = downloads.join("setup.exe");
        std::fs::write(&installer, b"retain installer").unwrap();
        std::fs::write(directory.join("payload"), b"delete payload").unwrap();
        let opened = open_directory(&directory).unwrap();
        let session = crate::online::account_session();
        assert!(
            clear_directory(
                &opened,
                &directory,
                std::slice::from_ref(&installer),
                &AtomicBool::new(true),
                session
            )
            .is_err()
        );
        assert!(directory.join("payload").exists());
        clear_directory(
            &opened,
            &directory,
            std::slice::from_ref(&installer),
            &AtomicBool::new(false),
            session,
        )
        .unwrap();
        assert!(!directory.join("payload").exists());
        assert_eq!(std::fs::read(&installer).unwrap(), b"retain installer");
    }

    #[test]
    fn download_only_typed_root_preserves_downloads_unless_explicitly_selected() {
        for (id, delete) in [(910006, false), (910007, true)] {
            let (temporary, mut config, directory) = fixture(id);
            let archive = temporary.path().join("offline");
            std::fs::create_dir(&archive).unwrap();
            config.offline_libraries.push(crate::config::GameLibrary {
                id: "offline".into(),
                name: "Offline".into(),
                path: archive.clone(),
                default: true,
            });
            let extras = temporary.path().join("extras");
            std::fs::create_dir(&extras).unwrap();
            config.extras_libraries.push(crate::config::GameLibrary {
                id: "extras".into(),
                name: "Extras".into(),
                path: extras.clone(),
                default: true,
            });
            config.save().unwrap();
            let artifacts: Vec<crate::domain::RemoteArtifact> = serde_json::from_value(serde_json::json!([
                {"product_id":id,"kind":"installer","name":"Fixture","operating_system":"windows","language":"en","size_bytes":4,"download_path":"/inert"}
            ])).unwrap();
            let destination = crate::download::destination(
                &archive,
                &format!("recovery-{id}"),
                None,
                &[&artifacts[0]],
            );
            std::fs::create_dir_all(&destination).unwrap();
            let file = destination.join("setup.exe");
            std::fs::write(&file, b"data").unwrap();
            let store = crate::state::StateStore::open().unwrap();
            let job = format!("recovery-job-{id}");
            store
                .save_download_job(&crate::state::DownloadJobUpdate {
                    job_id: &job,
                    product_id: id,
                    title: "Fixture",
                    artifacts: &artifacts,
                    destination: &destination,
                    state: crate::state::DownloadState::Complete,
                    bytes_downloaded: 4,
                    total_bytes: Some(4),
                    completed_files: std::slice::from_ref(&file),
                    error: None,
                })
                .unwrap();
            store
                .record_completed_artifacts(
                    &job,
                    &format!("recovery-{id}"),
                    &artifacts,
                    std::slice::from_ref(&file),
                )
                .unwrap();
            let mut extra = artifacts[0].clone();
            extra.kind = crate::domain::ArtifactKind::Extra;
            extra.download_path = "/inert-extra".into();
            let extra_directory =
                crate::download::destination(&extras, &format!("recovery-{id}"), None, &[&extra]);
            std::fs::create_dir_all(&extra_directory).unwrap();
            let extra_file = extra_directory.join("soundtrack.zip");
            std::fs::write(&extra_file, b"data").unwrap();
            store
                .record_completed_artifacts(
                    &format!("{job}-extra"),
                    &format!("recovery-{id}"),
                    &[extra],
                    std::slice::from_ref(&extra_file),
                )
                .unwrap();
            std::fs::write(directory.join("partial-payload"), b"remove").unwrap();
            let result =
                reset_game(plan(&config, id), delete, &AtomicBool::new(false), |_| {}).unwrap();
            assert!(result.failures.is_empty(), "{:?}", result.failures);
            assert!(!directory.join("partial-payload").exists());
            assert_eq!(file.exists(), !delete);
            assert_eq!(extra_file.exists(), !delete);
            assert_eq!(result.retained_downloads, 2 * usize::from(!delete));
            assert_eq!(
                store
                    .managed_files_for_products(&[id])
                    .unwrap()
                    .iter()
                    .any(|file| file.present),
                !delete
            );
        }
    }

    #[test]
    fn corrupt_journal_refuses_recovery_then_restored_fixture_supports_interrupted_retry() {
        let (_temporary, config, directory) = fixture(910008);
        let preview = plan(&config, 910008);
        let journal = super::super::operation_journal::path(
            &config.game_libraries[0].path,
            "recovery-910008",
        )
        .unwrap();
        std::fs::create_dir_all(journal.parent().unwrap()).unwrap();
        std::fs::write(&journal, b"{broken").unwrap();
        assert!(prepare_uninstall(&config, 910008, "recovery-910008").is_err());
        let error = reset_game(preview, false, &AtomicBool::new(false), |_| {})
            .err()
            .unwrap();
        assert!(error.to_string().contains("No files were removed"));
        assert!(directory.join("goggame-910008.info").exists());
        assert_eq!(std::fs::read(&journal).unwrap(), b"{broken");
        // Restore this fixture's known no-operation state; production never deletes
        // an unreadable guard to bypass the refusal. Unrelated corruption is isolated.
        std::fs::remove_file(&journal).unwrap();
        let unrelated = journal.with_file_name("unrelated.operation.json");
        std::fs::write(&unrelated, b"{broken").unwrap();
        let cancel = AtomicBool::new(false);
        let result = reset_game(plan(&config, 910008), false, &cancel, |message| {
            if message.starts_with("Removing this") {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert!(!result.failures.is_empty());
        assert!(!journal.exists());
        assert!(receipt_path(&directory).unwrap().exists());
        std::fs::remove_file(directory.join("goggame-910008.info")).unwrap();
        let result = reset_game(
            plan(&config, 910008),
            false,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert!(result.failures.is_empty());
        assert!(!directory.exists());
        assert!(!receipt_path(&directory).unwrap().exists());
        assert_eq!(std::fs::read(unrelated).unwrap(), b"{broken");
    }
}
