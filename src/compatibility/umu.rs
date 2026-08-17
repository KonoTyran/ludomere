use super::*;
use std::{fs, path::PathBuf, process::Command};

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
        if request.background {
            quiet_setup(&mut c);
        }
        c
    }

    pub fn run_winetricks(
        &self,
        prefix: &std::path::Path,
        profile: &UmuProfile,
        verbs: &[String],
        working_directory: &std::path::Path,
        log_path: &std::path::Path,
    ) -> Result<CompatibilityProcess> {
        let mut command = Command::new(Self::executable());
        command
            .env("WINEPREFIX", prefix)
            .env("GAMEID", &profile.game_id)
            .env("STORE", "gog")
            .arg("winetricks")
            .arg("-q")
            .args(verbs)
            .current_dir(working_directory);
        quiet_setup(&mut command);
        CompatibilityProcess::spawn(command, log_path)
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
        let mut initialize = !prefix.exists();
        if prefix.exists() {
            if !prefix.join("dosdevices").is_dir() {
                if is_incomplete_umu_prefix(&prefix) {
                    initialize = true;
                } else {
                    return Err(CompatibilityFailure::PrefixConflict(prefix));
                }
            } else {
                validate_ownership(&prefix, &r.slug)?;
            }
        }
        if initialize {
            fs::create_dir_all(prefix.parent().unwrap())?;
            let game_directory = library.join(&r.slug);
            fs::create_dir_all(&game_directory)?;
            let mut c = prefix_initialization_command(&prefix, &r.profile, &game_directory);
            quiet_setup(&mut c);
            let mut p = CompatibilityProcess::spawn(c, &r.log_path)?;
            if !p.wait()?.success() {
                return Err(CompatibilityFailure::PrefixInitializationFailed);
            }
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

fn is_incomplete_umu_prefix(prefix: &std::path::Path) -> bool {
    fs::symlink_metadata(prefix.join("pfx")).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && fs::read_link(prefix.join("pfx")).is_ok_and(|target| target == std::path::Path::new("."))
        && prefix.join("tracked_files").is_file()
}

fn prefix_initialization_command(
    prefix: &std::path::Path,
    profile: &UmuProfile,
    game_directory: &std::path::Path,
) -> Command {
    let mut command = Command::new(UmuBackend::executable());
    command
        .env("WINEPREFIX", prefix)
        .env("GAMEID", &profile.game_id)
        .env("STORE", "gog")
        .env("PROTON_VERB", "waitforexitandrun")
        .current_dir(game_directory)
        .arg(prefix.join("drive_c/windows/regedit.exe"))
        .arg("/S");
    command
}

fn quiet_setup(command: &mut Command) {
    command
        .env("WINETRICKS_OPT_UNATTENDED", "1")
        .env("WINE_DISABLE_MENUBUILDER", "1")
        .env("WINEDLLOVERRIDES", "winemenubuilder.exe=d")
        .env("WINEDEBUG", "-all")
        .env("PROTON_LOG", "0");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(background: bool) -> CompatibilityRunRequest {
        CompatibilityRunRequest {
            prefix: "/prefix".into(),
            profile: UmuProfile::fallback(),
            executable: "/game.exe".into(),
            arguments: Vec::new(),
            working_directory: None,
            log_path: "/install.log".into(),
            background,
        }
    }

    #[test]
    fn setup_commands_are_quiet_without_affecting_game_launches() {
        let setup = UmuBackend::command(&request(true));
        assert!(setup.get_envs().any(|(name, value)| {
            name == "WINE_DISABLE_MENUBUILDER" && value == Some(std::ffi::OsStr::new("1"))
        }));
        let game = UmuBackend::command(&request(false));
        assert!(
            !game
                .get_envs()
                .any(|(name, _)| name == "WINE_DISABLE_MENUBUILDER")
        );
    }

    #[test]
    fn prefix_initialization_does_not_launch_an_empty_executable() {
        let command = prefix_initialization_command(
            std::path::Path::new("/prefix"),
            &UmuProfile::fallback(),
            std::path::Path::new("/library/game"),
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                std::ffi::OsStr::new("/prefix/drive_c/windows/regedit.exe"),
                std::ffi::OsStr::new("/S")
            ]
        );
        assert!(command.get_envs().any(|(name, value)| {
            name == "PROTON_VERB" && value == Some(std::ffi::OsStr::new("waitforexitandrun"))
        }));
        assert_eq!(
            command.get_current_dir(),
            Some(std::path::Path::new("/library/game"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn recognizes_only_umu_partial_prefixes_for_retry() {
        use std::os::unix::fs::symlink;
        let root =
            std::env::temp_dir().join(format!("ludomere-partial-prefix-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert!(!is_incomplete_umu_prefix(&root));
        symlink(".", root.join("pfx")).unwrap();
        fs::write(root.join("tracked_files"), b"").unwrap();
        assert!(is_incomplete_umu_prefix(&root));
        fs::remove_dir_all(root).unwrap();
    }
}
