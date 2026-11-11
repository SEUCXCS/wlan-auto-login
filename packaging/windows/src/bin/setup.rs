#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(all(windows, setup_payload))]
fn main() {
    if let Err(error) = wlanlogin_installer::ui::run(
        include_bytes!(env!("WLANLOGIN_USER_MSI")),
        include_bytes!(env!("WLANLOGIN_MACHINE_MSI")),
    ) {
        wlanlogin_installer::win::message(&format!("安装失败。\n{error:#}"), true);
        std::process::exit(1);
    }
}

#[cfg(not(all(windows, setup_payload)))]
fn main() {
    eprintln!("Build the setup executable using cargo xtask bundle windows.");
    std::process::exit(1);
}
