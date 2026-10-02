use super::*;
use std::{fs, path::PathBuf, process::Command};

#[derive(Default, Clone)]
pub struct UmuBackend {
    proton: PathBuf,
}
impl UmuBackend {
    pub(super) fn new(proton: PathBuf) -> Self {
        Self { proton }
    }
    pub fn executable() -> PathBuf {
        helper_executable(
            std::env::var_os("LUDOMERE_UMU_RUN").map(PathBuf::from),
            PathBuf::from("/usr/lib/ludomere/umu/umu-run"),
            std::env::current_exe().ok(),
            cfg!(debug_assertions),
        )
    }

    pub fn command(&self, request: &CompatibilityRunRequest) -> Result<Command> {
        super::check_prerequisites(&self.proton)?;
        Ok(self.run_command(request))
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new(Self::executable());
        command
            .env("PROTONPATH", &self.proton)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("UMU_RUNTIME_UPDATE", "0")
            .env("UMU_FOLDERS_PATH", crate::identity::data_root())
            .env_remove("UMU_NO_PROTON")
            .env_remove("UMU_NO_RUNTIME")
            .env_remove("RUNTIMEPATH");
        command
    }

    fn run_command(&self, request: &CompatibilityRunRequest) -> Command {
        let mut c = self.base_command();
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
        super::check_prerequisites(&self.proton)?;
        let mut command = self.base_command();
        let explicit_ca = ["CURL_CA_BUNDLE", "SSL_CERT_FILE", "SSL_CERT_DIR"]
            .into_iter()
            .filter_map(|key| std::env::var_os(key).map(|value| (key, value)))
            .collect::<Vec<_>>();
        let ca_policy =
            configure_winetricks_ca(&mut command, std::path::Path::new("/"), &explicit_ca);
        super::append_step_log(
            log_path,
            &format!("Ludomere Winetricks CA policy v1: {ca_policy}"),
        )?;
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

fn configure_winetricks_ca(
    command: &mut Command,
    host_root: &std::path::Path,
    explicit: &[(&str, std::ffi::OsString)],
) -> String {
    if !explicit.is_empty() {
        let mut mapped = Vec::new();
        for (key, value) in explicit {
            let translated = if *key == "SSL_CERT_DIR" {
                let paths = std::env::split_paths(value)
                    .map(|path| host_ca_path(&path, host_root, true).unwrap_or(path));
                std::env::join_paths(paths).ok()
            } else {
                host_ca_path(std::path::Path::new(value), host_root, false)
                    .map(PathBuf::into_os_string)
            };
            if let Some(translated) = translated
                && translated != *value
            {
                command.env(key, translated);
                mapped.push(*key);
            }
        }
        return format!(
            "preserving inherited {}; managed default not applied{}",
            explicit
                .iter()
                .map(|(key, _)| *key)
                .collect::<Vec<_>>()
                .join(", "),
            if mapped.is_empty() {
                String::new()
            } else {
                format!(
                    "; mapped selected host trust paths for {} into the runtime",
                    mapped.join(", ")
                )
            }
        );
    }
    // pressure-vessel replaces /etc, but exposes host /etc and /usr read-only
    // below /run/host. Resolve host symlinks before using that container path.
    for candidate in ["etc/ssl/cert.pem", "etc/ssl/certs/ca-certificates.crt"] {
        if let Some(path) = host_ca_path(&host_root.join(candidate), host_root, false) {
            command.env("CURL_CA_BUNDLE", path);
            return "mapped host default selected; TLS verification remains enabled".into();
        }
    }
    "no suitable host default found; curl retains its existing certificate configuration".into()
}

fn host_ca_path(
    path: &std::path::Path,
    host_root: &std::path::Path,
    directory: bool,
) -> Option<PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    if !path.is_absolute() {
        return None;
    }
    let path = path.canonicalize().ok()?;
    let relative = path.strip_prefix(host_root).ok()?;
    if !(relative.starts_with("etc") || relative.starts_with("usr")) {
        return None;
    }
    let metadata = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path)
        .ok()?
        .metadata()
        .ok()?;
    if (directory && metadata.is_dir()) || (!directory && metadata.is_file() && metadata.len() > 0)
    {
        Some(std::path::Path::new("/run/host").join(relative))
    } else {
        None
    }
}

impl UmuBackend {
    pub(crate) fn initialize_prefix_controlled(
        &self,
        r: InitializePrefixRequest,
        run_initialization: impl FnOnce(Command, &std::path::Path) -> anyhow::Result<()>,
    ) -> Result<CompatibilityPrefix> {
        self.initialize_prefix_inner(r, false, run_initialization)
    }

    pub(crate) fn rebuild_prefix_controlled(
        &self,
        r: InitializePrefixRequest,
        run_initialization: impl FnOnce(Command, &std::path::Path) -> anyhow::Result<()>,
    ) -> Result<CompatibilityPrefix> {
        validate_ownership(&prefix_path(&r.library, &r.slug), &r.slug)?;
        self.initialize_prefix_inner(r, true, run_initialization)
    }

    fn initialize_prefix_inner(
        &self,
        r: InitializePrefixRequest,
        rebuild: bool,
        run_initialization: impl FnOnce(Command, &std::path::Path) -> anyhow::Result<()>,
    ) -> Result<CompatibilityPrefix> {
        super::check_prerequisites(&self.proton)?;
        validate_slug(&r.slug)?;
        let library = validate_library(&r.library)?;
        let prefix = prefix_path(&library, &r.slug);
        let mut initialize = rebuild || !prefix.exists();
        if prefix.exists() && !rebuild {
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
            let mut c = prefix_initialization_command(
                self.base_command(),
                &prefix,
                &r.profile,
                &game_directory,
            );
            quiet_setup(&mut c);
            run_initialization(c, &r.log_path)
                .map_err(|error| CompatibilityFailure::Io(format!("{error:#}")))?;
            // Initialization creates a new prefix generation, never adopt old setup receipts.
            match fs::remove_file(prefix.join(".ludomere-gog-dependencies.json")) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
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
}

fn helper_executable(
    override_path: Option<PathBuf>,
    packaged: PathBuf,
    executable: Option<PathBuf>,
    development: bool,
) -> PathBuf {
    if let Some(path) = override_path.filter(|path| path.is_absolute()) {
        return path;
    }
    if packaged.is_file() {
        return packaged;
    }
    if development
        && let Some(staged) = executable
            .as_deref()
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
            .map(|root| root.join("helpers/umu/umu-run"))
        && staged.is_file()
    {
        return staged;
    }
    packaged
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
                    "Ludomere's bundled UMU helper is missing. Reinstall Ludomere.".into(),
                ),
            });
        }
        let out = self.base_command().arg("--version").output()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let version = text
            .split_whitespace()
            .find(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .map(str::to_owned);
        let healthy = out.status.success() && version.as_deref() == Some("1.4.4");
        Ok(CompatibilityBackendStatus {
            kind: CompatibilityBackendKind::Umu,
            available: true,
            version,
            healthy,
            message: (!healthy).then(|| "Ludomere requires its bundled UMU 1.4.4 helper.".into()),
        })
    }
    fn initialize_prefix(&self, r: InitializePrefixRequest) -> Result<CompatibilityPrefix> {
        self.initialize_prefix_controlled(r, |command, log| {
            let mut process = CompatibilityProcess::spawn(command, log)?;
            if !process.wait()?.success() {
                process.stop()?;
                return Err(CompatibilityFailure::PrefixInitializationFailed.into());
            }
            while process.group_running()? {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(())
        })
    }
    fn run_executable(&self, r: CompatibilityRunRequest) -> Result<CompatibilityProcess> {
        if !r.executable.is_file() {
            return Err(CompatibilityFailure::ExecutableMissing(r.executable));
        }
        let c = self.command(&r)?;
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
    mut command: Command,
    prefix: &std::path::Path,
    profile: &UmuProfile,
    game_directory: &std::path::Path,
) -> Command {
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

    #[test]
    fn winetricks_ca_uses_resolved_host_trust_without_overriding_user_settings() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let extracted = root
            .path()
            .join("etc/ca-certificates/extracted/tls-ca-bundle.pem");
        fs::create_dir_all(extracted.parent().unwrap()).unwrap();
        fs::write(&extracted, b"inert certificate fixture").unwrap();
        fs::create_dir_all(root.path().join("etc/ssl")).unwrap();
        symlink(
            "../ca-certificates/extracted/tls-ca-bundle.pem",
            root.path().join("etc/ssl/cert.pem"),
        )
        .unwrap();
        let mut command = Command::new("never-executed");
        assert!(
            configure_winetricks_ca(&mut command, root.path(), &[])
                .starts_with("mapped host default selected")
        );
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            vec![(
                std::ffi::OsStr::new("CURL_CA_BUNDLE"),
                Some(std::ffi::OsStr::new(
                    "/run/host/etc/ca-certificates/extracted/tls-ca-bundle.pem"
                )),
            )]
        );
        assert_eq!(command.get_args().count(), 0);
        for key in ["CURL_CA_BUNDLE", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
            for value in ["/custom/user-selected trust", ""] {
                let mut command = Command::new("never-executed");
                command.env(key, value);
                assert_eq!(
                    configure_winetricks_ca(&mut command, root.path(), &[(key, value.into())]),
                    format!("preserving inherited {key}; managed default not applied")
                );
                assert_eq!(
                    command.get_envs().collect::<Vec<_>>(),
                    vec![(std::ffi::OsStr::new(key), Some(std::ffi::OsStr::new(value)))]
                );
            }
        }
    }

    #[test]
    fn winetricks_ca_policy_log_contains_names_without_private_values() {
        let root = tempfile::tempdir().unwrap();
        let mut command = Command::new("never-executed");
        command.env("CURL_CA_BUNDLE", "/private/ca-value-canary");
        command.env("SSL_CERT_DIR", "/private/directory-value-canary");
        let policy = configure_winetricks_ca(
            &mut command,
            root.path(),
            &[
                ("CURL_CA_BUNDLE", "/private/ca-value-canary".into()),
                ("SSL_CERT_DIR", "/private/directory-value-canary".into()),
            ],
        );
        let log = root.path().join("installation.log");
        crate::compatibility::append_step_log(
            &log,
            &format!("Ludomere Winetricks CA policy v1: {policy}"),
        )
        .unwrap();
        let text = fs::read_to_string(log).unwrap();
        assert!(text.contains(
            "preserving inherited CURL_CA_BUNDLE, SSL_CERT_DIR; managed default not applied"
        ));
        assert!(!text.contains("canary"));
        assert!(!text.contains("/private"));
        assert_eq!(command.get_args().count(), 0);
    }

    #[test]
    fn winetricks_preserves_selected_ca_files_across_host_layouts() {
        use std::os::unix::fs::symlink;
        for selected in [
            "etc/ca-certificates/extracted/tls-ca-bundle.pem",
            "etc/ssl/certs/ca-certificates.crt",
            "usr/local/share/certificates/custom trust.pem",
        ] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join(selected);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, b"only the selected trust fixture").unwrap();
            fs::create_dir_all(root.path().join("etc/ssl")).unwrap();
            let link = root.path().join("etc/ssl/cert.pem");
            symlink(&target, &link).unwrap();
            for key in ["SSL_CERT_FILE", "CURL_CA_BUNDLE"] {
                let mut command = Command::new("never-executed");
                command.env(key, &link);
                let policy = configure_winetricks_ca(
                    &mut command,
                    root.path(),
                    &[(key, link.as_os_str().to_owned())],
                );
                assert!(policy.contains("mapped selected host trust paths"));
                assert!(!policy.contains(selected));
                assert_eq!(
                    command.get_envs().collect::<Vec<_>>(),
                    vec![(
                        std::ffi::OsStr::new(key),
                        Some(std::path::Path::new("/run/host").join(selected).as_os_str()),
                    )]
                );
                assert_eq!(
                    fs::read(&target).unwrap(),
                    b"only the selected trust fixture"
                );
            }
        }
    }

    #[test]
    fn winetricks_cargo_ca_pair_maps_without_replacing_custom_directory_entries() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let root = tempfile::tempdir().unwrap();
        let cert = root.path().join("etc/ssl/cert.pem");
        let directory = root.path().join("etc/ssl/certs");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&cert, b"selected Cargo-shaped certificate").unwrap();
        let mut dirs = directory.as_os_str().to_owned();
        dirs.push(":relative::/missing/explicit:");
        dirs.push(std::ffi::OsString::from_vec(b"non-utf8-\xff".to_vec()));
        dirs.push(":");
        let explicit = [
            ("SSL_CERT_FILE", cert.into_os_string()),
            ("SSL_CERT_DIR", dirs),
        ];
        let mut command = Command::new("never-executed");
        command.envs(explicit.iter().map(|(key, value)| (*key, value)));
        let policy = configure_winetricks_ca(&mut command, root.path(), &explicit);
        assert!(
            policy.contains("mapped selected host trust paths for SSL_CERT_FILE, SSL_CERT_DIR")
        );
        let env = command
            .get_envs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            env.len(),
            2,
            "no new default may override the selected trust"
        );
        assert_eq!(
            env[std::ffi::OsStr::new("SSL_CERT_FILE")].unwrap(),
            "/run/host/etc/ssl/cert.pem"
        );
        assert_eq!(
            env[std::ffi::OsStr::new("SSL_CERT_DIR")]
                .unwrap()
                .as_bytes(),
            b"/run/host/etc/ssl/certs:relative::/missing/explicit:non-utf8-\xff:"
        );
        assert_eq!(command.get_args().count(), 0);
    }

    #[test]
    fn winetricks_ca_fallback_rejects_missing_empty_and_unmapped_targets() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let certs = root.path().join("etc/ssl/certs");
        fs::create_dir_all(&certs).unwrap();
        for contents in [None, Some(b"".as_slice())] {
            if let Some(contents) = contents {
                fs::write(certs.join("ca-certificates.crt"), contents).unwrap();
            }
            let mut command = Command::new("never-executed");
            assert!(
                configure_winetricks_ca(&mut command, root.path(), &[])
                    .starts_with("no suitable host default found")
            );
            assert_eq!(command.get_envs().count(), 0);
        }
        fs::create_dir_all(root.path().join("var/private")).unwrap();
        fs::write(root.path().join("var/private/bundle"), b"unmapped").unwrap();
        symlink(
            "../../var/private/bundle",
            root.path().join("etc/ssl/cert.pem"),
        )
        .unwrap();
        let mut command = Command::new("never-executed");
        assert!(
            configure_winetricks_ca(&mut command, root.path(), &[])
                .starts_with("no suitable host default found")
        );
        assert_eq!(command.get_envs().count(), 0);
        fs::write(certs.join("ca-certificates.crt"), b"inert Debian bundle").unwrap();
        assert!(
            configure_winetricks_ca(&mut command, root.path(), &[])
                .starts_with("mapped host default selected")
        );
        assert_eq!(
            command.get_envs().next().unwrap().1,
            Some(std::ffi::OsStr::new(
                "/run/host/etc/ssl/certs/ca-certificates.crt"
            ))
        );
    }

    #[test]
    fn development_helper_resolution_is_anchored_and_respects_explicit_and_packaged_paths() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("target/debug/ludomere");
        let staged = root.path().join("target/helpers/umu/umu-run");
        fs::create_dir_all(staged.parent().unwrap()).unwrap();
        fs::write(&staged, b"fixture; never executed").unwrap();
        let packaged = root.path().join("packaged/umu-run");
        assert_eq!(
            helper_executable(None, packaged.clone(), Some(executable.clone()), true),
            staged
        );
        assert_eq!(
            helper_executable(None, packaged.clone(), Some(executable.clone()), false),
            packaged
        );
        let missing_override = root.path().join("explicit-missing");
        assert_eq!(
            helper_executable(
                Some(missing_override.clone()),
                packaged.clone(),
                Some(executable.clone()),
                true
            ),
            missing_override
        );
        fs::create_dir_all(packaged.parent().unwrap()).unwrap();
        fs::write(&packaged, b"fixture; never executed").unwrap();
        assert_eq!(
            helper_executable(None, packaged.clone(), Some(executable), true),
            packaged
        );
        assert_eq!(
            helper_executable(
                None,
                root.path().join("absent"),
                Some(root.path().join("other/debug/ludomere")),
                true
            ),
            root.path().join("absent")
        );
    }

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
        let backend = UmuBackend::new("/selected/GE-Proton".into());
        let setup = backend.run_command(&request(true));
        assert!(setup.get_envs().any(|(name, value)| {
            name == "WINE_DISABLE_MENUBUILDER" && value == Some(std::ffi::OsStr::new("1"))
        }));
        let game = backend.run_command(&request(false));
        for command in [&setup, &game] {
            assert!(command.get_envs().any(|(name, value)| name == "PROTONPATH"
                && value == Some(std::ffi::OsStr::new("/selected/GE-Proton"))));
            assert!(
                command
                    .get_envs()
                    .any(|(name, value)| name == "UMU_RUNTIME_UPDATE"
                        && value == Some(std::ffi::OsStr::new("0")))
            );
        }
        assert!(
            !game
                .get_envs()
                .any(|(name, _)| name == "WINE_DISABLE_MENUBUILDER")
        );
    }

    #[test]
    fn prefix_initialization_does_not_launch_an_empty_executable() {
        let command = prefix_initialization_command(
            UmuBackend::new("/selected/proton".into()).base_command(),
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
