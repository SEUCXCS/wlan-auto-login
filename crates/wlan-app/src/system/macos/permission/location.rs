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
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

fn activate() -> bool {
    // NSRunningApplication supports calls from any thread. Bring only this
    // application forward for an explicitly interactive authorization request.
    unsafe {
        let app: Retained<objc2::runtime::AnyObject> =
            msg_send![objc2::class!(NSRunningApplication), currentApplication];
        let active: bool = msg_send![&*app, isActive];
        active || msg_send![&*app, activateWithOptions: 0usize]
    }
}

fn active() -> bool {
    unsafe {
        let app: Retained<objc2::runtime::AnyObject> =
            msg_send![objc2::class!(NSRunningApplication), currentApplication];
        msg_send![&*app, isActive]
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
        let message = match self {
            Self::Granted => return Ok(()),
            Self::Denied => Message::LocationDenied,
            Self::Disabled => Message::LocationServicesDisabled,
            Self::Restricted => Message::LocationRestricted,
            _ => Message::LocationUnavailable {
                state: format!("{self:?}"),
            },
        };
        Err(message.into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Activation {
    Idle,
    Pending,
    Requested,
    Failed,
    TimedOut,
}

struct State {
    status: watch::Sender<Status>,
    activation: watch::Sender<Activation>,
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
    authorization: tokio::sync::Mutex<()>,
}

impl Location {
    pub fn new() -> Result<Self> {
        let (sender, status) = watch::channel(Status::Unknown);
        let (activation, _) = watch::channel(Activation::Idle);
        let state = Arc::new(State {
            status: sender,
            activation,
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
                    let mut activating = None;
                    while !worker.stop.load(Ordering::Acquire) {
                        let status = read(&manager);
                        let request = worker.request.swap(false, Ordering::AcqRel);
                        if request && status == Status::NotDetermined {
                            if activate() {
                                activating = Some(Instant::now());
                                worker.activation.send_replace(Activation::Pending);
                            } else {
                                worker.activation.send_replace(Activation::Failed);
                            }
                        } else if request {
                            // Authorization may have changed after the client queued its request.
                            worker.activation.send_replace(Activation::Requested);
                        }
                        let mut requested = false;
                        if let Some(started) = activating {
                            if status != Status::NotDetermined {
                                activating = None;
                                worker.activation.send_replace(Activation::Requested);
                            } else if active() {
                                // Activation acceptance alone is insufficient. Core Location
                                // ignores in-use requests from an inactive application.
                                manager.requestWhenInUseAuthorization();
                                requested = true;
                                activating = None;
                                worker.activation.send_replace(Activation::Requested);
                            } else if started.elapsed() >= Duration::from_secs(3) {
                                activating = None;
                                worker.activation.send_replace(Activation::TimedOut);
                            }
                        }
                        // macOS CLI-style hosts need an active Location client for CoreWLAN.
                        // No coordinates are stored, returned over IPC or written to logs.
                        if !updating && (status == Status::Granted || requested) {
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
                        let status = read(&manager);
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
        Ok(Self {
            state,
            status,
            authorization: tokio::sync::Mutex::new(()),
        })
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
        tracing::info!(state = ?self.status(), "{}", Message::LocationAuthorizationBefore);
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            let _authorization = self.authorization.lock().await;
            self.ready().await?;
            let mut receiver = self.status.clone();
            if *receiver.borrow_and_update() == Status::NotDetermined {
                self.state.activation.send_replace(Activation::Idle);
                let mut activation = self.state.activation.subscribe();
                self.state.request.store(true, Ordering::Release);
                loop {
                    match *activation.borrow_and_update() {
                        Activation::Requested => break,
                        Activation::Failed => {
                            crate::infra::error::bail!(Message::LocationActivationFailed)
                        }
                        Activation::TimedOut => {
                            crate::infra::error::bail!(Message::LocationActivationTimedOut)
                        }
                        Activation::Idle | Activation::Pending => {}
                    }
                    activation
                        .changed()
                        .await
                        .context(Message::LocationManagerStopped)?;
                }
                loop {
                    if *receiver.borrow_and_update() != Status::NotDetermined {
                        break;
                    }
                    receiver
                        .changed()
                        .await
                        .context(Message::LocationManagerStopped)?;
                }
            }
            Ok(*receiver.borrow())
        })
        .await
        .context(Message::LocationAuthorizationTimedOut)
        .and_then(|result| result);
        tracing::info!(state = ?self.status(), error = ?result.as_ref().err(), "{}", Message::LocationAuthorizationAfter);
        result
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

    fn simulated_location(status: Status) -> Location {
        let (sender, receiver) = watch::channel(status);
        let (activation, _) = watch::channel(Activation::Idle);
        Location {
            state: Arc::new(State {
                status: sender,
                activation,
                request: AtomicBool::new(false),
                stop: AtomicBool::new(false),
            }),
            status: receiver,
            authorization: tokio::sync::Mutex::new(()),
        }
    }

    #[tokio::test]
    async fn decided_authorization_and_global_disable_never_request_again() -> Result<()> {
        for status in [
            Status::Granted,
            Status::Denied,
            Status::Disabled,
            Status::Restricted,
        ] {
            let location = simulated_location(status);
            assert_eq!(location.authorize().await?, status);
            assert!(!location.state.request.load(Ordering::Acquire));
        }
        assert!(matches!(
            Status::Denied.check(),
            Err(crate::infra::error::Error::Message(Message::LocationDenied))
        ));
        assert!(matches!(
            Status::Disabled.check(),
            Err(crate::infra::error::Error::Message(
                Message::LocationServicesDisabled
            ))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn activation_failure_and_permission_decision_timeout_are_distinct() -> Result<()> {
        for activation in [Activation::Failed, Activation::TimedOut] {
            let location = simulated_location(Status::NotDetermined);
            let state = location.state.clone();
            let worker = async move {
                while !state.request.swap(false, Ordering::AcqRel) {
                    tokio::task::yield_now().await;
                }
                state.activation.send_replace(activation);
            };
            let (result, ()) = tokio::join!(location.authorize(), worker);
            let error = result.err().context("Expected activation failure")?;
            let expected = if activation == Activation::Failed {
                Message::LocationActivationFailed
            } else {
                Message::LocationActivationTimedOut
            };
            assert_eq!(error.to_string(), expected.to_string());
        }
        let location = simulated_location(Status::NotDetermined);
        let state = location.state.clone();
        let worker = async move {
            while !state.request.swap(false, Ordering::AcqRel) {
                tokio::task::yield_now().await;
            }
            state.activation.send_replace(Activation::Requested);
            tokio::time::advance(Duration::from_secs(46)).await;
        };
        tokio::time::pause();
        let (result, ()) = tokio::join!(location.authorize(), worker);
        assert!(matches!(
            result,
            Err(crate::infra::error::Error::Context {
                message: Message::LocationAuthorizationTimedOut,
                ..
            })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn authorization_waits_for_activation_then_uses_the_callback_state() -> Result<()> {
        let location = simulated_location(Status::NotDetermined);
        let state = location.state.clone();
        let worker = async move {
            while !state.request.swap(false, Ordering::AcqRel) {
                tokio::task::yield_now().await;
            }
            state.activation.send_replace(Activation::Pending);
            tokio::task::yield_now().await;
            // A callback can arrive before the waiter observes request acceptance.
            state.status.send_replace(Status::Granted);
            state.activation.send_replace(Activation::Requested);
        };
        let (result, ()) = tokio::join!(location.authorize(), worker);
        assert_eq!(result?, Status::Granted);
        Ok(())
    }

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
