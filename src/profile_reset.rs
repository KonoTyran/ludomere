//! Full profile reset runs only after replacing the process, never beside profile writers.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub const CLEANUP_ARGUMENT: &str = "--complete-profile-reset";
const MANIFEST: &str = ".profile-reset.json";
const FAILURE: &str = ".profile-reset-error.txt";
const LOCK: &str = ".profile.lock";
const LIMIT: u64 = 256 * 1024;
static PROFILE_LOCK: OnceLock<File> = OnceLock::new();
static PENDING: Mutex<Option<String>> = Mutex::new(None);
static PREPARATION: (Mutex<bool>, std::sync::Condvar) =
    (Mutex::new(false), std::sync::Condvar::new());
static ACTIVITIES: Mutex<ActivityState> = Mutex::new(ActivityState {
    frozen: false,
    counts: BTreeMap::new(),
});

#[derive(Default)]
struct ActivityState {
    frozen: bool,
    counts: BTreeMap<&'static str, usize>,
}

pub(crate) struct ActivityGuard(&'static str);

impl ActivityGuard {
    pub(crate) fn running_game(&mut self) {
        let mut state = ACTIVITIES.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(count) = state.counts.get_mut(self.0) {
            *count -= 1;
            if *count == 0 {
                state.counts.remove(self.0);
            }
        }
        self.0 = "running game";
        *state.counts.entry(self.0).or_default() += 1;
    }
}

pub(crate) fn begin_activity(kind: &'static str) -> Result<ActivityGuard> {
    let mut state = ACTIVITIES.lock().unwrap_or_else(|error| error.into_inner());
    ensure!(
        !state.frozen,
        "Profile reset is closing Ludomere; this operation cannot start"
    );
    *state.counts.entry(kind).or_default() += 1;
    Ok(ActivityGuard(kind))
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut state = ACTIVITIES.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(count) = state.counts.get_mut(self.0) {
            *count -= 1;
            if *count == 0 {
                state.counts.remove(self.0);
            }
        }
    }
}

fn freeze(state: &mut ActivityState) -> Result<()> {
    ensure!(!state.frozen, "A profile reset is already pending");
    ensure!(
        state.counts.is_empty(),
        "Finish or stop active operations before resetting the profile: {}",
        state.counts.keys().copied().collect::<Vec<_>>().join(", ")
    );
    state.frozen = true;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u8,
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    protected: Vec<PathBuf>,
    #[serde(default)]
    journals: Vec<Journal>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    path: PathBuf,
    hash: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    plan: Plan,
    plan_digest: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn current_roots() -> [PathBuf; 3] {
    [
        crate::identity::config_root(),
        crate::identity::data_root(),
        crate::identity::cache_root(),
    ]
}

// Linux OFD lock conversion retains the old shared lock when an exclusive request fails.
// flock(2) upgrades do not, which could expose a running process during reset preflight.
fn lock_profile(file: &File, exclusive: bool, wait: bool) -> std::io::Result<()> {
    let mut lock = libc::flock {
        l_type: if exclusive {
            libc::F_WRLCK
        } else {
            libc::F_RDLCK
        } as _,
        l_whence: libc::SEEK_SET as _,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    if unsafe {
        libc::fcntl(
            file.as_raw_fd(),
            if wait {
                libc::F_OFD_SETLKW
            } else {
                libc::F_OFD_SETLK
            },
            &mut lock,
        )
    } == -1
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Called before any GTK/profile worker starts. Shared locks preserve normal secondary activation.
pub fn initialize() -> Result<()> {
    if PROFILE_LOCK.get().is_none() {
        let root = crate::identity::config_root();
        fs::create_dir_all(&root)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join(LOCK))?;
        validate_file(&file)?;
        lock_profile(&file, false, true).context("waiting for profile reset to finish")?;
        PROFILE_LOCK
            .set(file)
            .map_err(|_| anyhow::anyhow!("Profile lock already initialized"))?;
    }
    match fs::symlink_metadata(crate::identity::config_root().join(MANIFEST)) {
        Ok(_) => bail!("{}", pending_message()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub fn pending_message() -> String {
    let message = (|| -> Result<String> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(crate::identity::config_root().join(FAILURE))?;
        validate_file(&file)?;
        let mut text = String::new();
        file.take(8192).read_to_string(&mut text)?;
        Ok(text)
    })()
    .ok();
    format!(
        "The full profile reset is incomplete. No library workers have started. {} Retry the reset or close Ludomere. Installed games and installer payloads are preserved.",
        message.unwrap_or_default()
    )
}

pub struct PendingReset {
    request: Request,
    armed: bool,
}
pub struct ResetReservation {
    armed: bool,
}

/// Reserve synchronously before spawning preflight, so application shutdown waits for its result.
pub fn reserve() -> Result<ResetReservation> {
    freeze(&mut ACTIVITIES.lock().unwrap_or_else(|error| error.into_inner()))?;
    *PREPARATION
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = true;
    Ok(ResetReservation { armed: true })
}

/// The account is already revoked. Block new work, then drain on the worker thread.
pub fn reserve_for_sign_out() -> Result<ResetReservation> {
    let mut state = ACTIVITIES.lock().unwrap_or_else(|error| error.into_inner());
    ensure!(!state.frozen, "Profile cleanup is already pending");
    state.frozen = true;
    *PREPARATION
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = true;
    Ok(ResetReservation { armed: true })
}

pub fn keeps_running_games() -> bool {
    *PREPARATION
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        || PENDING
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
}

pub(crate) fn stopping_operations() -> bool {
    ACTIVITIES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .frozen
}

fn release_preparation(unfreeze: bool) {
    if unfreeze {
        ACTIVITIES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .frozen = false;
        if let Some(lock) = PROFILE_LOCK.get() {
            let _ = lock_profile(lock, false, true);
        }
    }
    *PREPARATION
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = false;
    PREPARATION.1.notify_all();
}

impl Drop for ResetReservation {
    fn drop(&mut self) {
        if self.armed {
            release_preparation(true);
        }
    }
}

/// Worker-only preflight. Failure does not sign out, erase data, or terminate an operation.
impl ResetReservation {
    pub fn prepare(mut self, config: &crate::config::Config) -> Result<PendingReset> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        loop {
            let state = ACTIVITIES.lock().unwrap_or_else(|error| error.into_inner());
            if state.counts.keys().all(|kind| *kind == "running game") {
                break;
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "Signed out. Profile reset is waiting for {} to stop; retry cleanup. Files were kept.",
                state
                    .counts
                    .keys()
                    .filter(|kind| **kind != "running game")
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            drop(state);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let lock = PROFILE_LOCK
            .get()
            .context("Profile lifecycle lock is unavailable; restart Ludomere")?;
        let result = (|| {
            lock_profile(lock, true, false)
                .context("Another Ludomere process is using this profile; close it and retry")?;
            let [config_root, data, cache] = current_roots();
            let mut protected = crate::config::LibraryKind::ALL
                .into_iter()
                .flat_map(|kind| config.libraries(kind))
                .map(|library| library.path.clone())
                .collect::<Vec<_>>();
            protected.push(config.download_directory.clone());
            let proton = crate::compatibility::proton_preferences()?;
            protected.extend(proton.default);
            protected.extend(proton.overrides.into_values());
            protected.extend([
                data.join("games"),
                data.join("downloads"),
                data.join("proton"),
                data.join("umu"),
                data.join("cloud-save-backups"),
                data.join("cloud-save-deletion-recovery"),
                data.join("comet"),
            ]);
            let store = crate::state::StateStore::open()?;
            let mut journals = Vec::new();
            for library in &config.game_libraries {
                let staging = library.path.join(".ludomere/staging");
                validate_path(&staging)?;
                if staging.is_dir() {
                    for entry in fs::read_dir(staging)? {
                        let path = entry?.path();
                        if path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.ends_with(".operation.json"))
                        {
                            journals.push(Journal {
                                hash: journal_hash(&path)?,
                                path,
                            });
                        }
                    }
                }
            }
            protected.extend(store.managed_files()?.into_iter().map(|file| file.path));
            protected.extend(
                store
                    .download_jobs()?
                    .into_iter()
                    .map(|job| job.destination),
            );
            drop(store);
            let mut expanded = Vec::new();
            for path in protected {
                ensure!(
                    path.is_absolute(),
                    "Payload path must be absolute before a profile reset: {}",
                    path.display()
                );
                expanded.push(path.clone());
                if let Ok(canonical) = fs::canonicalize(&path) {
                    expanded.push(canonical);
                }
            }
            expanded.sort();
            expanded.dedup();
            let plan = Plan {
                version: 1,
                config: config_root,
                data,
                cache,
                protected: expanded,
                journals,
            };
            validate_plan(&plan, &current_roots())?;
            let plan_digest = digest(&serde_json::to_vec(&plan)?);
            Ok(PendingReset {
                request: Request { plan, plan_digest },
                armed: true,
            })
        })();
        if result.is_ok() {
            self.armed = false;
        }
        result
    }
}

impl PendingReset {
    /// Schedule cleanup, then the caller closes the application. No data is deleted here.
    pub fn commit(mut self) -> Result<()> {
        validate_plan(&self.request.plan, &current_roots())?;
        let bytes = serde_json::to_vec(&self.request)?;
        ensure!(
            bytes.len() as u64 <= LIMIT,
            "Profile reset request is too large"
        );
        let mut temporary = tempfile::NamedTempFile::new_in(&self.request.plan.config)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist_noclobber(self.request.plan.config.join(MANIFEST))?;
        *PENDING.lock().unwrap_or_else(|error| error.into_inner()) = Some(digest(&bytes));
        self.armed = false;
        release_preparation(false);
        Ok(())
    }
}

impl Drop for PendingReset {
    fn drop(&mut self) {
        if self.armed {
            release_preparation(true);
        }
    }
}

/// Used only by the recovery window's explicit Retry button.
pub fn retry_pending() -> Result<()> {
    let lock = PROFILE_LOCK.get().context("Profile lock unavailable")?;
    lock_profile(lock, true, false).context("Another Ludomere process is using the profile")?;
    let (request, hash) = read_request(None)?;
    validate_plan(&request.plan, &current_roots())?;
    *PENDING.lock().unwrap_or_else(|error| error.into_inner()) = Some(hash);
    Ok(())
}

/// Called after GTK shutdown. Exec replaces all detached threads before any deletion.
pub fn finish_application() -> Result<()> {
    let preparing = PREPARATION
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (preparing, _) = PREPARATION
        .1
        .wait_timeout_while(preparing, std::time::Duration::from_secs(5), |preparing| {
            *preparing
        })
        .unwrap_or_else(|error| error.into_inner());
    ensure!(
        !*preparing,
        "Signed out, but profile cleanup is still pending. Profile data and operation records were kept; retry cleanup after reopening Ludomere."
    );
    drop(preparing);
    let Some(hash) = PENDING
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    else {
        return Ok(());
    };
    let fd = PROFILE_LOCK
        .get()
        .context("Profile lock unavailable")?
        .as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    ensure!(
        flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } == 0,
        "Could not preserve the reset lock across process replacement"
    );
    let error = std::process::Command::new("/proc/self/exe")
        .arg(CLEANUP_ARGUMENT)
        .arg(hash)
        .arg(fd.to_string())
        .exec();
    record_failure(&format!(
        "Could not restart Ludomere to finish the profile reset: {error}"
    ));
    Err(error).context("Could not restart Ludomere to finish the profile reset")
}

pub fn complete(arguments: &[String]) -> Result<()> {
    ensure!(
        arguments.len() == 4 && arguments[1] == CLEANUP_ARGUMENT,
        "Invalid profile reset invocation"
    );
    ensure!(
        arguments[2].len() == 64 && arguments[2].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid profile reset digest"
    );
    let fd = arguments[3].parse::<i32>()?;
    ensure!(fd >= 3, "Invalid inherited profile lock");
    ensure!(
        unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0,
        "Inherited profile lock is not open"
    );
    let file = unsafe { File::from_raw_fd(fd) };
    validate_file(&file)?;
    let expected = fs::symlink_metadata(crate::identity::config_root().join(LOCK))?;
    let actual = file.metadata()?;
    ensure!(
        expected.is_file() && expected.dev() == actual.dev() && expected.ino() == actual.ino(),
        "Inherited reset lock does not match this profile"
    );
    lock_profile(&file, true, false).context("Profile is still in use")?;
    PROFILE_LOCK
        .set(file)
        .map_err(|_| anyhow::anyhow!("Reset must run in a fresh process"))?;
    // SQLite and all other old descriptors must be closed, retaining only the verified lock.
    for (first, last) in [(3, fd as u32 - 1), (fd as u32 + 1, u32::MAX)] {
        if first <= last && unsafe { libc::syscall(libc::SYS_close_range, first, last, 0_u32) } != 0
        {
            return Err(std::io::Error::last_os_error())
                .context("Could not close old profile handles");
        }
    }
    let result = (|| {
        let (request, _) = read_request(Some(&arguments[2]))?;
        validate_plan(&request.plan, &current_roots())?;
        if matches!(
            crate::auth::logout_for_reset().context(
                "Could not secure the signed-out state; the profile has not been erased"
            )?,
            crate::auth::ResetCredentialCleanup::Unavailable
        ) {
            tracing::warn!(
                "Factory Reset: external credential entry may remain; automatic login stays disabled"
            );
        }
        remove_profile(&request.plan)?;
        Ok(())
    })();
    if let Err(error) = &result {
        record_failure(&format!("{error:#}"));
    }
    result
}

fn record_failure(message: &str) {
    if validate_path(&crate::identity::config_root()).is_err() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(crate::identity::config_root().join(FAILURE))
        && validate_file(&file).is_ok()
        && file.set_len(0).is_ok()
    {
        let _ = file.write_all(message.as_bytes());
    }
}

fn read_request(expected: Option<&str>) -> Result<(Request, String)> {
    read_request_at(&crate::identity::config_root(), expected)
}

fn read_request_at(root: &Path, expected: Option<&str>) -> Result<(Request, String)> {
    validate_path(root)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(root.join(MANIFEST))?;
    validate_file(&file)?;
    ensure!(
        file.metadata()?.len() <= LIMIT,
        "Reset request exceeds its size limit"
    );
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "Reset request exceeds its size limit"
    );
    let hash = digest(&bytes);
    ensure!(
        expected.is_none_or(|expected| expected == hash),
        "Reset request changed after confirmation"
    );
    let request: Request = serde_json::from_slice(&bytes).context("Reset request is damaged")?;
    ensure!(
        request.plan_digest == digest(&serde_json::to_vec(&request.plan)?),
        "Reset request failed its integrity check"
    );
    Ok((request, hash))
}

fn validate_file(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.nlink() == 1
            && metadata.permissions().mode() & 0o077 == 0,
        "Reset control file must be private, owned by this user, and not linked"
    );
    Ok(())
}

fn targets(plan: &Plan) -> Result<Vec<PathBuf>> {
    let mut paths = ["config.toml", "proton.json"]
        .into_iter()
        .map(|name| plan.config.join(name))
        .collect::<Vec<_>>();
    paths.extend(
        [
            "library.sqlite3",
            "library.sqlite3-wal",
            "library.sqlite3-shm",
            "library.sqlite3-journal",
            "account",
            "installation-logs",
            "runtime-logs",
            "install-targets",
        ]
        .into_iter()
        .map(|name| plan.data.join(name)),
    );
    paths.extend(
        [
            "account",
            "media",
            "products",
            "screenshots",
            "compatibility-libraries",
            "umu-database.json",
        ]
        .into_iter()
        .map(|name| plan.cache.join(name)),
    );
    if plan.cache.is_dir() {
        for entry in fs::read_dir(&plan.cache)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.strip_prefix("comet-session-").is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte == b'-')
            }) {
                paths.push(entry.path());
            }
        }
    }
    Ok(paths)
}

fn validate_plan(plan: &Plan, roots: &[PathBuf; 3]) -> Result<()> {
    ensure!(
        plan.version == 1
            && [&plan.config, &plan.data, &plan.cache] == [&roots[0], &roots[1], &roots[2]],
        "Reset request does not match this profile"
    );
    for root in roots {
        validate_path(root)?;
    }
    for protected in &plan.protected {
        ensure!(
            protected.is_absolute()
                && !protected
                    .components()
                    .any(|part| matches!(part, Component::ParentDir | Component::CurDir)),
            "Invalid protected payload path"
        );
    }
    for journal in &plan.journals {
        let library = journal
            .path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .context("Invalid reset operation journal")?;
        let name = journal
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .context("Invalid reset journal filename")?;
        ensure!(
            plan.protected.contains(&library.to_path_buf())
                && journal.path.parent() == Some(library.join(".ludomere/staging").as_path())
                && name.ends_with(".operation.json")
                && journal.hash.len() == 64,
            "Reset operation journal is outside configured storage"
        );
        validate_path(&journal.path)?;
        if journal.path.try_exists()? {
            ensure!(
                journal_hash(&journal.path)? == journal.hash,
                "Saved operation changed; review profile reset again"
            );
        }
    }
    for target in targets(plan)? {
        ensure!(
            !roots.iter().any(|root| root.starts_with(&target)),
            "A profile root overlaps reset-owned data; move the nested XDG directory before resetting"
        );
        ensure!(
            !plan
                .protected
                .iter()
                .any(|payload| target.starts_with(payload) || payload.starts_with(&target)),
            "Cannot reset while payload storage overlaps profile data: {}. Move or reconfigure that library first.",
            target.display()
        );
        // Never follow a leaf symlink either: refuse before making any destructive change.
        validate_path(&target)?;
    }
    Ok(())
}

fn validate_path(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Reset paths must be absolute");
    let mut current = PathBuf::from("/");
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                current.push(name);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) => ensure!(
                        !metadata.file_type().is_symlink(),
                        "Profile reset refuses a symlink: {}",
                        current.display()
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            _ => bail!("Invalid reset path"),
        }
    }
    Ok(())
}

fn remove_profile(plan: &Plan) -> Result<()> {
    for journal in &plan.journals {
        if journal.path.try_exists()? {
            ensure!(
                journal_hash(&journal.path)? == journal.hash,
                "Saved operation changed; cleanup remains incomplete"
            );
            remove_owned(&journal.path)?;
        }
    }
    for target in targets(plan)? {
        remove_owned(&target)?;
    }
    for name in [FAILURE, MANIFEST] {
        remove_owned(&plan.config.join(name))?;
    }
    Ok(())
}

fn journal_hash(path: &Path) -> Result<String> {
    validate_path(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.nlink() == 1 && metadata.len() <= 16 * 1024 * 1024,
        "Saved operation is not a bounded regular file; profile reset remains incomplete"
    );
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "Saved operation exceeds its size limit"
    );
    use crate::installation::operation_journal::OperationJournal;
    match serde_json::from_slice::<OperationJournal>(&bytes)
        .context("Saved operation is malformed; signed out, but profile cleanup needs review")?
    {
        OperationJournal::Depot { version: 1, record } => {
            ensure!(
                crate::installation::operation_journal::depot_path(&record.staging_path) == path,
                "Saved operation path does not match its identity"
            );
            crate::installation::dependency_setup::ensure_setup_quiescent(&record)?;
        }
        OperationJournal::Offline { version: 1, record } => {
            ensure!(
                crate::installation::operation_journal::offline_path(&record)? == path,
                "Saved operation path does not match its identity"
            );
        }
        _ => bail!("Unsupported saved operation; signed out, but profile cleanup needs review"),
    }
    Ok(digest(&bytes))
}

fn remove_owned(path: &Path) -> Result<()> {
    // Anchor every ancestor with O_NOFOLLOW; subsequent deletion is relative to that descriptor.
    let parent = path.parent().context("Reset target has no parent")?;
    let mut directory = File::open("/")?;
    for component in parent.components() {
        if let Component::Normal(name) = component {
            let name = std::ffi::CString::new(name.as_encoded_bytes())?;
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Ok(());
                }
                return Err(error.into());
            }
            directory = unsafe { File::from_raw_fd(fd) };
        }
    }
    remove_entry(
        &directory,
        path.file_name().context("Reset target has no filename")?,
    )
}

fn remove_entry(parent: &File, name: &std::ffi::OsStr) -> Result<()> {
    let name = std::ffi::CString::new(name.as_encoded_bytes())?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd >= 0 {
        let directory = unsafe { File::from_raw_fd(fd) };
        for entry in fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))? {
            remove_entry(&directory, &entry?.file_name())?;
        }
        ensure!(
            unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } == 0,
            "Could not remove an owned profile directory: {}",
            std::io::Error::last_os_error()
        );
    } else {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(());
        }
        if !matches!(error.raw_os_error(), Some(libc::ENOTDIR | libc::ELOOP)) {
            return Err(error.into());
        }
        ensure!(
            unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } == 0,
            "Could not remove an owned profile file: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Plan) {
        let directory = tempfile::tempdir().unwrap();
        let plan = Plan {
            version: 1,
            config: directory.path().join("config"),
            data: directory.path().join("data"),
            cache: directory.path().join("cache"),
            protected: vec![
                directory.path().join("data/games"),
                directory.path().join("data/proton"),
                directory.path().join("data/umu"),
                directory.path().join("data/cloud-save-backups"),
                directory.path().join("data/cloud-save-deletion-recovery"),
                directory.path().join("installers"),
                directory.path().join("extras"),
                directory.path().join("second-installers"),
            ],
            journals: Vec::new(),
        };
        for root in [&plan.config, &plan.data, &plan.cache] {
            fs::create_dir_all(root).unwrap();
        }
        (directory, plan)
    }

    fn write(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"fixture").unwrap();
    }
    fn roots(plan: &Plan) -> [PathBuf; 3] {
        [plan.config.clone(), plan.data.clone(), plan.cache.clone()]
    }
    fn manifest(plan: &Plan) -> Vec<u8> {
        serde_json::to_vec(&Request {
            plan: plan.clone(),
            plan_digest: digest(&serde_json::to_vec(plan).unwrap()),
        })
        .unwrap()
    }
    fn save_manifest(plan: &Plan, bytes: &[u8]) {
        fs::write(plan.config.join(MANIFEST), bytes).unwrap();
        fs::set_permissions(
            plan.config.join(MANIFEST),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }

    #[test]
    fn reset_discards_only_validated_idle_operation_journals_and_preserves_files() {
        let (_root, mut plan) = fixture();
        let library = plan.protected[0].clone();
        let destination = library.join("fixture");
        let path = crate::installation::operation_journal::path(&library, "fixture").unwrap();
        let mut record = crate::state::DepotOperationRecord {
            operation_id: "reset-fixture".into(),
            product_id: 9,
            build_id: "1".into(),
            branch: None,
            kind: "install".into(),
            state: "failed".into(),
            destination: destination.clone(),
            staging_path: library.join(".ludomere/staging/fixture.part"),
            plan_json: "{}".into(),
            bytes_completed: 3,
            total_bytes: Some(5),
            error: Some("inert failed setup".into()),
            created_at: 1,
            updated_at: 1,
            completed_at: None,
        };
        crate::installation::operation_journal::write_depot(&path, &record).unwrap();
        write(&destination.join("payload"));
        write(&library.join(".ludomere/compatibility/fixture/save"));
        write(&record.staging_path.join("partial"));
        plan.journals.push(Journal {
            path: path.clone(),
            hash: journal_hash(&path).unwrap(),
        });
        validate_plan(&plan, &roots(&plan)).unwrap();
        record.plan_json = serde_json::json!({"setup_process_guard": {
            "boot": fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim(), "group": null
        }}).to_string();
        crate::installation::operation_journal::write_depot(&path, &record).unwrap();
        assert!(journal_hash(&path).is_err());
        assert!(remove_profile(&plan).is_err());
        assert!(path.is_file());
        record.plan_json = "{}".into();
        crate::installation::operation_journal::write_depot(&path, &record).unwrap();
        remove_profile(&plan).unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read(destination.join("payload")).unwrap(), b"fixture");
        assert!(
            library
                .join(".ludomere/compatibility/fixture/save")
                .is_file()
        );
        assert!(record.staging_path.join("partial").is_file());
        remove_profile(&plan).unwrap();
    }

    #[test]
    fn reset_removes_profile_and_sidecars_preserving_payloads_and_unknown_files() {
        let (_directory, plan) = fixture();
        for path in targets(&plan).unwrap() {
            if path.extension().is_some() {
                write(&path);
            } else {
                write(&path.join("fixture"));
            }
        }
        write(
            &plan
                .cache
                .join("comet-session-12-345/heroic/gog_store/auth.json"),
        );
        for root in &plan.protected {
            write(&root.join("payload"));
        }
        write(&plan.data.join("comet/state/comet/redist/peer.dll"));
        write(&plan.data.join("unrecognized"));
        let signed_out = plan.config.join(".gog-signed-out");
        fs::write(&signed_out, b"signed out; external credential may remain").unwrap();
        save_manifest(&plan, &manifest(&plan));
        validate_plan(&plan, &roots(&plan)).unwrap();
        remove_profile(&plan).unwrap();
        assert!(targets(&plan).unwrap().iter().all(|path| !path.exists()));
        for root in &plan.protected {
            assert_eq!(fs::read(root.join("payload")).unwrap(), b"fixture");
        }
        assert!(
            plan.data
                .join("comet/state/comet/redist/peer.dll")
                .is_file()
        );
        assert!(plan.data.join("unrecognized").is_file());
        assert_eq!(
            fs::read(&signed_out).unwrap(),
            b"signed out; external credential may remain"
        );
        assert!(!plan.config.join(MANIFEST).exists());
    }

    #[test]
    fn nested_custom_libraries_installers_and_proton_refuse_entire_reset() {
        for relative in [
            "media/library",
            "products/installers",
            "media/custom-proton",
        ] {
            let (_directory, mut plan) = fixture();
            plan.protected.push(plan.cache.join(relative));
            write(&plan.cache.join(relative).join("payload"));
            write(&plan.config.join("config.toml"));
            assert!(
                validate_plan(&plan, &roots(&plan))
                    .unwrap_err()
                    .to_string()
                    .contains("overlaps")
            );
            assert!(plan.config.join("config.toml").is_file());
            assert!(plan.cache.join(relative).join("payload").is_file());
        }
        let (_directory, mut plan) = fixture();
        plan.config = plan.cache.join("media/config");
        fs::create_dir_all(&plan.config).unwrap();
        assert!(
            validate_plan(&plan, &roots(&plan))
                .unwrap_err()
                .to_string()
                .contains("profile root")
        );
    }

    #[test]
    fn reset_refuses_symlink_targets_and_never_follows_nested_links() {
        let (directory, plan) = fixture();
        let outside = directory.path().join("external");
        write(&outside.join("save"));
        std::os::unix::fs::symlink(&outside, plan.cache.join("media")).unwrap();
        assert!(validate_plan(&plan, &roots(&plan)).is_err());
        fs::remove_file(plan.cache.join("media")).unwrap();
        fs::create_dir(plan.cache.join("media")).unwrap();
        std::os::unix::fs::symlink(&outside, plan.cache.join("media/link")).unwrap();
        validate_plan(&plan, &roots(&plan)).unwrap();
        remove_profile(&plan).unwrap();
        assert!(outside.join("save").is_file());
        std::os::unix::fs::symlink(&outside, plan.cache.join("parent")).unwrap();
        assert!(remove_owned(&plan.cache.join("parent/save")).is_err());
        assert!(outside.join("save").is_file());
    }

    #[test]
    fn reset_refuses_profile_targets_overlapping_cloud_deletion_recovery() {
        let (_root, mut plan) = fixture();
        plan.cache = plan.data.join("cloud-save-deletion-recovery");
        write(&plan.cache.join("media/recovered-save"));
        assert!(
            validate_plan(&plan, &roots(&plan))
                .unwrap_err()
                .to_string()
                .contains("overlaps")
        );
        assert_eq!(
            fs::read(plan.cache.join("media/recovered-save")).unwrap(),
            b"fixture"
        );
    }

    #[test]
    fn manifest_rejects_changes_oversize_permissions_links_and_wrong_roots() {
        assert!(
            complete(&[
                "ludomere".into(),
                CLEANUP_ARGUMENT.into(),
                "0".repeat(64),
                i32::MAX.to_string()
            ])
            .unwrap_err()
            .to_string()
            .contains("not open")
        );
        let (_directory, mut plan) = fixture();
        let bytes = manifest(&plan);
        save_manifest(&plan, &bytes);
        assert!(read_request_at(&plan.config, Some(&digest(&bytes))).is_ok());
        assert!(read_request_at(&plan.config, Some(&"0".repeat(64))).is_err());
        fs::set_permissions(
            plan.config.join(MANIFEST),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(read_request_at(&plan.config, None).is_err());
        save_manifest(&plan, &bytes);
        fs::hard_link(plan.config.join(MANIFEST), plan.config.join("linked")).unwrap();
        assert!(read_request_at(&plan.config, None).is_err());
        fs::remove_file(plan.config.join("linked")).unwrap();
        save_manifest(&plan, &vec![b' '; LIMIT as usize + 1]);
        assert!(read_request_at(&plan.config, None).is_err());
        save_manifest(&plan, b"{}");
        assert!(read_request_at(&plan.config, None).is_err());
        fs::remove_file(plan.config.join(MANIFEST)).unwrap();
        let fifo =
            std::ffi::CString::new(plan.config.join(MANIFEST).as_os_str().as_encoded_bytes())
                .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(read_request_at(&plan.config, None).is_err());
        let expected = roots(&plan);
        plan.version = 2;
        assert!(validate_plan(&plan, &expected).is_err());
        plan.version = 1;
        plan.data = plan.cache.clone();
        assert!(validate_plan(&plan, &expected).is_err());
    }

    #[test]
    fn interrupted_reset_keeps_request_and_retry_finishes_remaining_files() {
        let (_directory, plan) = fixture();
        save_manifest(&plan, &manifest(&plan));
        write(&plan.config.join("config.toml"));
        write(&plan.data.join("library.sqlite3"));
        remove_owned(&plan.config.join("config.toml")).unwrap();
        assert!(plan.config.join(MANIFEST).is_file());
        let (request, _) = read_request_at(&plan.config, None).unwrap();
        validate_plan(&request.plan, &roots(&plan)).unwrap();
        remove_profile(&request.plan).unwrap();
        assert!(!plan.data.join("library.sqlite3").exists());
        assert!(!plan.config.join(MANIFEST).exists());
    }

    #[test]
    fn lifetime_shared_locks_exclude_reset_until_other_instances_close() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCK);
        let first = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        lock_profile(&first, false, true).unwrap();
        lock_profile(&second, false, true).unwrap();
        assert!(lock_profile(&first, true, false).is_err());
        // A failed upgrade must retain first's shared lifetime lock.
        assert!(lock_profile(&second, true, false).is_err());
        drop(second);
        lock_profile(&first, true, false).unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(lock_profile(&contender, false, false).is_err());
    }

    #[test]
    fn active_operations_block_reset_and_dropped_reservation_unfreezes_starts() {
        if std::env::var_os("LUDOMERE_TEST_RESET_ACTIVITY").is_none() {
            assert!(std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","profile_reset::tests::active_operations_block_reset_and_dropped_reservation_unfreezes_starts"]).env("LUDOMERE_TEST_RESET_ACTIVITY","1").status().unwrap().success());
            return;
        }
        let guard = begin_activity("reset test").unwrap();
        assert!(reserve().is_err());
        drop(guard);
        let reservation = reserve().unwrap();
        assert!(begin_activity("late payload").is_err());
        drop(reservation);
        assert!(begin_activity("retry payload").is_ok());
    }
}
