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
}
impl CompatibilityProcess {
    pub(crate) fn spawn(mut command: Command, log_path: &std::path::Path) -> Result<Self> {
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
        Ok(Self { child, group })
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait()
    }
    pub fn stop(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            let group = i32::try_from(self.group).map_err(|_| CompatibilityFailure::StopFailed)?;
            unsafe { libc::kill(-group, libc::SIGTERM) };
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                if self.child.try_wait()?.is_some() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            unsafe { libc::kill(-group, libc::SIGKILL) };
            Ok(())
        }
        #[cfg(not(unix))]
        {
            self.child
                .kill()
                .map_err(|_| CompatibilityFailure::StopFailed)
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
    append_step_log(
        log_path,
        &format!(
            "command: {}",
            parts
                .iter()
                .map(|part| shell_words::quote(part))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
