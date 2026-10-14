use std::time::Duration;

/// AppKit activation properties update on the main run loop. Keep it running for
/// GUI hosts while Tokio owns their asynchronous work, including managed hosts
/// that may later receive an explicit foreground authorization request over IPC.
pub fn run_gui(
    runtime: &tokio::runtime::Runtime,
    operation: impl std::future::Future<Output = std::process::ExitCode> + Send + 'static,
) -> std::process::ExitCode {
    use objc2::{msg_send, rc::Retained};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};
    use std::sync::mpsc;

    objc2::rc::autoreleasepool(|_| unsafe {
        let app: Retained<objc2::runtime::AnyObject> =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let _: () = msg_send![&*app, finishLaunching];
        let (sender, receiver) = mpsc::sync_channel(1);
        // The monitor also closes the channel if the operation panics.
        let task = runtime.spawn(operation);
        runtime.spawn(async move {
            let result = task.await.unwrap_or(std::process::ExitCode::FAILURE);
            let _ = sender.send(result);
        });
        loop {
            match receiver.try_recv() {
                Ok(result) => return result,
                Err(mpsc::TryRecvError::Disconnected) => return std::process::ExitCode::FAILURE,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            objc2::rc::autoreleasepool(|_| {
                let deadline = NSDate::dateWithTimeIntervalSinceNow(0.05);
                let event: Option<Retained<objc2::runtime::AnyObject>> = msg_send![
                    &*app, nextEventMatchingMask: usize::MAX,
                    untilDate: &*deadline, inMode: NSDefaultRunLoopMode, dequeue: true
                ];
                if let Some(event) = event {
                    let _: () = msg_send![&*app, sendEvent: &*event];
                }
            });
            // A run loop without sources may return immediately.
            std::thread::sleep(Duration::from_millis(10));
        }
    })
}
