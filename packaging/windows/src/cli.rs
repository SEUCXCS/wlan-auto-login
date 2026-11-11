//! Invoke the public CLI silently, with a bounded lifetime and captured diagnostics.
use crate::{Form, Scope, win};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::OpenOptions,
    io::Write,
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use zeroize::Zeroizing;

const OPERATION_TIMEOUT: Duration = Duration::from_secs(180);
struct Job(win::Handle);
impl Job {
    fn attach(child: &Child) -> Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            ensure!(
                !handle.is_null(),
                "Could not create an installer process job"
            );
            let job = Self(win::Handle(handle));
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            win::check(SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ))?;
            win::check(AssignProcessToJobObject(
                handle,
                child.as_raw_handle().cast(),
            ))?;
            Ok(job)
        }
    }
    fn terminate(&self) -> Result<()> {
        win::check(unsafe { TerminateJobObject(self.0.0, 1460) })
    }
}

fn command(
    binary: &Path,
    scope: Scope,
    arguments: &[&str],
    form: Option<&Form>,
) -> Result<Command> {
    let mut command = Command::new(binary);
    command
        .arg(scope.cli_flag())
        .args(arguments)
        .creation_flags(0x08000000)
        .env_remove("WLANLOGIN_PASSWORD")
        .env_remove("WLANLOGIN_USERNAME")
        .env_remove("WLANLOGIN_LINE")
        .stdin(if form.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    if let Some(form) = form {
        form.validate()?;
        command
            .arg("--password-stdin")
            .arg("--username")
            .arg(&form.username)
            .arg("--line")
            .arg(&form.line);
    }
    Ok(command)
}

pub fn run(
    binary: &Path,
    scope: Scope,
    arguments: &[&str],
    form: Option<&Form>,
    log: &Path,
) -> Result<()> {
    win::plain_path(log)?;
    let output = OpenOptions::new().create(true).append(true).open(log)?;
    let mut command = command(binary, scope, arguments, form)?;
    command
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output));
    let mut child = command.spawn().context("Could not start wlanlogin")?;
    let job = match Job::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let writer = if let Some(form) = form {
        let mut input = child.stdin.take().context("CLI stdin unavailable")?;
        let mut password = Zeroizing::new(form.password.as_bytes().to_vec());
        password.push(b'\n');
        Some(std::thread::spawn(move || input.write_all(&password)))
    } else {
        None
    };
    let result = wait(&mut child, &job, OPERATION_TIMEOUT);
    if result.is_err() {
        let _ = job.terminate();
        let _ = child.kill();
        let _ = child.wait();
    }
    let written = writer
        .map(|writer| {
            writer
                .join()
                .map_err(|_| anyhow::anyhow!("CLI stdin writer terminated"))
        })
        .transpose()?;
    let status = result?;
    if let Some(written) = written {
        written.context("Could not provide credentials to wlanlogin")?;
    }
    ensure!(
        status.success(),
        "wlanlogin operation failed (exit {:?}); see {}",
        status.code(),
        log.display()
    );
    Ok(())
}

fn wait(child: &mut Child, job: &Job, timeout: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            job.terminate()?;
            child.wait()?;
            bail!(
                "wlanlogin exceeded the {} second installation deadline; its process tree was terminated",
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reuse_and_purge_commands_preserve_the_cli_contract() -> Result<()> {
        let install = command(
            Path::new("wlanlogin.exe"),
            Scope::Machine,
            &["service", "install", "--non-interactive"],
            None,
        )?;
        let args: Vec<_> = install.get_args().collect();
        assert_eq!(
            args,
            ["--system", "service", "install", "--non-interactive"]
        );
        let uninstall = command(
            Path::new("wlanlogin.exe"),
            Scope::User,
            &["service", "uninstall", "--purge"],
            None,
        )?;
        assert_eq!(
            uninstall.get_args().collect::<Vec<_>>(),
            ["--user", "service", "uninstall", "--purge"]
        );
        let form = Form {
            username: "test-user".into(),
            password: "test-secret".into(),
            line: "free2".into(),
        };
        let supplied = command(
            Path::new("wlanlogin.exe"),
            Scope::Machine,
            &["service", "install", "--non-interactive"],
            Some(&form),
        )?;
        assert!(
            !supplied
                .get_args()
                .any(|arg| arg == "test-secret" || arg == "--password")
        );
        assert!(supplied.get_args().any(|arg| arg == "--password-stdin"));
        Ok(())
    }
    #[test]
    fn a_stalled_child_is_terminated_at_the_deadline() -> Result<()> {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let job = Job::attach(&child)?;
        let started = Instant::now();
        assert!(wait(&mut child, &job, Duration::from_millis(100)).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(child.try_wait()?.is_some());
        Ok(())
    }
}
