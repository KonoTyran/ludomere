use super::CompatibilityBackend;
use crate::{auth, state::StateStore};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

const VERSION: &str = "v0.2.0";
const COMET_URL: &str =
    "https://github.com/imLinguin/comet/releases/download/v0.2.0/comet-x86_64-unknown-linux-gnu";
const COMET_SHA256: &str = "cf9a0e44dbedd0fea283ac6398e1277df7516711b486c21629103c873b0a6a7d";
const SERVICE_URL: &str =
    "https://github.com/imLinguin/comet/releases/download/v0.2.0/GalaxyCommunication-dummy.exe";
const SERVICE_SHA256: &str = "abc208076a778ee738cae8451c9be7ab33c9787b0b69b2e7e4ffc70becc39e1e";
const GALAXY_CLIENT_ID: &str = "46899977096215655";

pub struct CometSession {
    child: Child,
    credential_root: PathBuf,
}

impl Drop for CometSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.credential_root);
    }
}

pub fn start(
    prefix: &Path,
    profile: &super::UmuProfile,
    log_path: &Path,
) -> Result<Option<CometSession>> {
    let Some(token) = auth::load_saved_token()? else {
        return Ok(None);
    };
    let Some(account) = StateStore::open()?.cached_profile()? else {
        return Ok(None);
    };
    let runtime = ensure_runtime()?;
    install_dummy_service(prefix, profile, log_path, &runtime.service)?;
    start_session(&runtime.comet, log_path, &token, &account.username)
}

pub fn start_native(log_path: &Path) -> Result<Option<CometSession>> {
    let Some(token) = auth::load_saved_token()? else {
        return Ok(None);
    };
    let Some(account) = StateStore::open()?.cached_profile()? else {
        return Ok(None);
    };
    let runtime = ensure_runtime()?;
    start_session(&runtime.comet, log_path, &token, &account.username)
}

fn start_session(
    comet: &Path,
    log_path: &Path,
    token: &auth::Token,
    username: &str,
) -> Result<Option<CometSession>> {
    let credential_root = write_credentials(token)?;
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    let stderr = stdout.try_clone()?;
    let child = Command::new(comet)
        .env("XDG_CONFIG_PATH", &credential_root)
        .env("COMET_IDLE_WAIT", "5")
        .args(["--from-heroic", "--username", username, "--quit"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .context("could not start Comet")?;
    Ok(Some(CometSession {
        child,
        credential_root,
    }))
}

struct Runtime {
    comet: PathBuf,
    service: PathBuf,
}

fn ensure_runtime() -> Result<Runtime> {
    let root = crate::identity::data_root()
        .join("tools/comet")
        .join(VERSION);
    fs::create_dir_all(&root)?;
    let comet = root.join("comet");
    let service = root.join("GalaxyCommunication.exe");
    ensure_asset(&comet, COMET_URL, COMET_SHA256, true)?;
    ensure_asset(&service, SERVICE_URL, SERVICE_SHA256, false)?;
    Ok(Runtime { comet, service })
}

fn ensure_asset(path: &Path, url: &str, expected: &str, executable: bool) -> Result<()> {
    if path.is_file() && digest(path)? == expected {
        return Ok(());
    }
    let response = reqwest::blocking::Client::builder()
        .user_agent(crate::identity::USER_AGENT)
        .build()?
        .get(url)
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size > 64 * 1024 * 1024)
    {
        bail!("Comet asset is unexpectedly large")
    }
    let bytes = response.bytes()?;
    if bytes.len() > 64 * 1024 * 1024 {
        bail!("Comet asset is unexpectedly large")
    }
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        bail!("Comet asset failed integrity verification")
    }
    let partial = path.with_extension(format!("part-{}", std::process::id()));
    fs::write(&partial, &bytes)?;
    if executable {
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(partial, path)?;
    Ok(())
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
    prefix: &Path,
    profile: &super::UmuProfile,
    log_path: &Path,
    source: &Path,
) -> Result<()> {
    let destination =
        prefix.join("drive_c/ProgramData/GOG.com/Galaxy/redists/GalaxyCommunication.exe");
    let registration = destination.with_extension("ludomere-registered");
    if destination.is_file() && digest(&destination)? == SERVICE_SHA256 && registration.is_file() {
        return Ok(());
    }
    fs::create_dir_all(destination.parent().unwrap())?;
    fs::copy(source, &destination)?;
    let sc = prefix.join("drive_c/windows/system32/sc.exe");
    if !sc.is_file() {
        bail!("the compatibility prefix does not contain sc.exe")
    }
    let mut process = super::default_backend().run_executable(super::CompatibilityRunRequest {
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
    })?;
    let status = process.wait()?;
    if !status.success() {
        bail!("could not register the Galaxy Communication service: {status}")
    }
    File::create(registration)?;
    Ok(())
}
