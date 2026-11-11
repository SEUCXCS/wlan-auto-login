#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--help") {
        println!("Internal Windows Installer action. Use wlanlogin-setup.exe to install.");
        return;
    }
    #[cfg(windows)]
    if let Err(error) = execute() {
        // Errors contain stages and native result codes, never child output or credentials.
        eprintln!("Windows installation action failed: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn execute() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--verify-package")
    {
        return wlanlogin_installer::verify::run(&arguments[1..]);
    }
    let result = wlanlogin_installer::transaction::dispatch(arguments.clone());
    if let Err(error) = &result {
        wlanlogin_installer::diagnostics::report_error(&arguments, error);
    }
    result
}
