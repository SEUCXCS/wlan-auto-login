//! Core Location objects and their delegate live on one continuously pumped run loop.
use crate::infra::error::{Context, Message, Result};
use objc2::{
    AnyThread, DefinedClass, define_class, msg_send, rc::Retained, runtime::ProtocolObject,
};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
use objc2_core_location::{
    CLAuthorizationStatus, CLLocation, CLLocationManager, CLLocationManagerDelegate,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

fn activate() {
    // NSRunningApplication supports calls from any thread. Bring only this
    // application forward for an explicitly interactive authorization request.
    unsafe {
        let app: Retained<objc2::runtime::AnyObject> =
            msg_send![objc2::class!(NSRunningApplication), currentApplication];
        let _: bool = msg_send![&*app, activateWithOptions: 2usize];
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    NotDetermined,
    Granted,
    Denied,
    Restricted,
    Disabled,
    Unknown,
}

impl Status {
    pub fn check(self) -> Result<()> {
        crate::infra::error::ensure!(
            self == Self::Granted,
            Message::LocationUnavailable {
                state: format!("{self:?}")
            }
        );
        Ok(())
    }
}

struct State {
    status: watch::Sender<Status>,
    request: AtomicBool,
    stop: AtomicBool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "WlanLoginLocationDelegate"]
    #[ivars = Arc<State>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl CLLocationManagerDelegate for Delegate {
        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn changed(&self, manager: &CLLocationManager) {
            self.ivars().status.send_replace(read(manager));
        }
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn updated(&self, _manager: &CLLocationManager, _locations: &NSArray<CLLocation>) {}
        #[unsafe(method(locationManager:didFailWithError:))]
        fn failed(&self, manager: &CLLocationManager, _error: &NSError) {
            self.ivars().status.send_replace(read(manager));
        }
    }
);

pub struct Location {
    state: Arc<State>,
    status: watch::Receiver<Status>,
}

impl Location {
    pub fn new() -> Result<Self> {
        let (sender, status) = watch::channel(Status::Unknown);
        let state = Arc::new(State {
            status: sender,
            request: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let worker = state.clone();
        std::thread::Builder::new()
            .name("location-runloop".into())
            .spawn(move || {
                objc2::rc::autoreleasepool(|_| unsafe {
                    let manager = CLLocationManager::new();
                    let allocated = Delegate::alloc().set_ivars(worker.clone());
                    let delegate: Retained<Delegate> = msg_send![super(allocated), init];
                    manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                    worker.status.send_replace(read(&manager));
                    let mut updating = false;
                    while !worker.stop.load(Ordering::Acquire) {
                        let status = read(&manager);
                        let request = worker.request.swap(false, Ordering::AcqRel);
                        if request && status == Status::NotDetermined {
                            manager.requestWhenInUseAuthorization();
                        }
                        // macOS CLI-style hosts need an active Location client for CoreWLAN.
                        // No coordinates are stored, returned over IPC or written to logs.
                        if !updating
                            && (status == Status::Granted
                                || (request && status == Status::NotDetermined))
                        {
                            manager.startUpdatingLocation();
                            updating = true;
                        } else if updating
                            && matches!(
                                status,
                                Status::Denied | Status::Restricted | Status::Disabled
                            )
                        {
                            manager.stopUpdatingLocation();
                            updating = false;
                        }
                        worker.status.send_if_modified(|previous| {
                            if *previous == status {
                                false
                            } else {
                                *previous = status;
                                true
                            }
                        });
                        CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, 0.1, false);
                        // A run loop without sources may return immediately.
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    manager.stopUpdatingLocation();
                    manager.setDelegate(None);
                });
            })
            .context(Message::LocationRunLoopFailed)?;
        Ok(Self { state, status })
    }

    pub async fn ready(&self) -> Result<()> {
        let mut receiver = self.status.clone();
        tokio::time::timeout(Duration::from_secs(3), async {
            while *receiver.borrow_and_update() == Status::Unknown {
                receiver
                    .changed()
                    .await
                    .context(Message::LocationManagerStopped)?;
            }
            Ok::<_, crate::infra::error::Error>(())
        })
        .await
        .context(Message::MacosHostUnavailable)?
    }

    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    pub async fn authorize(&self) -> Result<Status> {
        let mut receiver = self.status.clone();
        tokio::time::timeout(Duration::from_secs(45), async {
            // Initialization and callbacks both run on the manager's owning thread.
            while *receiver.borrow_and_update() == Status::Unknown {
                receiver
                    .changed()
                    .await
                    .context(Message::LocationManagerStopped)?;
            }
            if *receiver.borrow() == Status::NotDetermined {
                activate();
                self.state.request.store(true, Ordering::Release);
                loop {
                    receiver
                        .changed()
                        .await
                        .context(Message::LocationManagerStopped)?;
                    if *receiver.borrow_and_update() != Status::NotDetermined {
                        break;
                    }
                }
            }
            Ok(*receiver.borrow())
        })
        .await
        .context(Message::LocationAuthorizationTimedOut)?
    }
}

impl Drop for Location {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
    }
}

fn read(manager: &CLLocationManager) -> Status {
    unsafe {
        if !CLLocationManager::locationServicesEnabled_class() {
            return Status::Disabled;
        }
        map_status(manager.authorizationStatus())
    }
}

fn map_status(status: CLAuthorizationStatus) -> Status {
    if status == CLAuthorizationStatus::AuthorizedAlways
        || status == CLAuthorizationStatus::AuthorizedWhenInUse
    {
        Status::Granted
    } else if status == CLAuthorizationStatus::NotDetermined {
        Status::NotDetermined
    } else if status == CLAuthorizationStatus::Denied {
        Status::Denied
    } else if status == CLAuthorizationStatus::Restricted {
        Status::Restricted
    } else {
        Status::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_authorization_permits_ssid_reads() {
        for status in [
            Status::Unknown,
            Status::NotDetermined,
            Status::Denied,
            Status::Restricted,
            Status::Disabled,
        ] {
            assert!(status.check().is_err());
        }
        assert!(Status::Granted.check().is_ok());
        assert_eq!(map_status(CLAuthorizationStatus(999)), Status::Unknown);
    }
}
