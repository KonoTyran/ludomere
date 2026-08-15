use super::*;
use std::{path::PathBuf, process::Command};

#[derive(Default, Clone)]
pub struct UmuBackend;
impl UmuBackend {
    fn executable() -> PathBuf {
        PathBuf::from("/usr/bin/umu-run")
    }
    fn command(request: &CompatibilityRunRequest) -> Command {
        let mut c = Command::new(Self::executable());
        c.env("WINEPREFIX", &request.prefix)
            .env("GAMEID", &request.profile.game_id)
            .env("STORE", "gog")
            .env("PROTON_VERB", "waitforexitandrun")
            .arg(&request.executable)
            .args(&request.arguments);
        if let Some(dir) = &request.working_directory {
            c.current_dir(dir);
        }
        c
    }
}
impl CompatibilityBackend for UmuBackend {
    fn status(&self) -> Result<CompatibilityBackendStatus> {
        let exe = Self::executable();
        if !exe.is_file() {
            return Ok(CompatibilityBackendStatus {
                kind: CompatibilityBackendKind::Umu,
                available: false,
                version: None,
                healthy: false,
                message: Some(
                    "Install the umu-launcher package from the Arch multilib repository.".into(),
                ),
            });
        }
        let out = Command::new(exe).arg("--version").output()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let version = text
            .split_whitespace()
            .find(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .map(str::to_owned);
        let healthy = out.status.success()
            && version
                .as_deref()
                .and_then(|v| v.split('.').next()?.parse::<u32>().ok())
                .is_some_and(|major| major >= 1);
        Ok(CompatibilityBackendStatus {
            kind: CompatibilityBackendKind::Umu,
            available: true,
            version,
            healthy,
            message: (!healthy).then(|| "UMU 1.4 or newer is required".into()),
        })
    }
    fn initialize_prefix(&self, r: InitializePrefixRequest) -> Result<CompatibilityPrefix> {
        validate_slug(&r.slug)?;
        let library = validate_library(&r.library)?;
        let prefix = prefix_path(&library, &r.slug);
        if prefix.exists() {
            if !prefix.join("dosdevices").is_dir() {
                return Err(CompatibilityFailure::PrefixConflict(prefix));
            }
            validate_ownership(&prefix, &r.slug)?;
        }
        if !prefix.exists() {
            fs::create_dir_all(prefix.parent().unwrap())?;
            let req = CompatibilityRunRequest {
                prefix: prefix.clone(),
                profile: r.profile,
                executable: PathBuf::new(),
                arguments: vec![],
                working_directory: None,
                log_path: r.log_path.clone(),
            };
            let mut c = Command::new(Self::executable());
            c.env("WINEPREFIX", &req.prefix)
                .env("GAMEID", &req.profile.game_id)
                .env("STORE", "gog")
                .arg("");
            let mut p = CompatibilityProcess::spawn(c, &req.log_path)?;
            // Prefix creation deliberately ends by failing to launch the empty
            // executable. Validate Proton's durable output instead.
            let _status = p.wait()?;
            validate_prefix_structure(&prefix)?;
            write_ownership(&prefix, &r.slug)?;
        }
        configure_library_drive(&prefix, &library)?;
        Ok(CompatibilityPrefix {
            library_id: r.library_id,
            relative_path: prefix_relative(&r.slug),
            managed_by_ludomere: true,
        })
    }
    fn run_executable(&self, r: CompatibilityRunRequest) -> Result<CompatibilityProcess> {
        if !r.executable.is_file() {
            return Err(CompatibilityFailure::ExecutableMissing(r.executable));
        }
        let c = Self::command(&r);
        CompatibilityProcess::spawn(c, &r.log_path)
    }
    fn stop(&self, p: &mut CompatibilityProcess) -> Result<()> {
        p.stop()
    }
}
use std::fs;
