//! Select scope with the Windows task-dialog template, then show the official WixUI wizard.
use crate::{Scope, win};
use anyhow::{Result, ensure};
use std::ptr;
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::{
        ApplicationInstallationAndServicing::{
            INSTALLUILEVEL_FULL, MsiInstallProductW, MsiSetInternalUI,
        },
        LibraryLoader::GetModuleHandleW,
        Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
    },
    UI::{
        Controls::{
            TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDCBF_CANCEL_BUTTON,
            TDF_ALLOW_DIALOG_CANCELLATION, TDF_USE_COMMAND_LINKS, TaskDialogIndirect,
        },
        Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::{
            HWND_BROADCAST, IDCANCEL, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, SendMessageTimeoutW,
            WM_SETTINGCHANGE,
        },
    },
};

pub fn run(user_msi: &'static [u8], machine_msi: &'static [u8]) -> Result<()> {
    let scope = if std::env::args().any(|arg| arg == "--machine") || win::elevated()? {
        Scope::Machine
    } else {
        let Some(scope) = choose_scope()? else {
            std::process::exit(1602);
        };
        scope
    };
    if scope == Scope::Machine && !win::elevated()? {
        return elevate();
    }
    let temporary = if scope == Scope::Machine {
        tempfile::Builder::new()
            .prefix(".wlanlogin-setup-")
            .tempdir_in(win::program_files()?)?
    } else {
        tempfile::Builder::new()
            .prefix("wlanlogin-setup-")
            .tempdir()?
    };
    win::private_directory(temporary.path(), scope == Scope::Machine)?;
    let msi = temporary.path().join("wlanlogin.msi");
    std::fs::write(
        &msi,
        if scope == Scope::Machine {
            machine_msi
        } else {
            user_msi
        },
    )?;
    let result = unsafe {
        MsiSetInternalUI(INSTALLUILEVEL_FULL, ptr::null_mut());
        MsiInstallProductW(
            win::wide(msi).as_ptr(),
            win::wide("REBOOT=ReallySuppress").as_ptr(),
        )
    };
    if result == 1602 {
        drop(temporary);
        std::process::exit(1602);
    }
    ensure!(
        result == 0 || result == 3010,
        "Windows Installer returned {result}. Recovery details, if needed, are in the installation directory's .installer-* folder."
    );
    unsafe {
        let mut ignored = 0;
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            win::wide("Environment").as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &mut ignored,
        );
    }
    Ok(())
}

fn choose_scope() -> Result<Option<Scope>> {
    let title = win::wide("CXCS WLAN Login 安装");
    let heading = win::wide("为谁安装 CXCS WLAN Login？");
    let user = win::wide("仅当前用户\n登录 Windows 后自动启动，无需管理员权限。");
    let machine = win::wide("此计算机上的所有用户\n开机自动启动，需要管理员授权。");
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: 100,
            pszButtonText: user.as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 101,
            pszButtonText: machine.as_ptr(),
        },
    ];
    let configuration = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hInstance: unsafe { GetModuleHandleW(ptr::null()) },
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_USE_COMMAND_LINKS,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: heading.as_ptr(),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: 101usize as *const u16,
        },
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: 100,
        ..Default::default()
    };
    let mut selected = 0;
    let result = unsafe {
        TaskDialogIndirect(
            &configuration,
            &mut selected,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    ensure!(
        result >= 0,
        "Could not open installation scope selector ({result:#x})"
    );
    match selected {
        100 => Ok(Some(Scope::User)),
        101 => Ok(Some(Scope::Machine)),
        IDCANCEL => Ok(None),
        _ => anyhow::bail!("Unexpected scope selection"),
    }
}

fn elevate() -> Result<()> {
    unsafe {
        let executable = win::wide(std::env::current_exe()?);
        let verb = win::wide("runas");
        let arguments = win::wide("--machine");
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of_val(&info) as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = executable.as_ptr();
        info.lpParameters = arguments.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        win::check(ShellExecuteExW(&mut info))?;
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        std::process::exit(code as i32);
    }
}
