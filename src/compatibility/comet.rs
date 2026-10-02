use super::CompatibilityBackend;
use crate::{auth, state::StateStore};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

const GALAXY_CLIENT_ID: &str = "46899977096215655";

#[path = "comet_update.rs"]
mod update;
pub use update::{CometUpdate, install_update, update_candidate};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Build {
    pub version: String,
    pub files: std::collections::BTreeMap<String, String>,
}

pub struct CometSession {
    child: Child,
    credential_root: PathBuf,
    _activity: crate::profile_reset::ActivityGuard,
}

impl Drop for CometSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.credential_root);
    }
}

pub fn start(
    backend: &super::UmuBackend,
    prefix: &Path,
    profile: &super::UmuProfile,
    log_path: &Path,
) -> Result<Option<CometSession>> {
    let session = auth::session();
    let Some(token) = auth::load_saved_token()? else {
        return Ok(None);
    };
    let Some(account) = StateStore::open()?.cached_profile()? else {
        return Ok(None);
    };
    let runtime = ensure_runtime()?;
    install_dummy_service(backend, prefix, profile, log_path, &runtime.service)?;
    start_session(&runtime.comet, log_path, &token, &account.username, session)
}

pub fn start_native(log_path: &Path) -> Result<Option<CometSession>> {
    let session = auth::session();
    let Some(token) = auth::load_saved_token()? else {
        return Ok(None);
    };
    let Some(account) = StateStore::open()?.cached_profile()? else {
        return Ok(None);
    };
    let runtime = ensure_runtime()?;
    start_session(&runtime.comet, log_path, &token, &account.username, session)
}

fn start_session(
    comet: &Path,
    _log_path: &Path,
    token: &auth::Token,
    username: &str,
    session: u64,
) -> Result<Option<CometSession>> {
    ensure!(
        auth::session_is_current(session),
        "GOG session changed; online services were not started"
    );
    let activity = crate::profile_reset::begin_activity("GOG online service")?;
    let credential_root = write_credentials(token)?;
    let child = match session_command(comet, &credential_root, username).spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = fs::remove_dir_all(&credential_root);
            return Err(error).context("could not start Comet");
        }
    };
    let service = CometSession {
        child,
        credential_root,
        _activity: activity,
    };
    ensure!(
        auth::session_is_current(session),
        "GOG session changed; online services were stopped"
    );
    Ok(Some(service))
}

fn session_command(comet: &Path, credential_root: &Path, username: &str) -> Command {
    let mut command = Command::new(comet);
    command
        .env("XDG_CONFIG_PATH", credential_root)
        .env(
            "XDG_DATA_HOME",
            crate::identity::data_root().join("comet/state"),
        )
        .env("XDG_CONFIG_HOME", credential_root)
        .env("COMET_IDLE_WAIT", "5")
        // Upstream logs credentials even at info/warn. Suppress both streams too.
        .env("COMET_LOG", "off")
        .args(["--from-heroic", "--username", username, "--quit"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

struct Runtime {
    comet: PathBuf,
    service: PathBuf,
}

fn ensure_runtime() -> Result<Runtime> {
    let root = effective_directory()?;
    verify_build(&root)?;
    Ok(Runtime {
        comet: root.join("comet"),
        service: root.join("GalaxyCommunication.exe"),
    })
}

fn bundled_directory() -> PathBuf {
    helper_directory(
        std::env::var_os("LUDOMERE_COMET_DIR").map(PathBuf::from),
        PathBuf::from("/usr/lib/ludomere/comet"),
        std::env::current_exe().ok(),
        cfg!(debug_assertions),
    )
}

fn helper_directory(
    override_path: Option<PathBuf>,
    packaged: PathBuf,
    executable: Option<PathBuf>,
    development: bool,
) -> PathBuf {
    if let Some(path) = override_path.filter(|path| path.is_absolute()) {
        return path;
    }
    match fs::symlink_metadata(&packaged) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return packaged,
    }
    if development
        && let Some(staged) = executable
            .as_deref()
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
            .map(|root| root.join("helpers/comet"))
        && fs::symlink_metadata(&staged).is_ok()
    {
        return staged;
    }
    packaged
}

fn verify_build(root: &Path) -> Result<Build> {
    ensure!(
        fs::symlink_metadata(root)?.is_dir(),
        "Comet directory must be a real directory"
    );
    let build: Build =
        serde_json::from_slice(&update::local_bytes(&root.join("build.json"), 8192)?)?;
    update::validate_version(&build.version)?;
    ensure!(build.files.len() == 2, "Invalid Comet helper inventory");
    for name in ["comet", "GalaxyCommunication.exe"] {
        let expected = build
            .files
            .get(name)
            .context("Missing Comet helper checksum")?;
        ensure!(
            update::is_digest(expected)
                && format!(
                    "{:x}",
                    Sha256::digest(update::local_bytes(&root.join(name), 64 * 1024 * 1024)?)
                ) == *expected,
            "Comet is missing or damaged; reinstall Ludomere or prepare development helpers"
        );
    }
    ensure!(
        fs::metadata(root.join("comet"))?.permissions().mode() & 0o111 != 0,
        "Comet is not executable"
    );
    Ok(build)
}

pub fn installed_version() -> Result<String> {
    let root = effective_directory()?;
    Ok(verify_build(&root)?.version)
}

fn effective_directory() -> Result<PathBuf> {
    select_directory(&bundled_directory(), update::active_directory()?)
}

fn select_directory(bundled: &Path, active: Option<PathBuf>) -> Result<PathBuf> {
    let Some(active) = active else {
        return Ok(bundled.to_owned());
    };
    let active_build = verify_build(&active)?;
    if bundled.exists() {
        let packaged = verify_build(bundled)?;
        if super::components::newer_version(&packaged.version, &active_build.version)? {
            return Ok(bundled.to_owned());
        }
    }
    Ok(active)
}

/// Upstream appends `comet` to XDG_DATA_HOME; metadata checks use this same root.
pub fn data_directory() -> PathBuf {
    crate::identity::data_root().join("comet/state/comet")
}

fn digest(path: &Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

fn write_credentials(token: &auth::Token) -> Result<PathBuf> {
    let root = crate::identity::cache_root().join(format!(
        "comet-session-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let directory = root.join("heroic/gog_store");
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let value = serde_json::json!({
        (GALAXY_CLIENT_ID): {
            "access_token": token.access_token,
            "refresh_token": token.refresh_token,
            "user_id": token.user_id,
        }
    });
    let path = directory.join("auth.json");
    fs::write(&path, serde_json::to_vec(&value)?)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(root)
}

fn install_dummy_service(
    backend: &super::UmuBackend,
    prefix: &Path,
    profile: &super::UmuProfile,
    log_path: &Path,
    source: &Path,
) -> Result<()> {
    let destination =
        prefix.join("drive_c/ProgramData/GOG.com/Galaxy/redists/GalaxyCommunication.exe");
    let registration = destination.with_extension("ludomere-registered");
    if destination.is_file() && digest(&destination)? == digest(source)? && registration.is_file() {
        return Ok(());
    }
    fs::create_dir_all(destination.parent().unwrap())?;
    fs::copy(source, &destination)?;
    // The registered service points at this stable path. Replacing its bytes does
    // not require creating the existing Windows service again.
    if registration.is_file() {
        return Ok(());
    }
    let sc = prefix.join("drive_c/windows/system32/sc.exe");
    if !sc.is_file() {
        anyhow::bail!("the compatibility prefix does not contain sc.exe")
    }
    let mut process = backend.run_executable(super::CompatibilityRunRequest {
        prefix: prefix.to_owned(),
        profile: profile.clone(),
        executable: sc,
        arguments: vec![
            "create".into(),
            "GalaxyCommunication".into(),
            "binpath=C:\\ProgramData\\GOG.com\\Galaxy\\redists\\GalaxyCommunication.exe".into(),
        ],
        working_directory: destination.parent().map(PathBuf::from),
        log_path: log_path.to_owned(),
        background: true,
    })?;
    let status = process.wait()?;
    if !status.success() {
        anyhow::bail!("could not register the Galaxy Communication service: {status}")
    }
    File::create(registration)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_helper_discovery_preserves_override_package_and_validation() {
        let root = tempfile::tempdir().unwrap();
        let packaged = root.path().join("packaged");
        let executable = root.path().join("target/debug/ludomere");
        let staged = root.path().join("target/helpers/comet");
        fs::create_dir_all(&staged).unwrap();
        assert_eq!(
            helper_directory(None, packaged.clone(), Some(executable.clone()), true),
            staged
        );
        assert!(verify_build(&staged).is_err());
        assert_eq!(
            helper_directory(None, packaged.clone(), Some(executable.clone()), false),
            packaged
        );
        let custom = root.path().join("custom");
        assert_eq!(
            helper_directory(
                Some(custom.clone()),
                packaged.clone(),
                Some(executable.clone()),
                true
            ),
            custom
        );
        fs::create_dir(&packaged).unwrap();
        assert_eq!(
            helper_directory(None, packaged.clone(), Some(executable), true),
            packaged
        );
        assert!(verify_build(&packaged).is_err());
    }

    #[test]
    fn comet_diagnostics_cannot_enter_the_game_log() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("comet");
        fs::write(&executable, "#!/bin/sh\ntest \"$COMET_LOG\" = off || exit 1\nprintf 'synthetic secret stdout'\nprintf 'synthetic secret stderr' >&2\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = session_command(&executable, root.path(), "account");
        assert!(
            command.get_envs().any(
                |(key, value)| key == "COMET_LOG" && value == Some(std::ffi::OsStr::new("off"))
            )
        );
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn changed_service_keeps_existing_registration_without_running_umu() {
        let root = tempfile::tempdir().unwrap();
        let destination = root
            .path()
            .join("drive_c/ProgramData/GOG.com/Galaxy/redists/GalaxyCommunication.exe");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, b"old helper").unwrap();
        fs::write(destination.with_extension("ludomere-registered"), b"").unwrap();
        let source = root.path().join("new-helper.exe");
        fs::write(&source, b"new helper").unwrap();
        install_dummy_service(
            &super::super::UmuBackend::new(root.path().join("missing-proton")),
            root.path(),
            &super::super::UmuProfile::fallback(),
            &root.path().join("log"),
            &source,
        )
        .unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"new helper");
        assert!(!root.path().join("log").exists());
    }

    #[test]
    fn newer_packaged_helper_supersedes_active_version() {
        let root = tempfile::tempdir().unwrap();
        let bundled = root.path().join("bundled");
        let active = root.path().join("active");
        for (path, version) in [(&bundled, "0.4.0"), (&active, "0.3.2")] {
            fs::create_dir(path).unwrap();
            let mut files = std::collections::BTreeMap::new();
            for name in ["comet", "GalaxyCommunication.exe"] {
                fs::write(path.join(name), b"helper").unwrap();
                fs::set_permissions(path.join(name), fs::Permissions::from_mode(0o755)).unwrap();
                files.insert(name.into(), format!("{:x}", Sha256::digest(b"helper")));
            }
            fs::write(
                path.join("build.json"),
                serde_json::to_vec(&Build {
                    version: version.into(),
                    files,
                })
                .unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            select_directory(&bundled, Some(active.clone())).unwrap(),
            bundled
        );
        let mut build = verify_build(&active).unwrap();
        build.version = "0.5.0".into();
        fs::write(
            active.join("build.json"),
            serde_json::to_vec(&build).unwrap(),
        )
        .unwrap();
        assert_eq!(
            select_directory(&bundled, Some(active.clone())).unwrap(),
            active
        );
        fs::remove_file(active.join("comet")).unwrap();
        assert!(select_directory(&bundled, Some(active)).is_err());
    }
}
