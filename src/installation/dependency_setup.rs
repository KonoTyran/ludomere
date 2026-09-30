//! Per-prefix completion of the resolved GOG prerequisite plan.
use super::depot_actions::ActionContext;
use crate::{
    compatibility::{CompatibilityBackend, CompatibilityRunRequest},
    gog::dependencies::{Dependency, Method, Plan, PreparedDependency},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
struct SetupProcessGuard {
    boot: String,
    group: Option<u32>,
}

fn boot_identity() -> Result<String> {
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    ensure!(
        valid_boot(boot.trim()),
        "Could not establish setup process boot identity"
    );
    Ok(boot.trim().into())
}

fn valid_boot(boot: &str) -> bool {
    boot.len() == 36
        && boot.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub(crate) fn ensure_setup_quiescent(record: &crate::state::DepotOperationRecord) -> Result<()> {
    let plan: serde_json::Value = serde_json::from_str(&record.plan_json)?;
    let Some(value) = plan.get("setup_process_guard") else {
        return Ok(());
    };
    let guard: SetupProcessGuard = serde_json::from_value(value.clone())
        .context("Invalid setup process guard; recovery must not delete files")?;
    ensure!(
        valid_boot(&guard.boot)
            && guard
                .group
                .is_none_or(|group| group > 1 && group <= i32::MAX as u32),
        "Invalid setup process identity; recovery must not delete files"
    );
    if guard.boot != boot_identity()? {
        return Ok(());
    }
    let group = guard.group.context(
        "Required setup was interrupted before its process identity was saved. Reboot before resuming or removing this game; restarting Ludomere alone is not sufficient.",
    )?;
    ensure!(
        !crate::compatibility::CompatibilityProcess::group_is_running(group)?,
        "A required setup process may still be writing game files. Stop it or reboot before retrying this operation."
    );
    Ok(())
}

fn write_setup_guard(operation: &str, guard: Option<SetupProcessGuard>) -> Result<()> {
    let (path, mut record) = super::operation_journal::find_depot(operation)?;
    let mut plan: serde_json::Value = serde_json::from_str(&record.plan_json)?;
    let object = plan
        .as_object_mut()
        .context("Invalid saved operation plan")?;
    if let Some(guard) = guard {
        object.insert("setup_process_guard".into(), serde_json::to_value(guard)?);
    } else {
        object.remove("setup_process_guard");
    }
    record.plan_json = serde_json::to_string(&plan)?;
    super::operation_journal::write_depot(&path, &record)?;
    fs::File::open(path.parent().context("Setup journal has no directory")?)?.sync_all()?;
    Ok(())
}

pub(crate) fn run_tracked(
    operation: Option<&str>,
    stopped: &impl Fn() -> bool,
    name: &str,
    log: &Path,
    spawn: impl FnOnce() -> Result<crate::compatibility::CompatibilityProcess>,
) -> Result<()> {
    ensure!(!stopped(), "Required setup cancelled before starting");
    let boot = boot_identity()?;
    if let Some(operation) = operation {
        let (_, record) = super::operation_journal::find_depot(operation)?;
        ensure_setup_quiescent(&record)?;
        // Arm durably before spawn. An unknown same-boot group cannot be assumed safe
        // after a crash, even if the application itself has since restarted.
        write_setup_guard(
            operation,
            Some(SetupProcessGuard {
                boot: boot.clone(),
                group: None,
            }),
        )?;
    }
    if stopped() {
        if let Some(operation) = operation {
            write_setup_guard(operation, None)?;
        }
        anyhow::bail!("Required setup cancelled before starting");
    }
    let mut process = match spawn() {
        Ok(process) => process,
        Err(error) => {
            if let Some(operation) = operation {
                write_setup_guard(operation, None)?;
            }
            return Err(error);
        }
    };
    if let Some(operation) = operation
        && let Err(error) = write_setup_guard(
            operation,
            Some(SetupProcessGuard {
                boot,
                group: Some(process.group_id()),
            }),
        )
    {
        process.stop().context(
            "Setup identity could not be saved and its process could not be drained; reboot before recovery",
        )?;
        write_setup_guard(operation, None)?;
        return Err(error);
    }
    let result = wait_process(&mut process, stopped, name, log);
    // Any inspection error is uncertain, so attempt a controlled drain before removing
    // the durable guard. Failure leaves it armed for both in-process and restart recovery.
    if !matches!(process.group_running(), Ok(false)) {
        process
            .stop()
            .context("Required setup did not drain; recovery remains blocked")?;
    }
    if let Some(operation) = operation {
        write_setup_guard(operation, None)?;
    }
    result
}

// Preserve the reviewed graphics/audio and MSVC recipes, but use Proton's builtin XInput
// controller stack instead of Winetricks' native-only XInput registry overrides.
// A receipt describes this method, never claims the catalog's native installer ran.
pub(crate) fn override_verbs(id: &str, requested: &[String]) -> Option<Vec<String>> {
    let verbs: &[&str] = match id {
        "DirectX" => &["d3dcompiler_43", "d3dx9", "xact"],
        "MSVC2010" | "MSVC2010_x64" => &["vcrun2010"],
        "MSVC2012" | "MSVC2012_x64" => &["vcrun2012"],
        "MSVC2013" | "MSVC2013_x64" => &["vcrun2013"],
        "MSVC2015" | "MSVC2015_x64"
            if requested
                .iter()
                .any(|id| matches!(id.as_str(), "MSVC2019" | "MSVC2019_x64")) =>
        {
            &["vcrun2019"]
        }
        "MSVC2015" | "MSVC2015_x64" => &["vcrun2015"],
        "MSVC2019" | "MSVC2019_x64" => &["vcrun2019"],
        _ => return None,
    };
    Some(verbs.iter().map(|verb| (*verb).to_owned()).collect())
}

pub(crate) fn describe(plan: &Plan) -> String {
    let ids = plan
        .entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    if plan.entries.is_empty() {
        return "No additional GOG prerequisites are required.".into();
    }
    plan.entries
        .iter()
        .map(|entry| {
            let method = if let Some(verbs) = override_verbs(&entry.id, &ids) {
                format!("reviewed Wine recipe: {}", verbs.join(", "))
            } else {
                match &entry.method {
                    Method::Exe { .. } => "GOG executable".into(),
                    Method::Msi { .. } => "GOG Windows Installer package".into(),
                    Method::GameFiles => "required game-local files".into(),
                    Method::ScriptInterpreter { .. } => "GOG setup interpreter".into(),
                }
            };
            format!("{} ({}) — {method}", entry.name, entry.id)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn validate_required(plan: &Plan, required: &[String]) -> Result<()> {
    plan.validate()?;
    ensure!(
        plan.entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<BTreeSet<_>>()
            == required.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        "Saved prerequisite plan does not cover the complete selected build; prepare it again"
    );
    Ok(())
}

pub(crate) fn combined_manifest(
    payload: &crate::gog::depot_manifest::DepotManifest,
    plan: Option<&Plan>,
) -> Result<crate::gog::depot_manifest::DepotManifest> {
    use crate::gog::depot_manifest::DepotEntry;
    let mut combined = payload.clone();
    for dependency in plan
        .into_iter()
        .flat_map(|plan| &plan.entries)
        .filter(|entry| matches!(entry.method, Method::GameFiles))
    {
        let mut manifest = dependency.manifest()?;
        let offset = combined.small_files_containers.len();
        combined
            .small_files_containers
            .append(&mut manifest.small_files_containers);
        for mut entry in manifest.entries {
            let path = match &entry {
                DepotEntry::File(file) => &file.path,
                DepotEntry::Directory { path } | DepotEntry::Link { path, .. } => path,
            };
            if let Some(existing) = combined.entries.iter().find(|entry| match entry {
                DepotEntry::File(file) => file.path.eq_ignore_ascii_case(path),
                DepotEntry::Directory { path: other } | DepotEntry::Link { path: other, .. } => {
                    other.eq_ignore_ascii_case(path)
                }
            }) {
                ensure!(
                    super::manager::entries_equivalent(existing, &entry),
                    "Required game-local dependency conflicts with selected game files: {path}"
                );
                continue;
            }
            if let DepotEntry::File(file) = &mut entry
                && let Some(reference) = &mut file.small_file
            {
                reference.container_index = reference
                    .container_index
                    .checked_add(offset)
                    .context("small-files container index overflows")?;
            }
            combined.entries.push(entry);
        }
    }
    // Preparation must also prove the final ownership snapshot fits the local format.
    combined.snapshot_json()?;
    Ok(combined)
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PrefixIdentity {
    device: u64,
    inode: u64,
    drive_device: u64,
    drive_inode: u64,
}

fn prefix_identity(prefix: &Path) -> Result<PrefixIdentity> {
    for path in [prefix, &prefix.join("drive_c")] {
        ensure!(
            fs::symlink_metadata(path)?.is_dir(),
            "Dependency prefix is not a real directory"
        );
    }
    let root = fs::metadata(prefix)?;
    let drive = fs::metadata(prefix.join("drive_c"))?;
    for name in ["system.reg", "user.reg"] {
        let file = OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(prefix.join(name))
            .with_context(|| format!("Required prefix registry {name} is missing or unreadable; restore or recreate the prefix before retrying setup"))?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.nlink() == 1 && metadata.len() > 0,
            "Required prefix registry {name} is not a nonempty regular file; restore or recreate the prefix before retrying setup"
        );
    }
    Ok(PrefixIdentity {
        device: root.dev(),
        inode: root.ino(),
        drive_device: drive.dev(),
        drive_inode: drive.ino(),
    })
}

#[derive(Serialize, Deserialize)]
struct Receipts {
    version: u32,
    prefix: PrefixIdentity,
    complete: BTreeSet<String>,
}

/// Provenance for a launch-only correction of our former native-XInput recipe.
/// An unrelated prefix or a Winetricks log alone is not evidence of this recipe.
pub(crate) fn legacy_xinput_recipe(prefix: &Path) -> Result<bool> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(prefix.join(".ludomere-gog-dependencies.json"))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o022 == 0
            && metadata.len() <= 1024 * 1024,
        "Unsafe dependency completion record"
    );
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "Dependency completion record exceeds the safety limit"
    );
    let receipts: Receipts = serde_json::from_slice(&bytes)?;
    ensure!(
        receipts.version == 1,
        "Unsupported dependency completion record version"
    );
    if receipts.prefix != prefix_identity(prefix)? {
        return Ok(false);
    }
    Ok(receipts.complete.iter().any(|key| {
        key.strip_suffix(":winetricks-v1:d3dcompiler_43,d3dx9,xact,xinput")
            .is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    }))
}

fn complete_step(
    prefix: &Path,
    key: String,
    stopped: &impl Fn() -> bool,
    apply: impl FnOnce() -> Result<()>,
) -> Result<()> {
    ensure!(!stopped(), "Dependency setup cancelled");
    let identity = prefix_identity(prefix)?;
    let path = prefix.join(".ludomere-gog-dependencies.json");
    let mut receipts = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => {
            let metadata = file.metadata()?;
            ensure!(
                metadata.is_file() && metadata.nlink() == 1 && metadata.len() <= 1024 * 1024,
                "Unsafe dependency completion record"
            );
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            let receipts: Receipts = serde_json::from_slice(&bytes).context(
                "Reading dependency completion record; repair the record before retrying",
            )?;
            ensure!(
                receipts.version == 1,
                "Unsupported dependency completion record version"
            );
            if receipts.prefix == identity {
                receipts
            } else {
                Receipts {
                    version: 1,
                    prefix: identity,
                    complete: BTreeSet::new(),
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Receipts {
            version: 1,
            prefix: identity,
            complete: BTreeSet::new(),
        },
        Err(error) => return Err(error.into()),
    };
    if receipts.complete.contains(&key) {
        return Ok(());
    }
    apply()?;
    ensure!(
        !stopped() && prefix_identity(prefix)? == receipts.prefix,
        "Prefix/account changed or setup cancelled; completion was not recorded"
    );
    receipts.complete.insert(key);
    let mut temporary = tempfile::NamedTempFile::new_in(prefix)?;
    temporary.write_all(&serde_json::to_vec(&receipts)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

pub(crate) fn apply(
    backend: &crate::compatibility::UmuBackend,
    plan: &Plan,
    prepared: &[PreparedDependency],
    context: &ActionContext,
    stopped: impl Fn() -> bool,
    mut progress: impl FnMut(&str),
) -> Result<()> {
    let ids = plan
        .entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    for entry in &plan.entries {
        ensure!(!stopped(), "Dependency setup cancelled");
        if matches!(
            entry.method,
            Method::GameFiles | Method::ScriptInterpreter { .. }
        ) {
            continue;
        }
        progress(&entry.name);
        let verbs = override_verbs(&entry.id, &ids);
        let method = verbs
            .as_ref()
            .map(|verbs| format!("winetricks-v1:{}", verbs.join(",")))
            .unwrap_or_else(|| "gog-native-v1".into());
        complete_step(
            &context.prefix,
            format!("{}:{method}", entry.identity()),
            &stopped,
            || {
                crate::compatibility::append_step_log(
                    &context.log_path,
                    &format!(
                        "Required dependency {} ({}) via {method}",
                        entry.name, entry.id
                    ),
                )?;
                if let Some(verbs) = verbs {
                    // The selected compatibility method explicitly accepts an exact completed verb
                    // in this prefix, including when vendor metadata changed. It is not native proof.
                    let pending = super::manager::pending_dependency_verbs(&context.prefix, verbs)?;
                    if pending.is_empty() {
                        return Ok(());
                    }
                    run_tracked(
                        context.operation_id.as_deref(),
                        &stopped,
                        &entry.name,
                        &context.log_path,
                        || {
                            Ok(backend.run_winetricks(
                                &context.prefix,
                                &context.profile,
                                &pending,
                                &context.app,
                                &context.log_path,
                            )?)
                        },
                    )
                } else {
                    let root = verified_root(entry, prepared, &stopped)?;
                    ensure!(!stopped(), "Dependency setup cancelled");
                    let command = command(entry, &root, context)?;
                    run_tracked(
                        context.operation_id.as_deref(),
                        &stopped,
                        &entry.name,
                        &context.log_path,
                        || Ok(backend.run_executable(command)?),
                    )
                }
            },
        )?;
    }
    Ok(())
}

fn verified_root(
    entry: &Dependency,
    prepared: &[PreparedDependency],
    stopped: &impl Fn() -> bool,
) -> Result<PathBuf> {
    ensure!(
        prepared
            .iter()
            .any(|item| item.dependency.identity() == entry.identity()),
        "Dependency was not acquired as part of this plan"
    );
    crate::gog::dependencies::verify_cached(entry, stopped)?
        .context("Required dependency cache changed; retry to restore verified files")
}

pub(crate) fn interpret(
    backend: &dyn CompatibilityBackend,
    plan: &Plan,
    prepared: &[PreparedDependency],
    context: &ActionContext,
    setup: &super::depot_actions::GalaxySetup,
    operation_id: &str,
    stopped: impl Fn() -> bool,
) -> Result<()> {
    let entry = plan
        .entries
        .iter()
        .find(|entry| matches!(entry.method, Method::ScriptInterpreter { .. }))
        .context("GOG setup requires the verified ISI dependency; retry preparation")?;
    let root = verified_root(entry, prepared, &stopped)?;
    let mut invocation = command(entry, &root, context)?;
    ensure!(
        invocation.arguments.is_empty(),
        "Unsupported GOG interpreter argument contract"
    );
    invocation.arguments = super::depot_actions::galaxy_setup_arguments(context, setup, "")?;
    invocation.arguments.push(format!(
        "/supportDir={}\\.ludomere-support.part",
        context.windows_app
    ));
    invocation.working_directory = Some(context.app.clone());
    complete_step(
        &context.prefix,
        format!(
            "{}:isi-v1:{operation_id}:{}:{}:{}:{}",
            entry.identity(),
            context.product_id,
            setup.build_id,
            setup.language,
            setup.version
        ),
        &stopped,
        || {
            run_tracked(
                context.operation_id.as_deref(),
                &stopped,
                &entry.name,
                &context.log_path,
                || Ok(backend.run_executable(invocation)?),
            )
        },
    )
}

fn command(
    entry: &Dependency,
    root: &Path,
    context: &ActionContext,
) -> Result<CompatibilityRunRequest> {
    let (executable, arguments) = match &entry.method {
        Method::Exe { path, args } | Method::ScriptInterpreter { path, args } => {
            (root.join(path), args.clone())
        }
        Method::Msi { path, args } => {
            ensure!(
                fs::read_link(context.prefix.join("dosdevices/z:"))? == Path::new("/"),
                "This prefix has no verified Z: mapping to the host root; cannot safely open the required MSI package"
            );
            let package = root.join(path);
            ensure!(package.is_absolute(), "MSI package path must be absolute");
            let package = package.to_str().context("MSI path is not valid UTF-8")?;
            ensure!(
                !package.contains('\\') && !package.chars().any(char::is_control),
                "MSI path cannot be represented safely in Wine"
            );
            let mut arguments = vec!["/i".into(), format!("Z:{}", package.replace('/', "\\"))];
            arguments.extend(args.clone());
            (
                context.prefix.join("drive_c/windows/system32/msiexec.exe"),
                arguments,
            )
        }
        Method::GameFiles => anyhow::bail!("Game-local dependency has no executable"),
    };
    Ok(CompatibilityRunRequest {
        prefix: context.prefix.clone(),
        profile: context.profile.clone(),
        executable,
        arguments,
        working_directory: Some(root.to_owned()),
        log_path: context.log_path.clone(),
        background: true,
    })
}

pub(crate) fn wait_process(
    process: &mut crate::compatibility::CompatibilityProcess,
    stopped: &impl Fn() -> bool,
    name: &str,
    log: &Path,
) -> Result<()> {
    loop {
        if stopped() {
            process
                .stop()
                .context("Stopping required dependency process group")?;
            process.wait()?;
            return Err(crate::download::depot::DepotCancelled.into());
        }
        if let Some(status) = process.try_wait()? {
            if !status.success() {
                if process.group_running()? {
                    process
                        .stop()
                        .context("Draining failed required setup process group")?;
                }
                let detail = super::runtime_logs::installation_tail(log)
                    .unwrap_or_else(|_| "Installation log could not be read".into());
                anyhow::bail!(
                    "Required dependency {name} failed ({status}). Log: {}\n{detail}",
                    log.display()
                );
            }
            if !process.group_running()? {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GuardJournal {
        root: tempfile::TempDir,
        path: PathBuf,
        previous_config: Option<Vec<u8>>,
    }

    impl GuardJournal {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let previous_config = fs::read(crate::config::Config::path()).ok();
            let config = crate::config::Config {
                game_libraries: vec![crate::config::GameLibrary {
                    id: "setup-guard-fixture".into(),
                    name: "Inert fixture".into(),
                    path: root.path().to_owned(),
                    default: true,
                }],
                ..crate::config::Config::default()
            };
            fs::create_dir_all(crate::config::Config::path().parent().unwrap()).unwrap();
            config.save().unwrap();
            let path = super::super::operation_journal::path(root.path(), "fixture").unwrap();
            Self {
                root,
                path,
                previous_config,
            }
        }

        fn write(&self, guard: Option<serde_json::Value>) -> crate::state::DepotOperationRecord {
            let mut plan = serde_json::json!({});
            if let Some(guard) = guard {
                plan["setup_process_guard"] = guard;
            }
            let record = crate::state::DepotOperationRecord {
                operation_id: "guard-fixture".into(),
                product_id: 7357136,
                build_id: "fixture".into(),
                branch: None,
                kind: "install".into(),
                state: "failed".into(),
                destination: self.root.path().join("fixture"),
                staging_path: self.path.with_extension("part"),
                plan_json: plan.to_string(),
                bytes_completed: 0,
                total_bytes: None,
                error: Some("inert failure".into()),
                created_at: 1,
                updated_at: 1,
                completed_at: None,
            };
            super::super::operation_journal::write_depot(&self.path, &record).unwrap();
            self.read()
        }

        fn read(&self) -> crate::state::DepotOperationRecord {
            let super::super::operation_journal::OperationJournal::Depot { record, .. } =
                super::super::operation_journal::read(&self.path).unwrap()
            else {
                panic!("wrong journal kind")
            };
            record
        }
    }

    impl Drop for GuardJournal {
        fn drop(&mut self) {
            if let Some(bytes) = &self.previous_config {
                fs::write(crate::config::Config::path(), bytes).unwrap();
            } else {
                let _ = fs::remove_file(crate::config::Config::path());
            }
        }
    }

    #[test]
    fn persisted_setup_guards_reject_unknown_malformed_and_live_groups() {
        let journal = GuardJournal::new();
        let boot = boot_identity().unwrap();
        let unknown = journal.write(Some(serde_json::json!({"boot":boot,"group":null})));
        assert!(
            ensure_setup_quiescent(&unknown)
                .unwrap_err()
                .to_string()
                .contains("Reboot")
        );
        for guard in [
            serde_json::json!({"boot":"invalid","group":null}),
            serde_json::json!({"boot":boot,"group":0}),
            serde_json::json!({"boot":boot,"group":1}),
            serde_json::json!({"boot":boot,"group":u32::MAX}),
            serde_json::json!({"boot":boot,"group":"unknown"}),
            serde_json::Value::Null,
        ] {
            assert!(ensure_setup_quiescent(&journal.write(Some(guard))).is_err());
        }
        let other_boot = if boot == "00000000-0000-0000-0000-000000000000" {
            "11111111-1111-1111-1111-111111111111"
        } else {
            "00000000-0000-0000-0000-000000000000"
        };
        assert!(
            ensure_setup_quiescent(
                &journal.write(Some(serde_json::json!({"boot":other_boot,"group":null})))
            )
            .is_ok()
        );
        assert!(ensure_setup_quiescent(&journal.write(None)).is_ok());

        let mut command = std::process::Command::new("python3");
        command.args(["-I", "-c", "import time; time.sleep(15)"]);
        let mut process = crate::compatibility::CompatibilityProcess::spawn(
            command,
            &journal.root.path().join("process.log"),
        )
        .unwrap();
        let record = journal.write(Some(
            serde_json::json!({"boot":boot,"group":process.group_id()}),
        ));
        let blocked = ensure_setup_quiescent(&record);
        let stopped = process.stop();
        assert!(blocked.is_err());
        stopped.unwrap();
        assert!(ensure_setup_quiescent(&journal.read()).is_ok());
    }

    #[test]
    fn tracked_setup_arms_journal_before_spawn_and_blocks_uncertain_resume() {
        let journal = GuardJournal::new();
        journal.write(None);
        let called = std::cell::Cell::new(false);
        let result = run_tracked(
            Some("guard-fixture"),
            &|| false,
            "Inert setup",
            &journal.root.path().join("log"),
            || {
                called.set(true);
                let record = journal.read();
                let plan: serde_json::Value = serde_json::from_str(&record.plan_json).unwrap();
                assert_eq!(
                    plan["setup_process_guard"]["boot"],
                    boot_identity().unwrap()
                );
                assert!(plan["setup_process_guard"]["group"].is_null());
                assert!(ensure_setup_quiescent(&record).is_err());
                anyhow::bail!("synthetic spawn failure; no process created")
            },
        );
        assert!(called.get());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("synthetic spawn failure")
        );
        let cleared: serde_json::Value = serde_json::from_str(&journal.read().plan_json).unwrap();
        assert!(cleared.get("setup_process_guard").is_none());

        journal.write(Some(
            serde_json::json!({"boot":boot_identity().unwrap(),"group":null}),
        ));
        called.set(false);
        let blocked = run_tracked(
            Some("guard-fixture"),
            &|| false,
            "Inert setup",
            &journal.root.path().join("log"),
            || {
                called.set(true);
                anyhow::bail!("must not spawn")
            },
        );
        assert!(blocked.unwrap_err().to_string().contains("Reboot"));
        assert!(!called.get());
        assert!(ensure_setup_quiescent(&journal.read()).is_err());
    }

    #[test]
    fn interpreter_resume_skips_only_the_same_operation_and_reinstall_runs_again() {
        use crate::compatibility::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct InertBackend(AtomicUsize);
        impl CompatibilityBackend for InertBackend {
            fn status(&self) -> crate::compatibility::Result<CompatibilityBackendStatus> {
                unreachable!()
            }
            fn initialize_prefix(
                &self,
                _: InitializePrefixRequest,
            ) -> crate::compatibility::Result<CompatibilityPrefix> {
                unreachable!()
            }
            fn stop(&self, process: &mut CompatibilityProcess) -> crate::compatibility::Result<()> {
                process.stop()
            }
            fn run_executable(
                &self,
                request: CompatibilityRunRequest,
            ) -> crate::compatibility::Result<CompatibilityProcess> {
                assert!(
                    request
                        .arguments
                        .iter()
                        .any(|argument| argument == "/ProductId=7")
                );
                assert!(
                    request
                        .arguments
                        .iter()
                        .any(|argument| argument == "/supportDir=L:\\game\\.ludomere-support.part")
                );
                self.0.fetch_add(1, Ordering::Relaxed);
                // Only the owned inert stand-in is executed, never the catalog-shaped file.
                let mut command = std::process::Command::new("/usr/bin/python3");
                command.args(["-c", "pass"]);
                CompatibilityProcess::spawn(command, &request.log_path)
            }
        }
        let bytes = b"inert interpreter fixture";
        let raw = serde_json::to_vec(&serde_json::json!({"version":2,"depot":{"items":[{
        "type":"DepotFile","path":"__redist/ISI/isi.exe","chunks":[{
            "compressedMd5":format!("{:x}",md5::compute(bytes)),"compressedSize":bytes.len(),
            "md5":format!("{:x}",md5::compute(bytes)),"size":bytes.len()
        }]}]}}))
        .unwrap();
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let manifest_bytes = encoder.finish().unwrap();
        let entry = Dependency {
            id: "ISI".into(),
            name: "Inert interpreter".into(),
            manifest_id: format!("{:x}", md5::compute(&manifest_bytes)),
            manifest_bytes,
            method: Method::ScriptInterpreter {
                path: "__redist/ISI/isi.exe".into(),
                args: Vec::new(),
            },
        };
        let root = crate::identity::data_root()
            .join("gog-dependencies")
            .join(entry.identity());
        fs::create_dir_all(root.join("__redist/ISI")).unwrap();
        fs::write(root.join("__redist/ISI/isi.exe"), bytes).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let prefix = temporary.path().join("prefix");
        fs::create_dir_all(prefix.join("drive_c")).unwrap();
        for registry in ["system.reg", "user.reg"] {
            fs::write(prefix.join(registry), "WINE REGISTRY Version 2\n").unwrap();
        }
        let context = ActionContext {
            operation_id: None,
            product_id: 7,
            app: temporary.path().join("game"),
            support: temporary.path().join("support"),
            prefix,
            windows_app: "L:\\game".into(),
            profile: UmuProfile::fallback(),
            log_path: temporary.path().join("log"),
            galaxy_setup: None,
        };
        let setup = super::super::depot_actions::GalaxySetup {
            executable: String::new(),
            arguments: String::new(),
            language: "en-US".into(),
            build_id: "build".into(),
            version: "1".into(),
        };
        let plan = Plan {
            version: 1,
            catalog_build: "1".into(),
            entries: vec![entry.clone()],
        };
        let prepared = vec![PreparedDependency {
            dependency: entry,
            root: root.clone(),
        }];
        let backend = InertBackend(AtomicUsize::new(0));
        interpret(
            &backend,
            &plan,
            &prepared,
            &context,
            &setup,
            "install-1",
            || false,
        )
        .unwrap();
        interpret(
            &backend,
            &plan,
            &prepared,
            &context,
            &setup,
            "install-1",
            || false,
        )
        .unwrap();
        assert_eq!(backend.0.load(Ordering::Relaxed), 1);
        interpret(
            &backend,
            &plan,
            &prepared,
            &context,
            &setup,
            "install-2",
            || false,
        )
        .unwrap();
        assert_eq!(backend.0.load(Ordering::Relaxed), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_frozen_plan_and_msi_path_contract_are_checked_before_execution() {
        let empty = Plan {
            version: 1,
            catalog_build: String::new(),
            entries: Vec::new(),
        };
        assert!(validate_required(&empty, &["openAL".into()]).is_err());
        assert!(validate_required(&empty, &[]).is_ok());
        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("prefix");
        fs::create_dir_all(prefix.join("dosdevices")).unwrap();
        let context = ActionContext {
            operation_id: None,
            product_id: 7,
            app: root.path().join("game"),
            support: root.path().join("support"),
            prefix: prefix.clone(),
            windows_app: "L:\\game".into(),
            profile: crate::compatibility::UmuProfile::fallback(),
            log_path: root.path().join("log"),
            galaxy_setup: None,
        };
        let mut dependency = Dependency {
            id: "XNA".into(),
            name: "XNA".into(),
            manifest_id: String::new(),
            manifest_bytes: Vec::new(),
            method: Method::Msi {
                path: "__redist/XNA/package with spaces.msi".into(),
                args: vec!["/qn".into(), "/norestart".into()],
            },
        };
        let cache = root.path().join("cache with spaces");
        assert!(command(&dependency, &cache, &context).is_err());
        std::os::unix::fs::symlink("/", prefix.join("dosdevices/z:")).unwrap();
        let invocation = command(&dependency, &cache, &context).unwrap();
        assert_eq!(
            invocation.arguments,
            vec![
                "/i".to_owned(),
                format!(
                    "Z:{}\\__redist\\XNA\\package with spaces.msi",
                    cache.to_str().unwrap().replace('/', "\\")
                ),
                "/qn".into(),
                "/norestart".into()
            ]
        );
        dependency.method = Method::Exe {
            path: "__redist/openAL/oalinst.exe".into(),
            args: vec!["/S".into()],
        };
        assert_eq!(
            command(&dependency, &cache, &context).unwrap().arguments,
            ["/S"]
        );
        assert!(override_verbs("openAL", &[]).is_none());
        assert_eq!(
            override_verbs("MSVC2015", &["MSVC2019_x64".into()]).unwrap(),
            ["vcrun2019"]
        );
    }

    #[test]
    fn game_local_manifest_tracks_required_files_and_rejects_payload_collisions() {
        let bytes = include_bytes!("../../tests/fixtures/gog-dependencies/DOSBox074.zlib").to_vec();
        let dependency = Dependency {
            id: "DOSBox074".into(),
            name: "DOSBox".into(),
            manifest_id: format!("{:x}", md5::compute(&bytes)),
            manifest_bytes: bytes,
            method: Method::GameFiles,
        };
        let plan = Plan {
            version: 1,
            catalog_build: "1".into(),
            entries: vec![dependency.clone()],
        };
        let payload = crate::gog::depot_manifest::DepotManifest {
            generation: 2,
            entries: Vec::new(),
            small_files_containers: Vec::new(),
        };
        let combined = combined_manifest(&payload, Some(&plan)).unwrap();
        assert!(!combined.entries.is_empty());
        assert_eq!(
            combined.identity(),
            dependency.manifest().unwrap().identity()
        );
        let mut conflicting = combined.clone();
        let crate::gog::depot_manifest::DepotEntry::File(file) = conflicting
            .entries
            .iter_mut()
            .find(|entry| matches!(entry, crate::gog::depot_manifest::DepotEntry::File(_)))
            .unwrap()
        else {
            unreachable!()
        };
        file.executable = !file.executable;
        assert!(combined_manifest(&conflicting, Some(&plan)).is_err());
    }
    #[test]
    fn xinput_correction_requires_our_exact_receipt_and_current_prefix() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("prefix");
        fs::create_dir_all(prefix.join("drive_c")).unwrap();
        for registry in ["system.reg", "user.reg"] {
            fs::write(prefix.join(registry), "WINE REGISTRY Version 2\n").unwrap();
        }
        let path = prefix.join(".ludomere-gog-dependencies.json");
        assert!(!legacy_xinput_recipe(&prefix).unwrap());
        let recipe = format!(
            "{}:winetricks-v1:d3dcompiler_43,d3dx9,xact,xinput",
            "a".repeat(64)
        );
        complete_step(&prefix, recipe, &|| false, || Ok(())).unwrap();
        assert!(legacy_xinput_recipe(&prefix).unwrap());
        let original = fs::read(&path).unwrap();
        let registry = fs::read(prefix.join("user.reg")).unwrap();
        assert!(legacy_xinput_recipe(&prefix).unwrap());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(prefix.join("user.reg")).unwrap(), registry);

        for key in [
            "xinput",
            "not-a-digest:winetricks-v1:d3dcompiler_43,d3dx9,xact,xinput",
            "abc:winetricks-v1:xinput",
        ] {
            let mut receipt: Receipts = serde_json::from_slice(&original).unwrap();
            receipt.complete = BTreeSet::from([key.to_owned()]);
            fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
            assert!(!legacy_xinput_recipe(&prefix).unwrap());
        }
        fs::write(&path, b"malformed").unwrap();
        assert!(legacy_xinput_recipe(&prefix).is_err());
        fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(legacy_xinput_recipe(&prefix).is_err());
        fs::write(&path, &original).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(legacy_xinput_recipe(&prefix).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let saved = prefix.join("saved-receipt");
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        assert!(legacy_xinput_recipe(&prefix).is_err());
        fs::remove_file(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
        fs::rename(prefix.join("drive_c"), prefix.join("old-drive")).unwrap();
        fs::create_dir(prefix.join("drive_c")).unwrap();
        assert!(!legacy_xinput_recipe(&prefix).unwrap());

        let current = override_verbs("DirectX", &["DirectX".into()]).unwrap();
        assert_eq!(current, ["d3dcompiler_43", "d3dx9", "xact"]);
        complete_step(
            &prefix,
            format!("{}:winetricks-v1:{}", "a".repeat(64), current.join(",")),
            &|| false,
            || Ok(()),
        )
        .unwrap();
        assert!(!legacy_xinput_recipe(&prefix).unwrap());
    }

    #[test]
    fn partial_retry_revision_and_recreated_prefix_never_reuse_wrong_success() {
        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("prefix");
        fs::create_dir_all(prefix.join("drive_c")).unwrap();
        fs::write(prefix.join("system.reg"), "WINE REGISTRY Version 2\n").unwrap();
        fs::write(prefix.join("user.reg"), "WINE REGISTRY Version 2\n").unwrap();
        let count = std::cell::Cell::new(0);
        let run = || {
            count.set(count.get() + 1);
            Ok(())
        };
        complete_step(&prefix, "openAL-revision1-native".into(), &|| false, run).unwrap();
        assert!(
            complete_step(
                &prefix,
                "vcredist-revision1-native".into(),
                &|| false,
                || anyhow::bail!("synthetic failure")
            )
            .is_err()
        );
        complete_step(&prefix, "openAL-revision1-native".into(), &|| false, run).unwrap();
        assert_eq!(count.get(), 1);
        complete_step(&prefix, "vcredist-revision1-native".into(), &|| false, run).unwrap();
        complete_step(&prefix, "openAL-revision2-native".into(), &|| false, run).unwrap();
        assert_eq!(count.get(), 3);
        fs::rename(prefix.join("drive_c"), prefix.join("old-drive")).unwrap();
        fs::create_dir(prefix.join("drive_c")).unwrap();
        complete_step(&prefix, "openAL-revision2-native".into(), &|| false, run).unwrap();
        assert_eq!(count.get(), 4);
        fs::remove_file(prefix.join("system.reg")).unwrap();
        assert!(complete_step(&prefix, "openAL-revision2-native".into(), &|| false, run).is_err());
        assert_eq!(count.get(), 4);
        assert!(complete_step(&prefix, "cancelled".into(), &|| true, run).is_err());
        assert_eq!(count.get(), 4);
    }
}
