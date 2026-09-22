use super::TARGET_SSID;
use anyhow::{Context, Result, bail};
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2_core_foundation::{CFArray, CFRetained, CFString};
use objc2_core_wlan::{CWInterfaceMode, CWWiFiClient};
use objc2_system_configuration::{SCDynamicStore, SCDynamicStoreContext};
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::Notify;

struct Events {
    notify: Notify,
    generation: AtomicU64,
    observed: Mutex<Option<String>>,
}
pub struct Monitor {
    store: CFRetained<SCDynamicStore>,
    _queue: DispatchRetained<DispatchQueue>,
    events: Arc<Events>,
}
impl Monitor {
    pub async fn new() -> Result<Self> {
        let events = Arc::new(Events {
            notify: Notify::new(),
            generation: AtomicU64::new(0),
            observed: Mutex::new(None),
        });
        let mut context = SCDynamicStoreContext {
            version: 0,
            info: Arc::as_ptr(&events).cast_mut().cast(),
            retain: Some(retain),
            release: Some(release),
            copyDescription: None,
        };
        // SAFETY: callbacks retain/release the Arc allocation; all patterns are CFStrings.
        let store = unsafe {
            SCDynamicStore::new(
                None,
                &CFString::from_str("cxxy-wlanlogin"),
                Some(changed),
                &mut context,
            )
        }
        .context("无法创建系统网络监听")?;
        let patterns = CFArray::from_retained_objects(&[
            CFString::from_str("State:/Network/Interface/.*/AirPort"),
            CFString::from_str("State:/Network/Interface/.*/Link"),
            CFString::from_str("State:/Network/Service/.*/IPv4"),
        ]);
        let queue = DispatchQueue::new("org.seucxcs.cxxy-wlanlogin.network", None);
        unsafe {
            if !store.set_notification_keys(None, Some(patterns.as_opaque()))
                || !store.set_dispatch_queue(Some(&queue))
            {
                bail!("无法订阅系统网络事件");
            }
        }
        Ok(Self {
            store,
            _queue: queue,
            events,
        })
    }
    pub async fn snapshot(&self) -> Result<Option<String>> {
        let identity = current_interface()?;
        self.events.observe(&identity);
        Ok(identity
            .map(|name| format!("{name}#{}", self.events.generation.load(Ordering::Relaxed))))
    }
    pub async fn changed(&mut self) -> Result<()> {
        self.events.notify.notified().await;
        Ok(())
    }
}
impl Events {
    fn observe(&self, identity: &Option<String>) {
        if let Ok(mut previous) = self.observed.lock()
            && *previous != *identity
        {
            self.generation.fetch_add(1, Ordering::Relaxed);
            *previous = identity.clone();
        }
    }
}
fn current_interface() -> Result<Option<String>> {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: CoreWLAN getters have no additional preconditions and retained
        // interfaces remain alive throughout this autorelease pool.
        unsafe {
            let client = CWWiFiClient::sharedWiFiClient();
            let Some(interfaces) = client.interfaces() else {
                return Ok(None);
            };
            for interface in interfaces.iter() {
                let ssid = interface.ssid();
                if let Some(ssid) = ssid {
                    if ssid.to_string().as_bytes() == TARGET_SSID {
                        let name = interface
                            .interfaceName()
                            .context("无法读取 Wi-Fi 接口名称")?;
                        return Ok(Some(name.to_string()));
                    }
                } else if interface.interfaceMode() == CWInterfaceMode::Station {
                    bail!("无法读取 Wi-Fi SSID；请在系统设置中授予位置权限后重试");
                }
            }
            Ok(None)
        }
    })
}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe {
            self.store.set_dispatch_queue(None);
        }
    }
}
unsafe extern "C-unwind" fn retain(info: NonNull<c_void>) -> NonNull<c_void> {
    unsafe {
        Arc::increment_strong_count(info.as_ptr().cast::<Events>());
    }
    info
}
unsafe extern "C-unwind" fn release(info: NonNull<c_void>) {
    unsafe {
        Arc::decrement_strong_count(info.as_ptr().cast::<Events>());
    }
}
unsafe extern "C-unwind" fn changed(
    _: NonNull<SCDynamicStore>,
    _: NonNull<CFArray>,
    info: *mut c_void,
) {
    if info.is_null() {
        return;
    }
    let events = unsafe { &*info.cast::<Events>() };
    if let Ok(identity) = current_interface() {
        events.observe(&identity);
    }
    events.notify.notify_one();
}
