//! Launch the signed desktop host with the application's internal Wi-Fi entry.
use super::*;
pub(super) fn host_launch(
    app: &Path,
    scope: Scope,
    uid: u32,
    interactive: bool,
) -> crate::infra::command::Invocation {
    use crate::infra::command::Invocation;
    use std::ffi::OsString;
    let mut args: Vec<OsString> = vec!["-n".into()];
    if !interactive {
        args.push("-g".into());
    }
    args.extend(["-a".into(), app.as_os_str().to_owned(), "--args".into()]);
    args.extend(["daemon".into(), "--internal-wifi-host".into()]);
    if scope == Scope::System {
        args.push("--system-helper".into());
        let mut command: Vec<OsString> = vec![
            "asuser".into(),
            uid.to_string().into(),
            "/usr/bin/sudo".into(),
            "-u".into(),
            format!("#{uid}").into(),
            "/usr/bin/open".into(),
        ];
        command.extend(args);
        Invocation::new("/bin/launchctl", command)
    } else {
        Invocation::new("/usr/bin/open", args)
    }
}

pub(super) async fn launch_host(
    app: &Path,
    scope: Scope,
    uid: u32,
    interactive: bool,
) -> Result<()> {
    launch(
        app,
        scope,
        uid,
        interactive,
        &crate::infra::command::SystemRunner,
    )
    .await
}

pub(super) async fn launch(
    app: &Path,
    scope: Scope,
    uid: u32,
    interactive: bool,
    runner: &impl crate::infra::command::Runner,
) -> Result<()> {
    let command = host_launch(app, scope, uid, interactive);
    runner.output(&command).await?.check(&command)
}
