use crate::{
    config::{Paths, private_dir, write_private},
    control,
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const LABEL: &str = "org.seucxcs.cxxy-wlanlogin";

mod status;
use status::Manager;
pub use status::status;
mod lifecycle;
pub use lifecycle::{Action, manage};

#[derive(Clone, Debug)]
struct Invocation {
    program: OsString,
    args: Vec<OsString>,
    optional: bool,
}
impl Invocation {
    fn new(program: impl AsRef<OsStr>, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: args.into_iter().map(|v| v.as_ref().to_owned()).collect(),
            optional: false,
        }
    }
    fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
    async fn run(&self) -> Result<()> {
        let result = self.output().await?;
        if !self.optional {
            result.check(self)?;
        }
        Ok(())
    }
    async fn output(&self) -> Result<CommandOutput> {
        let mut command = tokio::process::Command::new(&self.program);
        command.args(&self.args).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW, available on Win7.
        let result = tokio::time::timeout(Duration::from_secs(30), command.output())
            .await
            .context("服务管理命令超时")??;
        Ok(CommandOutput {
            success: result.status.success(),
            stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        })
    }
}

struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}
impl CommandOutput {
    fn check(&self, command: &Invocation) -> Result<()> {
        if !self.success {
            bail!(
                "{} {:?} 执行失败：{}",
                command.program.to_string_lossy(),
                command.args,
                if self.stderr.trim().is_empty() {
                    self.stdout.trim()
                } else {
                    self.stderr.trim()
                }
            );
        }
        Ok(())
    }
}

struct Plan {
    manager: Manager,
    executable: PathBuf,
    definition: PathBuf,
    content: String,
    stop: Vec<Invocation>,
    start: Vec<Invocation>,
    remove: Vec<Invocation>,
    reload: Vec<Invocation>,
}

pub async fn install(paths: &Paths) -> Result<()> {
    install_plan(paths, plan(paths)?, &std::env::current_exe()?).await
}

async fn install_plan(paths: &Paths, plan: Plan, source: &Path) -> Result<()> {
    let _service_lock = control::service_lock(paths)?;
    assert_owned(&plan.definition)?;
    let source = source.canonicalize()?;
    let binary_dir = plan.executable.parent().context("安装路径无父目录")?;
    private_dir(binary_dir)?;
    if let Some(parent) = plan.definition.parent() {
        fs::create_dir_all(parent)?;
    }
    let old_definition = Backup::capture(&plan.definition)?;
    let old_executable = Backup::capture(&plan.executable)?;
    // Stage the executable before changing a running service.
    let staged = tempfile::NamedTempFile::new_in(binary_dir)?;
    fs::copy(&source, staged.path())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(staged.path(), fs::Permissions::from_mode(0o700))?;
    }
    for command in &plan.stop {
        command.run().await?;
    }
    control::stop(paths).await;
    wait_stopped(paths).await?;
    let outcome: Result<()> = async {
        if source != plan.executable.canonicalize().unwrap_or_default() {
            staged.persist(&plan.executable).map_err(|e| e.error)?;
        }
        write_definition(&plan.definition, plan.content.as_bytes())?;
        for command in &plan.start {
            command.run().await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = outcome {
        // Attempt every rollback step, and report incomplete recovery accurately.
        let mut rollback_errors = Vec::new();
        for command in &plan.remove {
            if let Err(e) = command.run().await {
                rollback_errors.push(e.to_string());
            }
        }
        for result in [
            old_executable.restore(&plan.executable),
            old_definition.restore(&plan.definition),
        ] {
            if let Err(e) = result {
                rollback_errors.push(e.to_string());
            }
        }
        let commands = if old_definition.existed() {
            &plan.start
        } else {
            &plan.reload
        };
        for command in commands {
            if let Err(e) = command.run().await {
                rollback_errors.push(e.to_string());
            }
        }
        if !rollback_errors.is_empty() {
            return Err(error).context(format!(
                "安装失败，回滚仍有错误：{}",
                rollback_errors.join("；")
            ));
        }
        return Err(error).context("安装失败，已回滚安装文件");
    }
    println!("后台服务已安装并启动，今后用户登录系统后自动运行。");
    Ok(())
}

pub async fn uninstall(paths: &Paths) -> Result<()> {
    let _service_lock = control::service_lock(paths)?;
    let plan = plan(paths)?;
    assert_owned(&plan.definition)?;
    if plan.definition.exists() {
        for command in &plan.remove {
            command.run().await?;
        }
    }
    control::stop(paths).await;
    wait_stopped(paths).await?;
    remove_if_present(&plan.definition)?;
    for command in &plan.reload {
        command.run().await?;
    }
    remove_if_present(&plan.executable).context("服务已卸载，但安装副本仍在使用，请关闭后删除")?;
    println!("后台服务已卸载；系统密钥管理器中的账号和密码仍保留。");
    Ok(())
}

async fn wait_stopped(paths: &Paths) -> Result<()> {
    for _ in 0..250 {
        if control::singleton(paths).is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("daemon 尚未退出，安装文件没有被替换；请停止正在运行的 daemon 后重试")
}

fn assert_owned(path: &Path) -> Result<()> {
    match fs::read_to_string(path) {
        Ok(content) if !content.contains(LABEL) => {
            bail!("拒绝覆盖或删除非本工具创建的服务文件：{}", path.display())
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
// Service directories may already contain other applications' files; do not chmod them.
fn write_definition(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("服务文件无父目录")?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
struct Backup(Option<tempfile::NamedTempFile>);
impl Backup {
    fn capture(path: &Path) -> Result<Self> {
        match fs::metadata(path) {
            Ok(_) => {
                let backup =
                    tempfile::NamedTempFile::new_in(path.parent().context("备份路径无父目录")?)?;
                fs::copy(path, backup.path())?;
                Ok(Self(Some(backup)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self(None)),
            Err(e) => Err(e.into()),
        }
    }
    fn existed(&self) -> bool {
        self.0.is_some()
    }
    fn restore(&self, path: &Path) -> Result<()> {
        match &self.0 {
            Some(backup) => {
                fs::copy(backup.path(), path)?;
                Ok(())
            }
            None => remove_if_present(path),
        }
    }
}

fn plan(paths: &Paths) -> Result<Plan> {
    let dirs = directories::ProjectDirs::from("org", "seucxcs", "cxxy-wlanlogin")
        .context("无法确定安装目录")?;
    let executable = dirs.data_local_dir().join("bin").join(if cfg!(windows) {
        "cxxy-wlanlogin-daemon.exe"
    } else {
        "cxxy-wlanlogin-daemon"
    });
    #[cfg(target_os = "linux")]
    {
        let base = directories::BaseDirs::new().context("无法确定用户服务目录")?;
        let _ = paths;
        linux_plan(
            executable,
            base.config_dir()
                .join("systemd/user/cxxy-wlanlogin.service"),
        )
    }
    #[cfg(target_os = "macos")]
    {
        let base = directories::BaseDirs::new().context("无法确定用户服务目录")?;
        let uid = unsafe { libc::geteuid() };
        let log = paths.config.join("daemon.log");
        macos_plan(
            executable,
            base.home_dir()
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
            log,
            uid,
        )
    }
    #[cfg(target_os = "windows")]
    {
        let sid = crate::platform::current_sid()?;
        windows_plan(executable, paths.config.join("daemon-task.xml"), &sid)
    }
}

#[cfg(any(target_os = "linux", test))]
fn linux_plan(executable: PathBuf, definition: PathBuf) -> Result<Plan> {
    let quoted = systemd_quote(&executable)?;
    let content = format!(
        "# {LABEL}\n[Unit]\nDescription=Student_CX automatic login\nStartLimitIntervalSec=0\n\n[Service]\nType=simple\nExecStart={quoted} daemon\nRestart=on-failure\nRestartSec=30\nTimeoutStopSec=25\n\n[Install]\nWantedBy=default.target\n"
    );
    let reload = || Invocation::new("systemctl", ["--user", "daemon-reload"]);
    Ok(Plan {
        manager: Manager::Systemd,
        executable,
        definition,
        content,
        stop: vec![
            Invocation::new("systemctl", ["--user", "stop", "cxxy-wlanlogin.service"]).optional(),
        ],
        start: vec![
            reload(),
            Invocation::new(
                "systemctl",
                ["--user", "enable", "--now", "cxxy-wlanlogin.service"],
            ),
            Invocation::new(
                "systemctl",
                ["--user", "is-active", "--quiet", "cxxy-wlanlogin.service"],
            ),
        ],
        remove: vec![Invocation::new(
            "systemctl",
            ["--user", "disable", "--now", "cxxy-wlanlogin.service"],
        )],
        reload: vec![reload()],
    })
}
#[cfg(any(target_os = "linux", test))]
fn systemd_quote(path: &Path) -> Result<String> {
    let text = path.to_str().context("systemd 路径必须是 UTF-8")?;
    if text.chars().any(char::is_control) {
        bail!("安装路径包含控制字符");
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
#[cfg(any(target_os = "macos", test))]
fn macos_plan(executable: PathBuf, definition: PathBuf, log: PathBuf, uid: u32) -> Result<Plan> {
    let exe = xml(executable.to_str().context("安装路径必须是 UTF-8")?);
    let log = xml(log.to_str().context("日志路径必须是 UTF-8")?);
    let domain = format!("gui/{uid}");
    let target = format!("{domain}/{LABEL}");
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{exe}</string><string>daemon</string></array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>30</integer>
<key>StandardErrorPath</key><string>{log}</string>
</dict></plist>
"#
    );
    let bootout =
        || Invocation::new("launchctl", [OsStr::new("bootout"), OsStr::new(&target)]).optional();
    Ok(Plan {
        manager: Manager::Launchd {
            domain: domain.clone(),
            target: target.clone(),
        },
        executable,
        content,
        stop: vec![bootout()],
        remove: vec![
            bootout(),
            Invocation::new("launchctl", ["disable", &target]),
        ],
        reload: vec![],
        start: vec![
            Invocation::new("launchctl", [OsStr::new("enable"), OsStr::new(&target)]),
            Invocation::new(
                "launchctl",
                [
                    OsStr::new("bootstrap"),
                    OsStr::new(&domain),
                    definition.as_os_str(),
                ],
            ),
        ],
        definition,
    })
}
#[cfg(any(target_os = "windows", test))]
fn windows_plan(executable: PathBuf, definition: PathBuf, sid: &str) -> Result<Plan> {
    let exe = xml(executable.to_str().context("安装路径必须是 UTF-8")?);
    let name = format!("cxxy-wlanlogin-{sid}");
    let sid = xml(sid);
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- {LABEL} -->
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<RegistrationInfo><Description>Student_CX automatic login</Description></RegistrationInfo>
<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger></Triggers>
<Principals><Principal id="User"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><AllowStartOnDemand>true</AllowStartOnDemand><Enabled>true</Enabled><Hidden>true</Hidden><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings>
<Actions Context="User"><Exec><Command>{exe}</Command><Arguments>daemon --background</Arguments></Exec></Actions>
</Task>
"#
    );
    Ok(Plan {
        manager: Manager::TaskScheduler { name: name.clone() },
        executable,
        content,
        stop: vec![Invocation::new("schtasks", ["/End", "/TN", &name]).optional()],
        start: vec![
            Invocation::new(
                "schtasks",
                [
                    OsStr::new("/Create"),
                    OsStr::new("/TN"),
                    OsStr::new(&name),
                    OsStr::new("/XML"),
                    definition.as_os_str(),
                    OsStr::new("/F"),
                ],
            ),
            Invocation::new("schtasks", ["/Run", "/TN", &name]),
        ],
        remove: vec![
            Invocation::new("schtasks", ["/End", "/TN", &name]).optional(),
            Invocation::new("schtasks", ["/Delete", "/TN", &name, "/F"]),
        ],
        reload: vec![],
        definition,
    })
}

pub fn background_log(paths: &Paths) -> Result<fs::File> {
    let path = paths.config.join("daemon.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
        write_private(&path, b"")?;
    }
    private_dir(&paths.config)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_services_use_user_sessions_without_credentials() -> Result<()> {
        let executable = PathBuf::from("/home/Test User/bin/cxxy");
        let definition = PathBuf::from("/tmp/definition");
        let plans = [
            linux_plan(executable.clone(), definition.clone())?,
            macos_plan(
                executable.clone(),
                definition.clone(),
                "/tmp/daemon.log".into(),
                501,
            )?,
            windows_plan(executable, definition, "S-1-5-21-123")?,
        ];
        for plan in &plans {
            assert!(plan.content.contains("daemon"));
            assert!(!plan.content.contains("--password"));
            assert!(!plan.content.contains("CXXY_WLAN_PASSWORD"));
            assert!(plan.content.contains(LABEL));
        }
        assert!(plans[0].content.contains("WantedBy=default.target"));
        assert!(plans[1].content.contains("<key>RunAtLoad</key><true/>"));
        assert!(
            plans[2]
                .content
                .contains("<LogonType>InteractiveToken</LogonType>")
        );
        assert!(
            plans[2]
                .content
                .contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>")
        );
        Ok(())
    }
    #[test]
    fn service_paths_are_escaped_and_unrelated_files_are_preserved() -> Result<()> {
        assert_eq!(
            systemd_quote(Path::new("/tmp/a %$\"b"))?,
            "\"/tmp/a %%$$\\\"b\""
        );
        assert!(systemd_quote(Path::new("/tmp/a\nb")).is_err());
        assert_eq!(xml("a<&\""), "a&lt;&amp;&quot;");
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("service");
        fs::write(&file, "a different service")?;
        assert!(assert_owned(&file).is_err());
        assert_eq!(fs::read_to_string(file)?, "a different service");
        Ok(())
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn failed_registration_restores_the_previous_service_and_binary() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths {
            config: dir.path().join("config"),
        };
        let executable = dir.path().join("daemon");
        let definition = dir.path().join("service");
        let original = dir.path().join("expected");
        let source = dir.path().join("new-binary");
        fs::write(&executable, "old executable")?;
        fs::write(&definition, format!("# {LABEL} old service"))?;
        fs::copy(&definition, &original)?;
        fs::write(&source, "new executable")?;
        let plan = Plan {
            manager: Manager::Systemd,
            executable: executable.clone(),
            definition: definition.clone(),
            content: format!("# {LABEL} new service"),
            stop: vec![],
            remove: vec![],
            reload: vec![],
            // Fails for the new definition, succeeds after the old one is restored.
            start: vec![Invocation::new(
                "cmp",
                [original.as_os_str(), definition.as_os_str()],
            )],
        };
        assert!(install_plan(&paths, plan, &source).await.is_err());
        assert_eq!(fs::read_to_string(executable)?, "old executable");
        assert_eq!(fs::read(definition)?, fs::read(original)?);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_first_install_removes_new_artifacts() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths {
            config: dir.path().join("config"),
        };
        let source = dir.path().join("source");
        fs::write(&source, "test binary, never executed")?;
        let executable = dir.path().join("bin/daemon");
        let definition = dir.path().join("service");
        let plan = Plan {
            manager: Manager::Systemd,
            executable: executable.clone(),
            definition: definition.clone(),
            content: format!("# {LABEL}"),
            stop: vec![],
            remove: vec![],
            reload: vec![],
            start: vec![Invocation::new("false", std::iter::empty::<&str>())],
        };
        assert!(install_plan(&paths, plan, &source).await.is_err());
        assert!(!executable.exists());
        assert!(!definition.exists());
        Ok(())
    }

    #[test]
    fn backups_restore_existing_files_and_remove_new_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("file");
        let absent = Backup::capture(&file)?;
        fs::write(&file, "original")?;
        let backup = Backup::capture(&file)?;
        fs::write(&file, "replacement")?;
        backup.restore(&file)?;
        assert_eq!(fs::read_to_string(&file)?, "original");
        absent.restore(&file)?;
        assert!(!file.exists());
        Ok(())
    }
}
