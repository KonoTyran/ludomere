use super::{CompatibilityFailure, Result};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub struct CompatibilityProcess {
    child: Child,
    group: u32,
    activity: Option<crate::profile_reset::ActivityGuard>,
}
impl CompatibilityProcess {
    pub(crate) fn spawn(mut command: Command, log_path: &std::path::Path) -> Result<Self> {
        let activity = crate::profile_reset::begin_activity("compatibility process")
            .map_err(std::io::Error::other)?;
        append_command_log(log_path, &command)?;
        let out = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        let err = out.try_clone()?;
        command.stdin(Stdio::null()).stdout(out).stderr(err);
        #[cfg(unix)]
        command.process_group(0);
        let child = command
            .spawn()
            .map_err(|_| CompatibilityFailure::InstallerLaunchRejected)?;
        let group = child.id();
        Ok(Self {
            child,
            group,
            activity: Some(activity),
        })
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait()
    }
    pub(crate) fn group_running(&mut self) -> std::io::Result<bool> {
        self.child.try_wait()?;
        Self::group_is_running(self.group)
    }
    pub(crate) fn group_id(&self) -> u32 {
        self.group
    }
    pub(crate) fn group_is_running(group: u32) -> std::io::Result<bool> {
        let group = i32::try_from(group).map_err(std::io::Error::other)?;
        if group <= 1 {
            return Err(std::io::Error::other("Invalid setup process group"));
        }
        if unsafe { libc::kill(-group, 0) } == 0 {
            // Orphaned zombies can outlive their launcher under a non-reaping container
            // init. They cannot write; every live member of our group must still drain.
            for entry in std::fs::read_dir("/proc")? {
                let entry = entry?;
                let Some(pid) = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<i32>().ok())
                else {
                    continue;
                };
                if unsafe { libc::getpgid(pid) } != group {
                    continue;
                }
                match std::fs::read_to_string(entry.path().join("stat")) {
                    Ok(stat) => {
                        if stat
                            .rsplit_once(") ")
                            .is_none_or(|(_, fields)| !fields.starts_with("Z "))
                        {
                            return Ok(true);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            return Ok(false);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(false)
        } else {
            Err(error)
        }
    }
    pub fn stop(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            let group = i32::try_from(self.group).map_err(|_| CompatibilityFailure::StopFailed)?;
            unsafe { libc::kill(-group, libc::SIGTERM) };
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                if !self.group_running()? {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            unsafe { libc::kill(-group, libc::SIGKILL) };
            self.child.wait()?;
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                if !self.group_running()? {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(CompatibilityFailure::StopFailed)
        }
        #[cfg(not(unix))]
        {
            self.child
                .kill()
                .map_err(|_| CompatibilityFailure::StopFailed)
        }
    }
}

impl Drop for CompatibilityProcess {
    fn drop(&mut self) {
        if !matches!(self.group_running(), Ok(false))
            && let Some(activity) = self.activity.take()
        {
            // A dropped handle does not terminate its child. Refuse reset until the next app start.
            std::mem::forget(activity);
        }
    }
}

pub(crate) fn append_step_log(log_path: &Path, step: &str) -> Result<()> {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    writeln!(log, "[install] {step}")?;
    Ok(())
}

fn append_command_log(log_path: &Path, command: &Command) -> Result<()> {
    let mut parts = vec![command.get_program().to_string_lossy().into_owned()];
    let mut redact_next = false;
    for argument in command.get_args() {
        let value = argument.to_string_lossy();
        let lower = value.to_ascii_lowercase();
        let sensitive = redact_next
            || ["token", "password", "secret", "authorization"]
                .iter()
                .any(|name| {
                    lower.starts_with(&format!("--{name}="))
                        || lower.starts_with(&format!("/{name}="))
                        || lower.starts_with(&format!("{name}="))
                });
        parts.push(if sensitive {
            "[redacted]".into()
        } else if value.starts_with("http://") || value.starts_with("https://") {
            value.split('?').next().unwrap_or_default().to_owned()
        } else {
            value.into_owned()
        });
        redact_next = [
            "--token",
            "--password",
            "--secret",
            "--authorization",
            "/password",
            "/d",
        ]
        .iter()
        .any(|name| lower == *name);
    }
    let working_directory = command
        .get_current_dir()
        .map_or_else(|| "<inherited>".into(), |path| path.display().to_string());
    let command = parts
        .iter()
        .map(|part| shell_words::quote(part))
        .collect::<Vec<_>>()
        .join(" ");
    append_step_log(
        log_path,
        &format!("command: {command} (working directory: {working_directory})"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SyntheticGroup {
        process: CompatibilityProcess,
        directory: tempfile::TempDir,
    }

    impl SyntheticGroup {
        fn start(leader_exit: Option<i32>, ignore_term: bool) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let mut command = Command::new("python3");
            command.args([
                "-I",
                "-c",
                r#"
import os, signal, sys, time
from pathlib import Path
output = Path(sys.argv[1])
if sys.argv[3] == 'ignore':
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
reader, writer = os.pipe()
child = os.fork()
if child == 0:
    os.close(reader)
    with output.open('wb', buffering=0) as stream:
        stream.write(b'ready\n')
        os.write(writer, b'1')
        os.close(writer)
        end = time.monotonic() + 15
        while time.monotonic() < end:
            stream.write(b'owned fixture\n')
            time.sleep(0.025)
    os._exit(0)
os.close(writer)
os.read(reader, 1)
os.close(reader)
if sys.argv[2] != 'running':
    sys.exit(int(sys.argv[2]))
os.waitpid(child, 0)
"#,
            ]);
            command.arg(directory.path().join("writes"));
            command.arg(leader_exit.map_or_else(|| "running".into(), |code| code.to_string()));
            command.arg(if ignore_term { "ignore" } else { "normal" });
            let process =
                CompatibilityProcess::spawn(command, &directory.path().join("output.log")).unwrap();
            let mut fixture = Self { process, directory };
            let deadline = Instant::now() + Duration::from_secs(5);
            while !fixture.directory.path().join("writes").exists() && Instant::now() < deadline {
                assert!(
                    fixture.process.group_running().unwrap(),
                    "fixture exited before becoming ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(fixture.directory.path().join("writes").exists());
            fixture
        }

        fn written(&self) -> u64 {
            std::fs::metadata(self.directory.path().join("writes"))
                .unwrap()
                .len()
        }
    }

    impl Drop for SyntheticGroup {
        fn drop(&mut self) {
            let _ = self.process.stop();
        }
    }

    #[test]
    fn exited_setup_leader_does_not_hide_live_owned_children() {
        for code in [0, 7] {
            let mut fixture = SyntheticGroup::start(Some(code), false);
            assert_eq!(fixture.process.wait().unwrap().code(), Some(code));
            assert!(fixture.process.group_running().unwrap());
            let before = fixture.written();
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                fixture.written() > before,
                "child must remain active after leader exit"
            );
            fixture.process.stop().unwrap();
            assert!(!fixture.process.group_running().unwrap());
            let stopped = fixture.written();
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(
                fixture.written(),
                stopped,
                "drained setup must not write later"
            );
        }
    }

    #[test]
    fn cancelling_term_ignoring_setup_drains_the_entire_owned_group() {
        let mut fixture = SyntheticGroup::start(None, true);
        assert!(fixture.process.try_wait().unwrap().is_none());
        let before = fixture.written();
        let started = Instant::now();
        fixture.process.stop().unwrap();
        assert!(started.elapsed() >= Duration::from_secs(3));
        assert!(started.elapsed() < Duration::from_secs(8));
        assert!(!fixture.process.group_running().unwrap());
        assert!(!fixture.process.wait().unwrap().success());
        let stopped = fixture.written();
        assert!(stopped > before, "fixture ignored TERM until escalation");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fixture.written(), stopped);
    }

    #[test]
    fn command_log_redacts_secrets_and_url_queries() {
        let path = std::env::temp_dir().join(format!(
            "ludomere-command-log-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut command = Command::new("setup.exe");
        command.args([
            "--password",
            "private",
            "https://example.invalid/file?token=private",
        ]);
        append_command_log(&path, &command).unwrap();
        let log = std::fs::read_to_string(&path).unwrap();
        assert!(log.contains("setup.exe --password '[redacted]' https://example.invalid/file"));
        assert!(!log.contains("private"));
        std::fs::remove_file(path).unwrap();
    }
}
