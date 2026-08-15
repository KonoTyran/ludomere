use crate::{domain::InstalledGame, state::StateStore};
use anyhow::{Context, Result};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Mutex, OnceLock, mpsc},
    thread,
    time::{Duration, Instant},
};

static RUNNING_GAMES: OnceLock<Mutex<HashMap<i64, mpsc::Sender<()>>>> = OnceLock::new();
static STOPPING_GAMES: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();

fn running_games() -> &'static Mutex<HashMap<i64, mpsc::Sender<()>>> {
    RUNNING_GAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn stopping_games() -> &'static Mutex<HashSet<i64>> {
    STOPPING_GAMES.get_or_init(|| Mutex::new(HashSet::new()))
}

#[derive(Debug)]
pub enum LaunchEvent {
    Started,
    Exited {
        started_at: i64,
        seconds: u64,
        exit_code: Option<i32>,
    },
    Failed(String),
}

pub fn launch_game(game: InstalledGame) -> mpsc::Receiver<LaunchEvent> {
    let (sender, receiver) = mpsc::channel();
    let (stop_sender, stop_receiver) = mpsc::channel();
    {
        let mut games = running_games().lock().unwrap();
        if games.contains_key(&game.product_id) {
            let _ = sender.send(LaunchEvent::Failed("the game is already running".into()));
            return receiver;
        }
        games.insert(game.product_id, stop_sender);
    }
    thread::spawn(move || {
        let product_id = game.product_id;
        match run_game(&game, &sender, &stop_receiver) {
            Ok((started_at, seconds, exit_code)) => {
                let _ = sender.send(LaunchEvent::Exited {
                    started_at,
                    seconds,
                    exit_code,
                });
            }
            Err(error) => {
                let _ = sender.send(LaunchEvent::Failed(format!("{error:#}")));
            }
        }
        running_games().lock().unwrap().remove(&product_id);
        stopping_games().lock().unwrap().remove(&product_id);
    });
    receiver
}

pub fn is_game_running(product_id: i64) -> bool {
    running_games().lock().unwrap().contains_key(&product_id)
}

pub fn stop_game(product_id: i64) -> bool {
    if is_game_stopping(product_id) {
        return true;
    }
    let sent = running_games()
        .lock()
        .unwrap()
        .get(&product_id)
        .is_some_and(|sender| sender.send(()).is_ok());
    if sent {
        stopping_games().lock().unwrap().insert(product_id);
    }
    sent
}

pub fn is_game_stopping(product_id: i64) -> bool {
    stopping_games().lock().unwrap().contains(&product_id)
}

pub fn stop_all_games() {
    let product_ids = running_games()
        .lock()
        .unwrap()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for product_id in product_ids {
        stop_game(product_id);
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if running_games().lock().unwrap().is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn run_game(
    game: &InstalledGame,
    events: &mpsc::Sender<LaunchEvent>,
    stop: &mpsc::Receiver<()>,
) -> Result<(i64, u64, Option<i32>)> {
    let mut game = game.clone();
    if refresh_fallback_profile(&mut game, crate::compatibility::resolve_profile) {
        game.updated_at = chrono::Utc::now().timestamp();
        if let Ok(store) = StateStore::open()
            && let Err(error) = super::save_game_preferences(&store, &game)
        {
            tracing::warn!(
                product_id = game.product_id,
                %error,
                "could not persist newly resolved UMU profile"
            );
        }
    }
    let executable = game
        .primary_executable
        .as_ref()
        .filter(|path| path.is_file())
        .context("the configured game executable is missing")?;
    let log_path = runtime_log_path(game.product_id)?;
    let stdout = File::create(&log_path)
        .with_context(|| format!("could not create runtime log {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .context("could not duplicate runtime log handle")?;
    let working_directory = launch_working_directory(&game, executable);
    let fix_overrides = StateStore::open()
        .and_then(|store| store.compatibility_fix_overrides(game.product_id))
        .unwrap_or_default();
    let fixes = crate::compatibility::effective_fixes(game.product_id, &fix_overrides);
    let _comet = start_online_services_fix(&game, &fixes, &log_path);
    let started_at = chrono::Utc::now().timestamp();
    let timer = Instant::now();
    let mut command = if let Some(compatibility) = &game.compatibility {
        let library = game
            .installation_directory
            .parent()
            .context("installation has no library root")?;
        let prefix = crate::compatibility::prefix_path(library, &compatibility.prefix_slug);
        crate::compatibility::configure_library_drive(&prefix, library)
            .map_err(|error| anyhow::anyhow!(error))?;
        let mut command = Command::new("/usr/bin/umu-run");
        command
            .env("WINEPREFIX", prefix)
            .env("GAMEID", &compatibility.profile.game_id)
            .env("STORE", "gog")
            .env("PROTON_VERB", "waitforexitandrun")
            .arg(executable)
            .args(&game.launch_arguments);
        command
    } else {
        let mut command = Command::new(executable);
        command.args(&game.launch_arguments);
        apply_native_launch_fixes(
            &mut command,
            game.product_id,
            &game.installation_directory,
            &fixes,
        )?;
        command
    };
    command
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .with_context(|| format!("could not launch {}", executable.display()))?;
    let process_group = child.id();
    events.send(LaunchEvent::Started).ok();
    let mut leader_status = None;
    let status = loop {
        if stop.try_recv().is_ok() {
            stop_process_group(process_group, &mut child)?;
            break match leader_status {
                Some(status) => status,
                None => child.wait().context("could not monitor the stopped game")?,
            };
        }
        if leader_status.is_none() {
            leader_status = child
                .try_wait()
                .context("could not monitor the running game")?;
        }
        if let Some(status) = leader_status
            && !process_group_is_running(process_group)
        {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    let seconds = timer.elapsed().as_secs();
    StateStore::open()?.record_game_session(game.product_id, started_at, seconds)?;
    if !status.success()
        && let Ok(mut log) = OpenOptions::new().append(true).open(&log_path)
    {
        let _ = writeln!(
            log,
            "\n[Ludomere] Game process exited with status {status}."
        );
    }
    Ok((started_at, seconds, status.code()))
}

fn bundled_libraries(root: &std::path::Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_owned()];
    let mut libraries = Vec::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                libraries.push(path);
            }
        }
    }
    libraries.sort();
    libraries.dedup();
    libraries
}

fn start_online_services_fix(
    game: &InstalledGame,
    fixes: &[crate::compatibility::LaunchFixDefinition],
    log_path: &std::path::Path,
) -> Option<crate::compatibility::comet::CometSession> {
    if !fixes.iter().any(|fix| {
        matches!(
            fix.operation,
            crate::compatibility::LaunchFixOperation::StartGogOnlineServices
        )
    }) {
        return None;
    }
    let result = if let Some(compatibility) = &game.compatibility {
        let library = game.installation_directory.parent()?;
        let prefix = crate::compatibility::prefix_path(library, &compatibility.prefix_slug);
        crate::compatibility::comet::start(&prefix, &compatibility.profile, log_path)
    } else {
        crate::compatibility::comet::start_native(log_path)
    };
    match result {
        Ok(session) => session,
        Err(error) => {
            if let Ok(mut log) = OpenOptions::new().append(true).open(log_path) {
                let _ = writeln!(log, "[Ludomere] GOG online services unavailable: {error:#}");
            }
            None
        }
    }
}

fn apply_native_launch_fixes(
    command: &mut Command,
    product_id: i64,
    root: &std::path::Path,
    fixes: &[crate::compatibility::LaunchFixDefinition],
) -> Result<()> {
    let files = bundled_libraries(root);
    let mut library_paths = Vec::new();
    let mut preload = Vec::new();
    let managed_directory = crate::identity::cache_root()
        .join("compatibility-libraries")
        .join(product_id.to_string());
    if managed_directory.symlink_metadata().is_ok() {
        fs::remove_dir_all(&managed_directory).with_context(|| {
            format!(
                "could not reset managed compatibility libraries at {}",
                managed_directory.display()
            )
        })?;
    }
    let mut managed_files = HashMap::new();
    for fix in fixes {
        match &fix.operation {
            crate::compatibility::LaunchFixOperation::AddBundledLibraryDirectories {
                filename_prefix,
            } => {
                library_paths.extend(files.iter().filter_map(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .filter(|name| name.starts_with(filename_prefix))
                        .and_then(|_| path.parent().map(PathBuf::from))
                }));
            }
            crate::compatibility::LaunchFixOperation::ClearBundledLibraryExecutableStack {
                filename_prefix,
            } => {
                if fs::create_dir_all(&managed_directory).is_err() {
                    continue;
                }
                for source in files.iter().filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(filename_prefix))
                }) {
                    let Some(filename) = source.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let destination = managed_directory.join(filename);
                    if fs::copy(source, &destination).is_ok()
                        && clear_elf_executable_stack(&destination).is_ok()
                    {
                        managed_files.insert(filename.to_owned(), destination);
                    }
                }
                if !managed_files.is_empty() {
                    library_paths.insert(0, managed_directory.clone());
                }
            }
            crate::compatibility::LaunchFixOperation::PreloadBundledLibrary { filename } => {
                // LD_PRELOAD treats whitespace as a separator, so an absolute
                // path beneath a directory such as "GOG Games" is invalid.
                // The companion library-path fix lets the loader resolve this
                // basename (normally the library's SONAME) safely instead.
                if files
                    .iter()
                    .any(|path| path.file_name().and_then(|name| name.to_str()) == Some(filename))
                {
                    preload.push(PathBuf::from(filename));
                }
            }
            crate::compatibility::LaunchFixOperation::CreateBundledLibraryAlias {
                filename,
                alias,
            } => {
                if !valid_library_filename(filename) || !valid_library_filename(alias) {
                    continue;
                }
                let source = managed_files.get(filename).or_else(|| {
                    files.iter().find(|path| {
                        path.file_name().and_then(|name| name.to_str()) == Some(filename)
                    })
                });
                let Some(source) = source else {
                    continue;
                };
                if fs::create_dir_all(&managed_directory).is_err() {
                    continue;
                }
                let destination = managed_directory.join(alias);
                if destination.symlink_metadata().is_ok() && fs::remove_file(&destination).is_err()
                {
                    continue;
                }
                #[cfg(unix)]
                if std::os::unix::fs::symlink(source, &destination).is_ok() {
                    library_paths.push(managed_directory.clone());
                }
            }
            crate::compatibility::LaunchFixOperation::StartGogOnlineServices => {}
        }
    }
    set_path_environment(command, "LD_LIBRARY_PATH", library_paths);
    set_path_environment(command, "LD_PRELOAD", preload);
    Ok(())
}

fn clear_elf_executable_stack(path: &std::path::Path) -> std::io::Result<()> {
    let mut bytes = fs::read(path)?;
    if bytes.get(..4) != Some(b"\x7fELF") || bytes.get(5) != Some(&1) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unsupported ELF file",
        ));
    }
    let (offset_at, entry_size_at, count_at, flags_at) = match bytes.get(4) {
        Some(2) => (32, 54, 56, 4),
        Some(1) => (28, 42, 44, 24),
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsupported ELF class",
            ));
        }
    };
    let read_u16 = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|value| u16::from_le_bytes([value[0], value[1]]))
    };
    let entry_size = read_u16(entry_size_at)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated ELF"))?
        as usize;
    let count = read_u16(count_at)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated ELF"))?
        as usize;
    let offset_bytes = bytes
        .get(offset_at..offset_at + if offset_at == 32 { 8 } else { 4 })
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated ELF"))?;
    let offset = if offset_at == 32 {
        u64::from_le_bytes(offset_bytes.try_into().unwrap()) as usize
    } else {
        u32::from_le_bytes(offset_bytes.try_into().unwrap()) as usize
    };
    for index in 0..count {
        let header = offset.saturating_add(index.saturating_mul(entry_size));
        let kind: [u8; 4] = bytes
            .get(header..header + 4)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated ELF"))?
            .try_into()
            .unwrap();
        if u32::from_le_bytes(kind) == 0x6474_e551 {
            let at = header + flags_at;
            let flags: [u8; 4] = bytes
                .get(at..at + 4)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated ELF")
                })?
                .try_into()
                .unwrap();
            bytes[at..at + 4].copy_from_slice(&(u32::from_le_bytes(flags) & !1).to_le_bytes());
        }
    }
    fs::write(path, bytes)
}

fn valid_library_filename(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\0')
}

fn set_path_environment(command: &mut Command, name: &str, mut paths: Vec<PathBuf>) {
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return;
    }
    if let Some(existing) = std::env::var_os(name) {
        paths.extend(std::env::split_paths(&existing));
    }
    if let Ok(value) = std::env::join_paths(paths) {
        command.env(name, value);
    }
}

fn refresh_fallback_profile(
    game: &mut InstalledGame,
    resolve: impl FnOnce(
        i64,
    ) -> (
        crate::compatibility::UmuProfile,
        Option<crate::compatibility::UmuDatabaseEntry>,
    ),
) -> bool {
    let Some(compatibility) = game.compatibility.as_mut() else {
        return false;
    };
    if compatibility.profile.source != crate::compatibility::UmuProfileSource::DefaultFallback {
        return false;
    }
    let (profile, entry) = resolve(game.product_id);
    if entry.is_none() {
        return false;
    }
    compatibility.profile = profile;
    true
}

fn launch_working_directory<'a>(
    game: &'a InstalledGame,
    executable: &'a std::path::Path,
) -> &'a std::path::Path {
    // GOG Windows play tasks locate an executable relative to the payload
    // root. Subdirectory executables still expect resources and data relative
    // to that root (for example Grim Dawn's x64 executable). This also matches
    // the working directory in GOG-generated shortcuts.
    if game.compatibility.is_some() {
        &game.installation_directory
    } else {
        executable.parent().unwrap_or(&game.installation_directory)
    }
}

#[cfg(unix)]
fn process_group_is_running(process_group: u32) -> bool {
    let Ok(group) = i32::try_from(process_group) else {
        return false;
    };
    // SAFETY: signal zero checks the existence of a process group without delivering a signal.
    unsafe { libc::kill(-group, 0) == 0 }
}

#[cfg(not(unix))]
fn process_group_is_running(_process_group: u32) -> bool {
    false
}

#[cfg(unix)]
fn stop_process_group(process_group: u32, child: &mut std::process::Child) -> Result<()> {
    let group = i32::try_from(process_group).context("game process ID is out of range")?;
    // SAFETY: kill is called with a valid negative process-group ID and no borrowed memory.
    if unsafe { libc::kill(-group, libc::SIGTERM) } != 0 {
        child.kill().context("could not stop the running game")?;
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let _ = child.try_wait()?;
        if !process_group_is_running(process_group) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: kill is called with the same valid process-group ID as above.
    unsafe { libc::kill(-group, libc::SIGKILL) };
    Ok(())
}

#[cfg(not(unix))]
fn stop_process_group(_process_group: u32, child: &mut std::process::Child) -> Result<()> {
    child.kill().context("could not stop the running game")
}

pub fn runtime_log_path(product_id: i64) -> Result<PathBuf> {
    let root = crate::identity::runtime_logs();
    fs::create_dir_all(&root)?;
    Ok(root.join(format!("{product_id}.log")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compatibility::{
        CompatibilityBackendKind, GameCompatibilityPreferences, UmuProfile, UmuProfileSource,
    };
    use std::{process::Command, thread, time::Duration};

    fn test_game(root: PathBuf, executable: PathBuf, windows: bool) -> InstalledGame {
        InstalledGame {
            product_id: 1,
            library_id: "test".into(),
            installed_version: Some("1".into()),
            installation_directory: root,
            installer_revision_id: None,
            installer_job_id: None,
            installer_files: Vec::new(),
            installer_complete: true,
            installer_operating_system: Some(if windows { "windows" } else { "linux" }.into()),
            installer_language: Some("English".into()),
            compatibility: windows.then(|| GameCompatibilityPreferences {
                backend: CompatibilityBackendKind::Umu,
                prefix_slug: "game".into(),
                profile: UmuProfile {
                    game_id: "umu-default".into(),
                    store: "gog".into(),
                    source: UmuProfileSource::DefaultFallback,
                },
                pending_profile: None,
            }),
            primary_executable: Some(executable),
            launch_arguments: Vec::new(),
            state: crate::domain::InstallationState::Installed,
            error: None,
            installed_at: Some(1),
            verified_at: None,
            last_played_at: None,
            playtime_seconds: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn windows_subdirectory_executable_uses_payload_root_as_working_directory() {
        let root = PathBuf::from("/games/grim_dawn");
        let executable = root.join("x64/Grim Dawn.exe");
        let game = test_game(root.clone(), executable.clone(), true);
        assert_eq!(launch_working_directory(&game, &executable), root);
    }

    #[test]
    fn native_executable_keeps_its_parent_as_working_directory() {
        let root = PathBuf::from("/games/native");
        let executable = root.join("game/start.sh");
        let game = test_game(root, executable.clone(), false);
        assert_eq!(
            launch_working_directory(&game, &executable),
            executable.parent().unwrap()
        );
    }

    #[test]
    fn fallback_profile_is_rechecked_and_upgraded_before_launch() {
        let mut game = test_game(
            PathBuf::from("/games/test"),
            PathBuf::from("game.exe"),
            true,
        );
        let changed = refresh_fallback_profile(&mut game, |_| {
            (
                UmuProfile {
                    game_id: "umu-42".into(),
                    store: "gog".into(),
                    source: UmuProfileSource::GogProductId,
                },
                Some(crate::compatibility::UmuDatabaseEntry {
                    title: "Test".into(),
                    store: "gog".into(),
                    codename: "1".into(),
                    umu_id: "umu-42".into(),
                    executable_pattern: None,
                    notes: None,
                }),
            )
        });
        assert!(changed);
        assert_eq!(game.compatibility.unwrap().profile.game_id, "umu-42");
    }

    #[test]
    fn resolved_profile_is_not_looked_up_again() {
        let mut game = test_game(
            PathBuf::from("/games/test"),
            PathBuf::from("game.exe"),
            true,
        );
        game.compatibility.as_mut().unwrap().profile = UmuProfile {
            game_id: "umu-42".into(),
            store: "gog".into(),
            source: UmuProfileSource::GogProductId,
        };
        let changed = refresh_fallback_profile(&mut game, |_| panic!("unexpected lookup"));
        assert!(!changed);
    }

    #[test]
    fn nonzero_process_exit_retains_its_code_without_being_a_spawn_failure() {
        let status = Command::new("sh").args(["-c", "exit 7"]).status().unwrap();
        assert!(!status.success());
        assert_eq!(status.code(), Some(7));
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_child_keeps_process_group_alive_and_stop_terminates_it() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 &"]);
        command.process_group(0);
        let mut wrapper = command.spawn().unwrap();
        let group = wrapper.id();
        let status = wrapper.wait().unwrap();
        assert!(status.success());
        assert!(process_group_is_running(group));

        stop_process_group(group, &mut wrapper).unwrap();
        for _ in 0..20 {
            if !process_group_is_running(group) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(!process_group_is_running(group));
    }
}
