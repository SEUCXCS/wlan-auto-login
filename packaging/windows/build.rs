use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(setup_payload)");
    for name in [
        "WLANLOGIN_USER_MSI",
        "WLANLOGIN_MACHINE_MSI",
        "WLANLOGIN_ICON",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=setup.manifest");
    if env::var_os("WLANLOGIN_USER_MSI").is_some() && env::var_os("WLANLOGIN_MACHINE_MSI").is_some()
    {
        println!("cargo:rustc-cfg=setup_payload");
        for name in ["WLANLOGIN_USER_MSI", "WLANLOGIN_MACHINE_MSI"] {
            println!(
                "cargo:rerun-if-changed={}",
                env::var(name).expect("MSI path")
            );
        }
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(icon) = env::var_os("WLANLOGIN_ICON") else {
        return;
    };
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let source = out.join("setup.rc");
    fs::write(
        &source,
        format!(
            "101 ICON \"{}\"\n",
            PathBuf::from(icon)
                .display()
                .to_string()
                .replace('\\', "\\\\")
        ),
    )
    .expect("write icon resource");
    let sdk = PathBuf::from(env::var_os("ProgramFiles(x86)").expect("Windows SDK location"))
        .join("Windows Kits/10/bin");
    let mut versions: Vec<_> = fs::read_dir(sdk)
        .expect("Windows SDK")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("x64/rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    versions.sort();
    let rc = versions
        .last()
        .expect("Install the Windows SDK resource compiler");
    let resource = out.join("setup.res");
    assert!(
        Command::new(rc)
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg(source)
            .status()
            .expect("rc.exe")
            .success()
    );
    println!(
        "cargo:rustc-link-arg-bin=wlanlogin-setup={}",
        resource.display()
    );
    println!("cargo:rustc-link-arg-bin=wlanlogin-setup=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin=wlanlogin-setup=/MANIFESTINPUT:{}",
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
            .join("setup.manifest")
            .display()
    );
}
