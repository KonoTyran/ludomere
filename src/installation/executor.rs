use crate::{
    domain::{InstallationState, InstalledGame},
    state::StateStore,
};
use anyhow::{Context, Result, bail};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationEvent {
    Starting {
        message: String,
    },
    Running {
        log_path: PathBuf,
        percentage: Option<u8>,
        message: String,
    },
    Complete {
        executable: Option<PathBuf>,
    },
    Cancelled,
    Failed(String),
    Prompt {
        text: String,
        choices: Vec<String>,
        context: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallationEvent {
    Started,
    Complete,
    Cancelled,
    Failed(String),
}

pub struct UninstallationHandle {
    cancellation: Arc<AtomicBool>,
    pub events: mpsc::Receiver<UninstallationEvent>,
}

#[derive(Clone)]
pub(crate) struct UninstallationControl {
    cancellation: Arc<AtomicBool>,
}

impl UninstallationHandle {
    pub(crate) fn control(&self) -> UninstallationControl {
        UninstallationControl {
            cancellation: self.cancellation.clone(),
        }
    }
}

impl UninstallationControl {
    pub(crate) fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdditionalInstaller {
    pub product_id: i64,
    pub revision_id: Option<i64>,
    #[serde(default)]
    pub version: Option<String>,
    pub title: String,
    pub files: Vec<PathBuf>,
}

pub struct InstallationHandle {
    cancellation: Arc<AtomicBool>,
    pub events: mpsc::Receiver<InstallationEvent>,
    responses: mpsc::Sender<String>,
}

#[derive(Clone)]
pub(crate) struct InstallationControl {
    cancellation: Arc<AtomicBool>,
    responses: mpsc::Sender<String>,
}

impl InstallationControl {
    pub(crate) fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }

    pub(crate) fn respond(&self, response: String) {
        let _ = self.responses.send(response);
    }
}

impl InstallationHandle {
    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }

    pub fn respond(&self, response: String) {
        let _ = self.responses.send(response);
    }

    pub(crate) fn control(&self) -> InstallationControl {
        InstallationControl {
            cancellation: self.cancellation.clone(),
            responses: self.responses.clone(),
        }
    }
}

pub fn start_installation(
    plan: InstalledGame,
    additional_installers: Vec<AdditionalInstaller>,
    install_base: bool,
    interactive_prompts: bool,
) -> InstallationHandle {
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = cancellation.clone();
    let (sender, events) = mpsc::channel();
    let (responses, response_receiver) = mpsc::channel();
    thread::spawn(move || {
        let Some(_permit) = crate::operation_gate::acquire_work(
            crate::state::WorkKind::Installation,
            &plan.product_id.to_string(),
            || worker_cancellation.load(Ordering::Acquire),
        ) else {
            let _ = sender.send(InstallationEvent::Cancelled);
            return;
        };
        if let Err(error) = run_installation(
            &plan,
            &additional_installers,
            install_base,
            interactive_prompts,
            &worker_cancellation,
            &sender,
            &response_receiver,
        ) {
            let message = format!("{error:#}");
            if worker_cancellation.load(Ordering::Acquire) {
                let _ = sender.send(InstallationEvent::Cancelled);
                return;
            }
            if let Ok(store) = StateStore::open() {
                let base_is_installed = !install_base
                    || plan
                        .primary_executable
                        .as_ref()
                        .is_some_and(|path| path.is_file())
                    || discover_linux_executable(&plan.installation_directory).is_some();
                if base_is_installed {
                    let mut installed = plan.clone();
                    installed.state = InstallationState::Installed;
                    installed.error = Some(message.clone());
                    installed.primary_executable = installed
                        .primary_executable
                        .filter(|path| path.is_file())
                        .or_else(|| discover_linux_executable(&installed.installation_directory));
                    let _ = super::save_game_preferences(&store, &installed);
                }
            }
            let _ = sender.send(InstallationEvent::Failed(message));
        }
    });
    InstallationHandle {
        cancellation,
        events,
        responses,
    }
}

pub fn start_uninstallation(game: InstalledGame) -> UninstallationHandle {
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = cancellation.clone();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let Some(_permit) = crate::operation_gate::acquire_work(
            crate::state::WorkKind::Installation,
            &game.product_id.to_string(),
            || worker_cancellation.load(Ordering::Acquire),
        ) else {
            let _ = sender.send(UninstallationEvent::Cancelled);
            return;
        };
        let _ = sender.send(UninstallationEvent::Started);
        match run_uninstallation(&game, &worker_cancellation) {
            Ok(()) => {
                let _ = sender.send(UninstallationEvent::Complete);
            }
            Err(error) => {
                let message = format!("{error:#}");
                if worker_cancellation.load(Ordering::Acquire) {
                    let _ = sender.send(UninstallationEvent::Cancelled);
                    return;
                }
                let _ = sender.send(UninstallationEvent::Failed(message));
            }
        }
    });
    UninstallationHandle {
        cancellation,
        events: receiver,
    }
}

fn run_uninstallation(game: &InstalledGame, cancellation: &AtomicBool) -> Result<()> {
    if super::marker::load(&game.installation_directory)?
        .is_some_and(|marker| marker.source == crate::domain::InstallationSource::GalaxyDepot)
    {
        return run_depot_uninstallation(
            game.product_id,
            &game.installation_directory,
            cancellation,
        );
    }
    if game.compatibility.is_some() {
        return run_windows_uninstallation(game, cancellation);
    }
    let uninstaller = find_native_uninstaller(&game.installation_directory).with_context(|| {
        format!(
            "no native GOG uninstaller was found in {}",
            game.installation_directory.display()
        )
    })?;
    let log_path = uninstallation_log_path(game.product_id)?;
    let mut log = File::create(&log_path)
        .with_context(|| format!("could not open installation log {}", log_path.display()))?;
    writeln!(
        log,
        "Starting native uninstaller: {}",
        uninstaller.display()
    )?;

    let pty = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("could not create uninstaller pseudo-terminal")?;
    let mut command = portable_pty::CommandBuilder::new("sh");
    command.arg(&uninstaller);
    command.arg("--noreadme");
    command.arg("--nooptions");
    command.arg("--noprompt");
    command.cwd(&game.installation_directory);
    let mut child = pty
        .slave
        .spawn_command(command)
        .with_context(|| format!("could not start GOG uninstaller {}", uninstaller.display()))?;
    drop(pty.slave);
    let mut output = pty
        .master
        .try_clone_reader()
        .context("could not capture uninstaller output")?;
    let mut input = pty
        .master
        .take_writer()
        .context("could not open uninstaller input")?;
    input
        .write_all(b"y\n")
        .context("could not confirm GOG uninstallation")?;
    input
        .flush()
        .context("could not send uninstall confirmation")?;
    let log_writer = thread::spawn(move || {
        let _ = std::io::copy(&mut output, &mut log);
        let _ = log.flush();
    });
    let status = loop {
        if cancellation.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            drop(pty.master);
            let _ = log_writer.join();
            bail!("uninstallation cancelled");
        }
        if let Some(status) = child
            .try_wait()
            .context("could not monitor GOG uninstaller")?
        {
            break status;
        }
        thread::sleep(Duration::from_millis(200));
    };
    drop(pty.master);
    let _ = log_writer.join();
    if !status.success() {
        bail!(
            "GOG uninstaller exited with {}. See {}",
            status.exit_code(),
            log_path.display()
        );
    }
    super::marker::remove(&game.installation_directory)?;
    remove_empty_installation_directories(&game.installation_directory);
    Ok(())
}

fn run_depot_uninstallation(
    product_id: i64,
    directory: &Path,
    cancellation: &AtomicBool,
) -> Result<()> {
    let library = directory
        .parent()
        .context("depot installation has no library root")?;
    let marker = super::marker::load(directory)?.context("depot installation marker is missing")?;
    if marker.source != crate::domain::InstallationSource::GalaxyDepot
        || marker.product_id != product_id
        || directory.file_name().and_then(|name| name.to_str()) != Some(marker.slug.as_str())
    {
        bail!("depot installation identity is inconsistent");
    }
    reject_symlink_path(directory)?;
    let journal = super::depot::operation_staging_path(library, directory, &marker.slug, "")?;
    let prefix = marker
        .compatibility
        .filter(|compatibility| compatibility.managed_by_ludomere)
        .map(|compatibility| {
            crate::compatibility::prefix_path(library, &compatibility.prefix_slug)
        });
    if let Some(prefix) = prefix.as_deref() {
        reject_symlink_path(prefix)?;
    }
    if cancellation.load(Ordering::Acquire) {
        bail!("uninstallation cancelled");
    }
    if directory.exists() {
        fs::remove_dir_all(directory)?;
    }
    if let Some(prefix) = prefix.filter(|path| path.exists()) {
        fs::remove_dir_all(prefix)?;
    }
    super::depot_actions::remove_support_staging(&journal)?;
    if journal.exists() {
        fs::remove_file(journal)?;
    }
    Ok(())
}

fn reject_symlink_path(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!("uninstallation path crosses a symlink");
        }
    }
    Ok(())
}

fn run_windows_uninstallation(game: &InstalledGame, cancellation: &AtomicBool) -> Result<()> {
    use crate::compatibility::{CompatibilityBackend, CompatibilityRunRequest};
    let compatibility = game
        .compatibility
        .as_ref()
        .context("Windows compatibility metadata is missing")?;
    let uninstaller = find_windows_uninstaller(&game.installation_directory)
        .context("no Windows GOG uninstaller was found")?;
    let library = game
        .installation_directory
        .parent()
        .context("installation has no library root")?;
    let prefix = crate::compatibility::prefix_path(library, &compatibility.prefix_slug);
    crate::compatibility::configure_library_drive(&prefix, library)
        .map_err(|e| anyhow::anyhow!(e))?;
    let log_path = uninstallation_log_path(game.product_id)?;
    let backend = crate::compatibility::default_backend();
    let profile = crate::compatibility::profile_for_use(game.product_id, &compatibility.profile);
    let mut process = backend.run_executable(CompatibilityRunRequest {
        prefix,
        profile,
        executable: uninstaller.clone(),
        arguments: Vec::new(),
        working_directory: uninstaller.parent().map(PathBuf::from),
        log_path,
        background: false,
    })?;
    loop {
        if cancellation.load(Ordering::Acquire) {
            backend.stop(&mut process)?;
            bail!("uninstallation cancelled")
        }
        if let Some(status) = process.try_wait()? {
            if !status.success() {
                bail!("Windows uninstaller exited unsuccessfully: {status}")
            }
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    super::marker::remove(&game.installation_directory)?;
    remove_empty_installation_directories(&game.installation_directory);
    Ok(())
}

fn find_windows_uninstaller(directory: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    fn walk(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
        if depth > 4 {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, depth + 1, out)
            } else {
                let n = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if n.ends_with(".exe") && (n.starts_with("unins") || n.contains("uninstall")) {
                    out.push(p)
                }
            }
        }
    }
    walk(directory, 0, &mut candidates);
    candidates.sort();
    candidates.into_iter().next()
}

fn remove_empty_installation_directories(root: &Path) {
    fn clean(directory: &Path) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        let children = entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        for child in children {
            clean(&child);
        }
        let _ = fs::remove_dir(directory);
    }
    clean(root);
}

fn find_native_uninstaller(directory: &Path) -> Option<PathBuf> {
    let mut candidates = fs::read_dir(directory)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        let name = name.to_ascii_lowercase();
                        name.starts_with("uninstall") && name.ends_with(".sh")
                    })
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next()
}

fn run_installation(
    plan: &InstalledGame,
    additional_installers: &[AdditionalInstaller],
    install_base: bool,
    interactive_prompts: bool,
    cancellation: &AtomicBool,
    events: &mpsc::Sender<InstallationEvent>,
    responses: &mpsc::Receiver<String>,
) -> Result<()> {
    let windows = plan
        .installer_operating_system
        .as_deref()
        .is_some_and(|os| os.eq_ignore_ascii_case("windows"));
    events
        .send(InstallationEvent::Starting {
            message: if windows {
                "Preparing Windows installer"
            } else {
                "Running native installer"
            }
            .into(),
        })
        .ok();
    if windows {
        return run_windows_installation(
            plan,
            additional_installers,
            install_base,
            interactive_prompts,
            cancellation,
            events,
        );
    }
    if install_base {
        validate_plan(plan)?;
    }
    fs::create_dir_all(&plan.installation_directory).with_context(|| {
        format!(
            "could not create installation directory {}",
            plan.installation_directory.display()
        )
    })?;
    let installer_destination = installer_destination_path(plan)?;
    let log_path = installation_log_path(plan.product_id)?;
    let mut log = File::create(&log_path)
        .with_context(|| format!("could not create installation log {}", log_path.display()))?;
    writeln!(log, "Using hidden pseudo-terminal")?;

    if install_base {
        let installer = linux_installer(
            &plan.installer_files,
            plan.installer_operating_system.as_deref(),
        )?;
        run_native_installer(
            &installer,
            &installer_destination,
            &log_path,
            cancellation,
            interactive_prompts,
            events,
            responses,
        )?;
        let executable = discover_linux_executable(&plan.installation_directory);
        let mut completed = plan.clone();
        let now = chrono::Utc::now().timestamp();
        completed.state = InstallationState::Installed;
        completed.error = None;
        completed.primary_executable = executable;
        completed.installed_at = Some(now);
        completed.updated_at = now;
        let store = StateStore::open()?;
        super::save_game_preferences(&store, &completed)?;
        let marker = super::marker::from_game(&completed, Vec::new());
        super::marker::write(&marker, &completed.installation_directory)?;
    }
    for additional in additional_installers {
        validate_installer_files(&additional.files)
            .with_context(|| format!("DLC installer is incomplete: {}", additional.title))?;
        let installer = linux_installer(&additional.files, Some("linux"))?;
        let mut log = File::options().append(true).open(&log_path)?;
        writeln!(log, "\nInstalling DLC: {}", additional.title)?;
        run_native_installer(
            &installer,
            &installer_destination,
            &log_path,
            cancellation,
            interactive_prompts,
            events,
            responses,
        )?;
        super::marker::record_dlc(
            plan,
            super::marker::InstalledDlc {
                product_id: additional.product_id,
                version: additional.version.clone(),
                revision_id: additional.revision_id,
                installed_at: chrono::Utc::now().timestamp(),
            },
        )?;
    }
    let executable = plan
        .primary_executable
        .clone()
        .filter(|path| path.is_file())
        .or_else(|| discover_linux_executable(&plan.installation_directory));
    let mut completed = plan.clone();
    let now = chrono::Utc::now().timestamp();
    completed.state = InstallationState::Installed;
    completed.error = None;
    completed.primary_executable = executable.clone();
    completed.installed_at = Some(now);
    completed.updated_at = now;
    let store = StateStore::open()?;
    super::save_game_preferences(&store, &completed)?;
    store.record_product_activity(completed.product_id, now)?;
    events.send(InstallationEvent::Complete { executable }).ok();
    Ok(())
}

fn run_windows_installation(
    plan: &InstalledGame,
    additional: &[AdditionalInstaller],
    install_base: bool,
    interactive_install: bool,
    cancellation: &AtomicBool,
    events: &mpsc::Sender<InstallationEvent>,
) -> Result<()> {
    use crate::compatibility::{
        CompatibilityBackend, CompatibilityBackendKind, CompatibilityRunRequest,
        GameCompatibilityPreferences, InitializePrefixRequest,
    };
    // Adding DLC to an existing installation does not use the base-game
    // installer. Requiring its archived files here caused DLC-only operations
    // to fail before the installation log was even created.
    if install_base {
        validate_plan(plan)?;
    }
    let backend = crate::compatibility::default_backend();
    let backend_status = backend.status()?;
    if !backend_status.available || !backend_status.healthy {
        bail!(
            "{}",
            backend_status
                .message
                .unwrap_or_else(|| "UMU is unavailable".into())
        );
    }
    let library = plan
        .installation_directory
        .parent()
        .context("installation directory has no library root")?
        .to_owned();
    let slug = plan
        .installation_directory
        .file_name()
        .and_then(|n| n.to_str())
        .context("invalid game slug")?
        .to_owned();
    let (detected_profile, _entry) = crate::compatibility::resolve_profile(plan.product_id);
    let profile = plan
        .compatibility
        .as_ref()
        .map(|c| crate::compatibility::profile_for_use(plan.product_id, &c.profile))
        .unwrap_or(detected_profile);
    let log_path = installation_log_path(plan.product_id)?;
    File::create(&log_path)
        .with_context(|| format!("could not create installation log {}", log_path.display()))?;
    let current_component = Arc::new(Mutex::new(None));
    let _log_monitor =
        UmuLogStatusMonitor::start(log_path.clone(), events.clone(), current_component.clone());
    let prefix = backend.initialize_prefix(InitializePrefixRequest {
        library_id: plan.library_id.clone(),
        library: library.clone(),
        slug: slug.clone(),
        profile: profile.clone(),
        log_path: log_path.clone(),
    })?;
    fs::create_dir_all(&plan.installation_directory)?;
    let prefix_path = library.join(&prefix.relative_path);
    let component_count = usize::from(install_base) + additional.len();
    let run_one = |files: &[PathBuf],
                   title: &str,
                   component_index: usize,
                   cancellation: &AtomicBool|
     -> Result<()> {
        let installer = windows_installer(files)?;
        let component_message =
            format!("Installing {title} — {component_index} of {component_count}");
        *current_component.lock().unwrap() = Some(component_message.clone());
        events
            .send(InstallationEvent::Running {
                log_path: log_path.clone(),
                percentage: None,
                message: component_message,
            })
            .ok();
        let mut process = backend.run_executable(CompatibilityRunRequest {
            prefix: prefix_path.clone(),
            profile: profile.clone(),
            executable: installer.clone(),
            arguments: crate::compatibility::inno_setup_arguments(
                plan.installer_language.as_deref(),
                &crate::compatibility::windows_destination(&slug),
                interactive_install,
            ),
            working_directory: installer.parent().map(PathBuf::from),
            log_path: log_path.clone(),
            background: !interactive_install,
        })?;
        loop {
            if cancellation.load(Ordering::Acquire) {
                backend.stop(&mut process)?;
                bail!("installation cancelled")
            }
            if let Some(status) = process.try_wait()? {
                if !status.success() {
                    bail!("Windows installer exited unsuccessfully: {status}")
                }
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    };
    let mut completed = plan.clone();
    completed.compatibility = Some(GameCompatibilityPreferences {
        backend: CompatibilityBackendKind::Umu,
        prefix_slug: slug.clone(),
        profile: profile.clone(),
        pending_profile: None,
    });
    if install_base {
        run_one(&plan.installer_files, "base game", 1, cancellation)?;
        if !super::directory_has_installed_payload(&plan.installation_directory) {
            bail!(
                "the installer did not place a plausible payload in {} (select {})",
                plan.installation_directory.display(),
                crate::compatibility::windows_destination(&slug)
            );
        }
        let now = chrono::Utc::now().timestamp();
        completed.state = InstallationState::Installed;
        completed.installed_at = Some(now);
        completed.updated_at = now;
        completed.primary_executable = super::discover_windows_executable(
            &plan.installation_directory,
            plan.product_id,
            &slug,
        )
        .selected;
        let store = StateStore::open()?;
        super::save_game_preferences(&store, &completed)?;
        super::marker::write(
            &super::marker::from_game(&completed, Vec::new()),
            &completed.installation_directory,
        )?;
    }
    for (additional_index, dlc) in additional.iter().enumerate() {
        validate_installer_files(&dlc.files)
            .with_context(|| format!("DLC installer is incomplete: {}", dlc.title))?;
        let component_index = usize::from(install_base) + additional_index + 1;
        run_one(&dlc.files, &dlc.title, component_index, cancellation)?;
        super::marker::record_dlc(
            &completed,
            super::marker::InstalledDlc {
                product_id: dlc.product_id,
                version: dlc.version.clone(),
                revision_id: dlc.revision_id,
                installed_at: chrono::Utc::now().timestamp(),
            },
        )?;
    }
    let executable = completed.primary_executable.clone().or_else(|| {
        super::discover_windows_executable(
            &completed.installation_directory,
            completed.product_id,
            &slug,
        )
        .selected
    });
    completed.primary_executable = executable.clone();
    completed.state = InstallationState::Installed;
    completed.updated_at = chrono::Utc::now().timestamp();
    let store = StateStore::open()?;
    super::save_game_preferences(&store, &completed)?;
    store.record_product_activity(completed.product_id, completed.updated_at)?;
    events.send(InstallationEvent::Complete { executable }).ok();
    Ok(())
}

struct UmuLogStatusMonitor {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl UmuLogStatusMonitor {
    fn start(
        log_path: PathBuf,
        events: mpsc::Sender<InstallationEvent>,
        current_component: Arc<Mutex<Option<String>>>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            let mut consumed = 0;
            let mut last_message = None;
            let mut partial_line = String::new();
            while !worker_stop.load(Ordering::Acquire) {
                if let Ok(bytes) = fs::read(&log_path) {
                    if bytes.len() < consumed {
                        consumed = 0;
                    }
                    if bytes.len() > consumed {
                        partial_line.push_str(&String::from_utf8_lossy(&bytes[consumed..]));
                        consumed = bytes.len();
                        let complete_length = partial_line
                            .rfind('\n')
                            .map_or(0, |newline| newline.saturating_add(1));
                        let complete = partial_line[..complete_length].to_owned();
                        partial_line.drain(..complete_length);
                        for line in complete.lines() {
                            let component = current_component.lock().unwrap().clone();
                            let Some(message) = umu_widget_status(line, component.as_deref())
                            else {
                                continue;
                            };
                            if last_message.as_deref() == Some(message.as_str()) {
                                continue;
                            }
                            last_message = Some(message.clone());
                            let _ = events.send(InstallationEvent::Running {
                                log_path: log_path.clone(),
                                percentage: None,
                                message,
                            });
                        }
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for UmuLogStatusMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn umu_widget_status(line: &str, current_component: Option<&str>) -> Option<String> {
    let message = if line.contains("Downloading steamrt")
        || line.contains("Downloading umu runtime")
    {
        "Downloading UMU runtime"
    } else if line.contains("Setting up Unified Launcher") {
        "Preparing UMU runtime"
    } else if line.contains("SHA256 is OK") || line.contains("Verifying integrity of") {
        "Verifying UMU runtime"
    } else if line.contains("Downloading UMU-Proton") {
        "Downloading UMU-Proton"
    } else if line.contains("Extracting UMU-Proton") {
        "Installing UMU-Proton"
    } else if line.contains("Upgrading prefix from") {
        "Creating Windows compatibility environment"
    } else if line.contains("Running protonfixes on") {
        "Applying game compatibility fixes"
    } else if line.contains("Installing winetricks") || line.contains("Using winetricks verb") {
        "Installing Windows dependencies"
    } else if line.contains("Winetricks complete") {
        "Finalizing compatibility environment"
    } else if line.starts_with("Proton:") && (line.contains("setup_") || line.contains(".exe")) {
        current_component?
    } else {
        return None;
    };
    Some(message.to_owned())
}

fn windows_installer(files: &[PathBuf]) -> Result<PathBuf> {
    files
        .iter()
        .find(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        })
        .cloned()
        .context("Windows installer executable is missing")
}
fn run_native_installer(
    installer: &Path,
    destination: &Path,
    log_path: &Path,
    cancellation: &AtomicBool,
    interactive_prompts: bool,
    events: &mpsc::Sender<InstallationEvent>,
    responses: &mpsc::Receiver<String>,
) -> Result<()> {
    let mut log = File::options().append(true).open(log_path)?;
    let arguments = [
        installer.to_string_lossy().into_owned(),
        "--".into(),
        "--i-agree-to-all-licenses".into(),
        "--noreadme".into(),
        "--nooptions".into(),
        "--noprompt".into(),
        format!("--destination={}", destination.display()),
    ];
    crate::compatibility::append_step_log(
        log_path,
        &format!(
            "command: sh {}",
            arguments
                .iter()
                .map(|argument| shell_words::quote(argument))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    )?;
    let pty = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("could not create installer pseudo-terminal")?;
    let mut command = portable_pty::CommandBuilder::new("sh");
    for argument in &arguments {
        command.arg(argument);
    }
    command.cwd(installer.parent().unwrap_or_else(|| Path::new(".")));
    let mut child = pty
        .slave
        .spawn_command(command)
        .with_context(|| format!("could not start Linux installer {}", installer.display()))?;
    drop(pty.slave);
    let mut output = pty
        .master
        .try_clone_reader()
        .context("could not capture installer pseudo-terminal")?;
    let mut input = pty
        .master
        .take_writer()
        .context("could not open installer input")?;
    let (output_sender, output_receiver) = mpsc::channel::<String>();
    let log_writer = thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        while let Ok(read) = output.read(&mut buffer) {
            if read == 0 {
                break;
            }
            let _ = log.write_all(&buffer[..read]);
            let _ = output_sender.send(String::from_utf8_lossy(&buffer[..read]).into_owned());
        }
        let _ = log.flush();
    });
    let mut prompt_tail = String::new();
    let mut last_percentage = None;
    events
        .send(InstallationEvent::Running {
            log_path: log_path.to_path_buf(),
            percentage: None,
            message: "Running native installer".into(),
        })
        .ok();
    loop {
        while let Ok(chunk) = output_receiver.try_recv() {
            prompt_tail.push_str(&chunk);
            if prompt_tail.len() > 16 * 1024 {
                prompt_tail.drain(..prompt_tail.len() - 16 * 1024);
            }
            let percentage = parse_native_installer_progress(&prompt_tail);
            if percentage != last_percentage {
                last_percentage = percentage;
                events
                    .send(InstallationEvent::Running {
                        log_path: log_path.to_path_buf(),
                        percentage,
                        message: "Running native installer".into(),
                    })
                    .ok();
            }
            match installer_prompt_action(&prompt_tail) {
                InstallerPromptAction::ReplaceExisting => {
                    let response = if interactive_prompts {
                        request_installer_response(
                            events,
                            responses,
                            cancellation,
                            "An installed file already exists. Replace it?".into(),
                            vec!["Yes".into(), "No".into(), "Always".into(), "Never".into()],
                            installer_prompt_context(&prompt_tail),
                        )?
                    } else {
                        "Always".into()
                    };
                    input
                        .write_all(format!("{response}\n").as_bytes())
                        .context("could not answer installer replacement prompt")?;
                    input.flush().context("could not send installer response")?;
                    prompt_tail.clear();
                }
                InstallerPromptAction::Unknown(prompt) => {
                    if interactive_prompts {
                        let response = request_installer_response(
                            events,
                            responses,
                            cancellation,
                            prompt,
                            Vec::new(),
                            installer_prompt_context(&prompt_tail),
                        )?;
                        input.write_all(format!("{response}\n").as_bytes())?;
                        input.flush()?;
                        prompt_tail.clear();
                    } else {
                        let _ = child.kill();
                        let _ = child.wait();
                        drop(pty.master);
                        let _ = log_writer.join();
                        bail!("installer requires an unsupported response: {prompt}");
                    }
                }
                InstallerPromptAction::None => {}
            }
        }
        if cancellation.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            drop(pty.master);
            let _ = log_writer.join();
            bail!("installation cancelled");
        }
        if let Some(status) = child
            .try_wait()
            .context("could not monitor Linux installer")?
        {
            drop(pty.master);
            let _ = log_writer.join();
            if !status.success() {
                bail!(
                    "Linux installer exited with {}. See {}",
                    status.exit_code(),
                    log_path.display()
                );
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn parse_native_installer_progress(output: &str) -> Option<u8> {
    let marker = "(total progress: ";
    let start = output.rfind(marker)? + marker.len();
    let percentage = output[start..].split('%').next()?.trim().parse().ok()?;
    (percentage <= 100).then_some(percentage)
}

fn request_installer_response(
    events: &mpsc::Sender<InstallationEvent>,
    responses: &mpsc::Receiver<String>,
    cancellation: &AtomicBool,
    text: String,
    choices: Vec<String>,
    context: String,
) -> Result<String> {
    events
        .send(InstallationEvent::Prompt {
            text,
            choices,
            context,
        })
        .ok();
    loop {
        if cancellation.load(Ordering::Acquire) {
            bail!("installation cancelled while waiting for a response");
        }
        match responses.recv_timeout(Duration::from_millis(200)) {
            Ok(response) if !response.trim().is_empty() => return Ok(response),
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("installer response dialog was closed")
            }
        }
    }
}

fn installer_prompt_context(output: &str) -> String {
    let cleaned = output.replace(['\r', '\u{8}'], "");
    let lines = cleaned
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines[lines.len().saturating_sub(10)..].join("\n")
}

#[derive(Debug, PartialEq, Eq)]
enum InstallerPromptAction {
    None,
    ReplaceExisting,
    Unknown(String),
}

fn installer_prompt_action(output: &str) -> InstallerPromptAction {
    let normalized = output.replace(['\r', '\u{8}'], "");
    let lower = normalized.to_ascii_lowercase();
    if lower.contains("already exists! replace?")
        && lower.trim_end().ends_with("[y/n/always/never]:")
    {
        return InstallerPromptAction::ReplaceExisting;
    }
    let last_line = normalized.lines().next_back().unwrap_or_default().trim();
    if (last_line.ends_with(":") || last_line.ends_with("?"))
        && (last_line.contains('[') || last_line.contains("(y/n)"))
    {
        return InstallerPromptAction::Unknown(last_line.to_owned());
    }
    InstallerPromptAction::None
}

fn installer_destination_path(plan: &InstalledGame) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let aliases = crate::identity::data_root().join("install-targets");
        fs::create_dir_all(&aliases)?;
        let alias = aliases.join(plan.product_id.to_string());
        match fs::symlink_metadata(&alias) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if fs::read_link(&alias).ok().as_deref() != Some(&plan.installation_directory) {
                    fs::remove_file(&alias)?;
                    symlink(&plan.installation_directory, &alias)?;
                }
            }
            Ok(_) => bail!(
                "installer destination alias is not a symbolic link: {}",
                alias.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                symlink(&plan.installation_directory, &alias)?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(alias)
    }
    #[cfg(not(unix))]
    Ok(plan.installation_directory.clone())
}

fn validate_plan(plan: &InstalledGame) -> Result<()> {
    if plan.installer_files.is_empty() {
        bail!("installation plan contains no installer files");
    }
    for path in &plan.installer_files {
        if !path.is_file() {
            bail!("installer file is missing: {}", path.display());
        }
    }
    Ok(())
}

fn validate_installer_files(files: &[PathBuf]) -> Result<()> {
    if files.is_empty() {
        bail!("no installer files were selected");
    }
    for path in files {
        if !path.is_file() {
            bail!("required installer file is missing: {}", path.display());
        }
    }
    Ok(())
}

fn linux_installer(files: &[PathBuf], operating_system: Option<&str>) -> Result<PathBuf> {
    if !matches!(operating_system, Some("linux") | Some("Linux")) {
        bail!("the selected installer is not a native Linux installer");
    }
    files
        .iter()
        .find(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sh"))
        })
        .cloned()
        .context("the selected installer set has no .sh launcher")
}

pub fn installation_log_path(product_id: i64) -> Result<PathBuf> {
    activity_log_path(product_id, "install")
}

pub fn uninstallation_log_path(product_id: i64) -> Result<PathBuf> {
    activity_log_path(product_id, "uninstall")
}

fn activity_log_path(product_id: i64, operation: &str) -> Result<PathBuf> {
    let root = crate::identity::installation_logs();
    fs::create_dir_all(&root)?;
    Ok(root.join(format!("{product_id}-{operation}.log")))
}

pub(crate) fn discover_linux_executable(root: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    collect_executables(root, root, 0, &mut candidates);
    candidates.sort_by_key(|path| executable_rank(root, path));
    candidates.into_iter().next()
}

fn collect_executables(root: &Path, directory: &Path, depth: usize, output: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_executables(root, &path, depth + 1, output);
        } else if is_linux_launch_candidate(root, &path) {
            output.push(path);
        }
    }
}

fn is_linux_launch_candidate(root: &Path, path: &Path) -> bool {
    if path == root || !path.is_file() {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.contains("uninstall") || name.contains("support") {
        return false;
    }
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sh"))
    {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    false
}

fn executable_rank(root: &Path, path: &Path) -> (u8, usize, String) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let rank = if name == "start.sh" {
        0
    } else if name == "game.sh" {
        1
    } else if name.ends_with(".sh") {
        2
    } else {
        3
    };
    let depth = path
        .strip_prefix(root)
        .map_or(usize::MAX, |relative| relative.components().count());
    (rank, depth, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{GalaxyDepotProvenance, InstallationSource},
        installation::marker::{InstallationMarker, InstalledCompatibility, InstalledComponent},
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn depot_uninstall_removes_payload_prefix_and_staging() {
        let library = std::env::temp_dir().join(format!(
            "gog-depot-uninstall-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = library.join("game");
        let prefix = crate::compatibility::prefix_path(&library, "game");
        fs::create_dir_all(&directory).unwrap();
        fs::create_dir_all(&prefix).unwrap();
        fs::write(directory.join("payload.bin"), b"game").unwrap();
        let marker = InstallationMarker {
            schema_version: 2,
            product_id: 7,
            slug: "game".into(),
            base: InstalledComponent {
                operating_system: Some("windows".into()),
                language: Some("en".into()),
                version: Some("1".into()),
                revision_id: None,
                installed_at: 1,
            },
            dlc: Vec::new(),
            compatibility: Some(InstalledCompatibility {
                backend: crate::compatibility::CompatibilityBackendKind::Umu,
                managed_by_ludomere: true,
                prefix_slug: "game".into(),
                profile: crate::compatibility::UmuProfile::fallback(),
            }),
            source: InstallationSource::GalaxyDepot,
            galaxy_depot: Some(GalaxyDepotProvenance {
                build_id: "build".into(),
                repository_id: "repository".into(),
                manifest_fingerprint: "sha256:test".into(),
                branch: None,
                language: Some("en".into()),
                architecture: Some("64".into()),
                depots: Vec::new(),
                dlc: Vec::new(),
            }),
            launch: None,
            dependencies: Vec::new(),
        };
        super::super::marker::write(&marker, &directory).unwrap();
        let journal =
            super::super::depot::operation_staging_path(&library, &directory, "game", "").unwrap();
        fs::create_dir_all(journal.parent().unwrap()).unwrap();
        fs::write(&journal, b"journal").unwrap();

        run_depot_uninstallation(7, &directory, &AtomicBool::new(false)).unwrap();

        assert!(!directory.exists());
        assert!(!prefix.exists());
        assert!(!journal.exists());
        fs::remove_dir_all(library).unwrap();
    }

    #[test]
    fn executable_discovery_prefers_start_script() {
        let root = std::env::temp_dir().join(format!(
            "gog-install-executable-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("game")).unwrap();
        fs::write(root.join("other.sh"), b"#!/bin/sh").unwrap();
        fs::write(root.join("game/start.sh"), b"#!/bin/sh").unwrap();
        assert_eq!(
            discover_linux_executable(&root),
            Some(root.join("game/start.sh"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_directory_cleanup_preserves_unknown_files() {
        let root =
            std::env::temp_dir().join(format!("gog-uninstall-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("empty/nested")).unwrap();
        fs::create_dir_all(root.join("saves")).unwrap();
        fs::write(root.join("saves/player.save"), b"save").unwrap();

        remove_empty_installation_directories(&root);

        assert!(!root.join("empty").exists());
        assert_eq!(fs::read(root.join("saves/player.save")).unwrap(), b"save");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recognizes_mojosetup_replace_prompt() {
        assert_eq!(
            installer_prompt_action(
                "File '/games/example/.mojosetup/desktop_icon' already exists! Replace?\r\n[y/n/Always/Never]: "
            ),
            InstallerPromptAction::ReplaceExisting
        );
    }

    #[test]
    fn rejects_unknown_interactive_prompt() {
        assert_eq!(
            installer_prompt_action("Choose an installation mode [1/2]: "),
            InstallerPromptAction::Unknown("Choose an installation mode [1/2]:".into())
        );
    }

    #[test]
    fn translates_stable_umu_runtime_and_prefix_messages() {
        assert_eq!(
            umu_widget_status(
                "[umu.umu_runtime:163] INFO: Downloading steamrt3 (3.0), please wait...",
                None,
            )
            .as_deref(),
            Some("Downloading UMU runtime")
        );
        assert_eq!(
            umu_widget_status(
                "[umu.umu_runtime:409] INFO: Verifying integrity of sniper_platform_3.0...",
                None,
            )
            .as_deref(),
            Some("Verifying UMU runtime")
        );
        assert_eq!(
            umu_widget_status(
                "Proton: Upgrading prefix from None to UMU-Proton-10.0-4",
                None
            )
            .as_deref(),
            Some("Creating Windows compatibility environment")
        );
    }

    #[test]
    fn translates_dependency_and_installer_launch_messages() {
        assert_eq!(
            umu_widget_status(
                "ProtonFixes[1] INFO: Installing winetricks d3dcompiler_43",
                None,
            )
            .as_deref(),
            Some("Installing Windows dependencies")
        );
        assert_eq!(
            umu_widget_status(
                "Proton: /downloads/setup_grim_dawn_1.3.0.6.exe",
                Some("Installing base game — 1 of 4"),
            )
            .as_deref(),
            Some("Installing base game — 1 of 4")
        );
        assert_eq!(umu_widget_status("fsync: up and running.", None), None);
    }
}
