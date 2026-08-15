use super::{CompatibilityFailure, Result};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    fs::OpenOptions,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub struct CompatibilityProcess {
    child: Child,
    group: u32,
}
impl CompatibilityProcess {
    pub(crate) fn spawn(mut command: Command, log_path: &std::path::Path) -> Result<Self> {
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
