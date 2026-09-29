//! Native SCM process entry. All used APIs are available on Windows 7.
use std::{
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicPtr, Ordering},
    },
};
use windows_sys::Win32::System::Services::*;

static HANDLE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(ptr::null_mut());
static STOP: OnceLock<tokio::sync::Notify> = OnceLock::new();

pub async fn stop_signal() {
    STOP.get_or_init(Default::default).notified().await;
}

pub fn dispatch() -> std::process::ExitCode {
    let mut name: Vec<u16> = "cxxy-wlanlogin".encode_utf16().chain(Some(0)).collect();
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: ptr::null_mut(),
            lpServiceProc: None,
        },
    ];
    // SAFETY: table/name live throughout the blocking dispatcher call.
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        let _ = crate::infra::logging::init(None);
        tracing::error!(
            "无法连接 Windows 服务管理器：{}；直接运行请使用 daemon",
            std::io::Error::last_os_error()
        );
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

unsafe extern "system" fn service_main(_: u32, _: *mut windows_sys::core::PWSTR) {
    let name: Vec<u16> = "cxxy-wlanlogin".encode_utf16().chain(Some(0)).collect();
    // SAFETY: SCM owns callbacks for this process and name remains valid during registration.
    let handle =
        unsafe { RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), ptr::null()) };
    if handle.is_null() {
        return;
    }
    HANDLE.store(handle, Ordering::Release);
    STOP.get_or_init(Default::default);
    report(SERVICE_START_PENDING, 0);
    let result = std::panic::catch_unwind(|| {
        let runtime = tokio::runtime::Runtime::new()?;
        report(SERVICE_RUNNING, 0);
        let result = runtime.block_on(crate::app::run_service());
        runtime.shutdown_timeout(crate::infra::shutdown::RUNTIME_GRACE);
        Ok::<_, anyhow::Error>(result)
    });
    let code = match result {
        Ok(Ok(code)) if code == std::process::ExitCode::SUCCESS => 0,
        _ => 1,
    };
    report(SERVICE_STOPPED, code);
    HANDLE.store(ptr::null_mut(), Ordering::Release);
}

unsafe extern "system" fn handler(
    control: u32,
    _: u32,
    _: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            report(SERVICE_STOP_PENDING, 0);
            STOP.get_or_init(Default::default).notify_one();
            0
        }
        SERVICE_CONTROL_INTERROGATE => 0,
        _ => 120, // ERROR_CALL_NOT_IMPLEMENTED
    }
}

fn report(state: u32, code: u32) {
    let handle = HANDLE.load(Ordering::Acquire);
    if handle.is_null() {
        return;
    }
    let pending = state == SERVICE_START_PENDING || state == SERVICE_STOP_PENDING;
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            0
        },
        dwWin32ExitCode: if code == 0 { 0 } else { 1066 }, // ERROR_SERVICE_SPECIFIC_ERROR
        dwServiceSpecificExitCode: code,
        dwCheckPoint: u32::from(pending),
        dwWaitHint: if pending { 30_000 } else { 0 },
    };
    // SAFETY: handle was registered with SCM and status has the documented layout.
    unsafe {
        SetServiceStatus(handle, &status);
    }
}
