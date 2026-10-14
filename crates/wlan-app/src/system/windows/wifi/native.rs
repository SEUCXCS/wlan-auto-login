use crate::infra::error::Message;
use crate::infra::error::{Result, bail};
use crate::system::{define::wifi::TARGET_SSID, shared::wifi::Worker};
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

struct NativeMonitor {
    handle: HANDLE,
    events: Arc<Events>,
}

pub struct Monitor {
    worker: Worker<Option<String>>,
    events: Arc<Events>,
}

impl Monitor {
    pub async fn new() -> Result<Self> {
        let events = Arc::new(Events {
            notify: Notify::new(),
            generation: AtomicU64::new(0),
        });
        let notifications = events.clone();
        let worker = Worker::new(move || {
            let mut native = None;
            move || {
                let monitor = match &mut native {
                    Some(monitor) => monitor,
                    slot @ None => slot.insert(NativeMonitor::new(notifications.clone())?),
                };
                monitor.snapshot()
            }
        })?;
        Ok(Self { worker, events })
    }

    pub async fn snapshot(&self) -> Result<Option<String>> {
        self.worker.query().await
    }

    pub async fn changed(&mut self) -> Result<()> {
        self.events.notify.notified().await;
        Ok(())
    }
}

impl NativeMonitor {
    fn new(events: Arc<Events>) -> Result<Self> {
        let mut handle = ptr::null_mut();
        let mut version = 0;
        // SAFETY: the API initializes the output handle/version; pointers live for the call.
        let result = unsafe { WlanOpenHandle(2, ptr::null(), &mut version, &mut handle) };
        if result != 0 {
            bail!(Message::WlanOpenHandleFailedResult {
                result: (result).to_string()
            });
        }
        let monitor = Self { handle, events };
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
            bail!(Message::FailedToSubscribeToWLANEventsResult {
                result: (result).to_string()
            });
        }
        Ok(monitor)
    }

    fn snapshot(&self) -> Result<Option<String>> {
        // SAFETY: all allocations returned by WLAN are freed via WlanFreeMemory;
        // arrays are bounded by the counts supplied by the API.
        unsafe {
            let mut list = ptr::null_mut();
            let code = WlanEnumInterfaces(self.handle, ptr::null(), &mut list);
            if code != 0 {
                bail!(Message::FailedToEnumerateWiFiInterfacesCode {
                    code: (code).to_string()
                });
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
                    bail!(Message::FailedToReadWiFiSSIDCheck {
                        code: (code).to_string()
                    });
                }
                let _data = WlanMemory(data);
                if (size as usize) < std::mem::size_of::<WLAN_CONNECTION_ATTRIBUTES>() {
                    bail!(Message::InvalidWLANResponseLength);
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
}

impl Drop for NativeMonitor {
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
impl crate::system::define::wifi::WifiConnection for Monitor {
    async fn target_session(&self) -> Result<Option<String>> {
        self.snapshot().await
    }
}
impl crate::system::define::wifi::WifiMonitor for Monitor {
    async fn changed(&mut self) -> Result<()> {
        Monitor::changed(self).await
    }
}
