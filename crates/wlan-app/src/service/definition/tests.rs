use super::linux::systemd_quote;
use super::*;

#[test]
fn system_services_boot_without_a_user_session() -> Result<()> {
    let exe = PathBuf::from("/opt/Test App/daemon");
    let definition = PathBuf::from("/tmp/system-definition");
    let linux = linux_system_plan(exe.clone(), definition.clone())?;
    assert!(linux.content.contains("--system daemon"));
    assert!(linux.content.contains("daemon --background"));
    assert!(linux.content.contains("# ApplicationLog=daemon.log"));
    assert!(linux.content.contains("WantedBy=multi-user.target"));
    assert!(!linux.content.contains("--user"));
    let macos = macos_system_plan(exe.clone(), definition.clone(), "/tmp/log".into())?;
    assert!(macos.content.contains("<string>--system</string>"));
    assert!(
        macos
            .start
            .iter()
            .any(|cmd| cmd.args.iter().any(|a| a == "system"))
    );
    assert!(!macos.start.iter().any(|cmd| {
        cmd.args
            .iter()
            .any(|a| a.to_string_lossy().contains("gui/"))
    }));
    let windows = windows_system_plan(exe, definition)?;
    assert!(windows.start.iter().any(|cmd| {
        cmd.args
            .iter()
            .any(|a| a.to_string_lossy().contains("--system daemon --service"))
    }));
    assert!(
        windows
            .start
            .iter()
            .any(|cmd| cmd.args.iter().any(|a| a == "auto"))
    );
    assert!(matches!(windows.manager, Manager::WindowsService));
    for plan in [linux, macos, windows] {
        assert_eq!(plan.scope, crate::scope::Scope::System);
        assert!(plan.content.contains(LABEL));
        assert!(!plan.content.contains("user_password"));
    }
    Ok(())
}
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
    assert!(plans[0].content.contains("daemon --background"));
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
