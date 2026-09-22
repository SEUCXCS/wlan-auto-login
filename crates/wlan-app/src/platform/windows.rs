use super::TARGET_SSID;
use anyhow::{Result, bail};
use std::{
    ffi::c_void,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::Notify;
use windows_sys::Win32::{Foundation::HANDLE, NetworkManagement::WiFi::*};

struct Events {
    notify: Notify,
    generation: AtomicU64,
}

pub struct Monitor {
    handle: HANDLE,
    events: Arc<Events>,
}

impl Monitor {
    pub async fn new() -> Result<Self> {
        let mut handle = ptr::null_mut();
        let mut version = 0;
        // SAFETY: the API initializes the output handle/version; pointers live for the call.
        let result = unsafe { WlanOpenHandle(2, ptr::null(), &mut version, &mut handle) };
        if result != 0 {
            bail!("WlanOpenHandle 失败: {result}");
        }
        let monitor = Self {
            handle,
            events: Arc::new(Events {
                notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
        };
        // SAFETY: the Arc allocation remains alive until WlanCloseHandle has drained callbacks.
        let result = unsafe {
            WlanRegisterNotification(
                handle,
                WLAN_NOTIFICATION_SOURCE_ACM,
                1,
                Some(changed),
                Arc::as_ptr(&monitor.events).cast(),
                ptr::null(),
                ptr::null_mut(),
            )
        };
        if result != 0 {
            bail!("无法监听 WLAN 事件: {result}");
        }
        Ok(monitor)
    }

    pub async fn snapshot(&self) -> Result<Option<String>> {
        // SAFETY: all allocations returned by WLAN are freed via WlanFreeMemory;
        // arrays are bounded by the counts supplied by the API.
        unsafe {
            let mut list = ptr::null_mut();
            let code = WlanEnumInterfaces(self.handle, ptr::null(), &mut list);
            if code != 0 {
                bail!("无法枚举 Wi-Fi 接口: {code}");
            }
            let allocation = WlanMemory(list.cast());
            let interfaces = std::slice::from_raw_parts(
                (*list).InterfaceInfo.as_ptr(),
                (*list).dwNumberOfItems as usize,
            );
            for interface in interfaces {
                if interface.isState != wlan_interface_state_connected {
                    continue;
                }
                let mut size = 0;
                let mut data = ptr::null_mut();
                let code = WlanQueryInterface(
                    self.handle,
                    &interface.InterfaceGuid,
                    wlan_intf_opcode_current_connection,
                    ptr::null(),
                    &mut size,
                    &mut data,
                    ptr::null_mut(),
                );
                if code != 0 {
                    bail!("无法读取 Wi-Fi SSID（请检查位置权限）: {code}");
                }
                let _data = WlanMemory(data);
                if (size as usize) < std::mem::size_of::<WLAN_CONNECTION_ATTRIBUTES>() {
                    bail!("WLAN 响应长度无效");
                }
                let attributes = &*data.cast::<WLAN_CONNECTION_ATTRIBUTES>();
                let ssid = &attributes.wlanAssociationAttributes.dot11Ssid;
                if ssid.uSSIDLength as usize > ssid.ucSSID.len() {
                    continue;
                }
                if &ssid.ucSSID[..ssid.uSSIDLength as usize] == TARGET_SSID {
                    let guid = interface.InterfaceGuid;
                    let id = format!(
                        "{:08x}-{:04x}-{:04x}-{:02x?}#{}",
                        guid.data1,
                        guid.data2,
                        guid.data3,
                        guid.data4,
                        self.events.generation.load(Ordering::Relaxed)
                    );
                    drop(allocation);
                    return Ok(Some(id));
                }
            }
            Ok(None)
        }
    }
    pub async fn changed(&mut self) -> Result<()> {
        self.events.notify.notified().await;
        Ok(())
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: this is our live handle. Closing outside a callback waits for callbacks.
        unsafe {
            WlanCloseHandle(self.handle, ptr::null());
        }
    }
}
struct WlanMemory(*mut c_void);
impl Drop for WlanMemory {
    fn drop(&mut self) {
        unsafe {
            WlanFreeMemory(self.0);
        }
    }
}

unsafe extern "system" fn changed(data: *mut L2_NOTIFICATION_DATA, context: *mut c_void) {
    if data.is_null() || context.is_null() {
        return;
    }
    // SAFETY: WLAN owns data for this call; the monitor keeps the Events allocation alive.
    let (data, events) = unsafe { (&*data, &*context.cast::<Events>()) };
    if data.NotificationSource != WLAN_NOTIFICATION_SOURCE_ACM {
        return;
    }
    let code = data.NotificationCode as i32;
    if code == wlan_notification_acm_connection_complete
        || code == wlan_notification_acm_disconnected
    {
        events.generation.fetch_add(1, Ordering::Relaxed);
        events.notify.notify_one();
    }
}

pub fn current_sid() -> Result<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    // SAFETY: the token and SID buffers live through each API call; the word
    // buffer is pointer-aligned and all Windows allocations/handles are freed.
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let result = (|| {
            let mut size = 0;
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
            let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
            if GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut string = ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut string) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut length = 0;
            while *string.add(length) != 0 {
                length += 1;
            }
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(string, length));
            LocalFree(string.cast());
            Ok(sid)
        })();
        CloseHandle(token);
        result
    }
}
