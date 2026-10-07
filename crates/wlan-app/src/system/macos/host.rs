//! A credential-free Wi-Fi host launched in the user's GUI session.
//! Root clients authenticate the helper's live code identity and active console user.
use super::{
    bundle,
    location::{Location, Status},
    wifi::native,
};
use crate::infra::{
    error::{Context, Message, Result, ensure},
    filesystem::private_dir,
    paths::{self, Paths},
    scope::Scope,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, Semaphore},
    task::JoinHandle,
    time::Instant,
};

const VERSION: u32 = 1;
const MAX_FRAME: usize = 16 * 1024;
static SYSTEM: AtomicBool = AtomicBool::new(false);
static REQUEST: AtomicU64 = AtomicU64::new(1);
static STARTUP: Mutex<()> = Mutex::const_new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Foreground,
    Service,
    System,
}
impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Service => "service",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
enum Operation {
    Status,
    Snapshot,
    Authorize,
    Stop,
}
#[derive(Serialize, Deserialize)]
struct Request {
    version: u32,
    id: u64,
    operation: Operation,
}
#[derive(Serialize, Deserialize)]
struct Response {
    version: u32,
    id: u64,
    location: Status,
    session: Option<String>,
    error: Option<String>,
}

pub fn configure(scope: Scope) {
    SYSTEM.store(scope == Scope::System, Ordering::Release);
}
pub(super) fn scope() -> Scope {
    if SYSTEM.load(Ordering::Acquire) {
        Scope::System
    } else {
        Scope::User
    }
}
fn socket(runtime: &Path, role: Role) -> PathBuf {
    runtime.join(format!("wifi-{}.sock", role.name()))
}

pub struct Host {
    task: JoinHandle<Result<()>>,
    socket: PathBuf,
    _lock: File,
}
impl Host {
    pub async fn start(role: Role) -> Result<Self> {
        ensure!(
            unsafe { libc::geteuid() } != 0,
            Message::MacosUserSessionUnavailable
        );
        bundle::current()?;
        let paths = Paths::new(Scope::User)?;
        crate::infra::cleanup::validate_directory(&paths.runtime)?;
        private_dir(&paths.runtime)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(paths.runtime.join(format!("wifi-{}.lock", role.name())))?;
        fs2::FileExt::try_lock_exclusive(&lock).context(Message::MacosHostAlreadyRunning)?;
        let path = socket(&paths.runtime, role);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(&path)?,
            Ok(_) => {
                return Err(crate::infra::error::error!(
                    Message::ControlSocketPathIsOccupiedByAnotherFile
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let location = Arc::new(Location::new()?);
        location.ready().await?;
        let wifi = Arc::new(native::Monitor::new().await?);
        let listener = UnixListener::bind(&path)?;
        if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
            let _ = fs::remove_file(&path);
            return Err(error.into());
        }
        let task = tokio::spawn(serve(listener, role, location, wifi));
        Ok(Self {
            task,
            socket: path,
            _lock: lock,
        })
    }
    pub async fn wait(mut self) -> Result<()> {
        (&mut self.task).await?
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_file(&self.socket);
    }
}

async fn serve(
    listener: UnixListener,
    role: Role,
    location: Arc<Location>,
    wifi: Arc<native::Monitor>,
) -> Result<()> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|error| {
        crate::infra::error::error!("Failed to create Wi-Fi session identity: {error}")
    })?;
    let instance = Arc::new(
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    );
    let permits = Arc::new(Semaphore::new(16));
    let stop = Arc::new(tokio::sync::Notify::new());
    let mut requests = tokio::task::JoinSet::new();
    let mut last_request = Instant::now();
    let mut expiry = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (mut stream, _) = accepted?;
                let uid = stream.peer_cred()?.uid();
                if uid != unsafe { libc::geteuid() } && uid != 0 { continue; }
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue; };
                last_request = Instant::now();
                let (location, wifi, instance, stop) = (location.clone(), wifi.clone(), instance.clone(), stop.clone());
                requests.spawn(async move {
                    let _permit = permit;
                    let _ = tokio::time::timeout(Duration::from_secs(50), async {
                        let request: Request = read_frame(&mut stream).await?;
                        ensure!(request.version == VERSION, Message::MacosHostProtocol);
                        let mut response = Response { version: VERSION, id: request.id, location: location.status(), session: None, error: None };
                        match request.operation {
                            Operation::Status => {},
                            Operation::Authorize => match location.authorize().await {
                                Ok(status) => response.location = status,
                                Err(error) => response.error = Some(error.to_string()),
                            },
                            Operation::Snapshot => {
                                let result = async {
                                    response.location.check()?;
                                    let snapshot = tokio::time::timeout(Duration::from_secs(4), wifi.snapshot()).await.context(Message::NativeWifiQueryTimedOut)??;
                                    location.status().check()?;
                                    Ok::<_, crate::infra::error::Error>(snapshot)
                                }.await;
                                match result {
                                    Ok(session) => response.session = session.map(|session| format!("{instance}:{session}")),
                                    Err(error) => response.error = Some(error.to_string()),
                                }
                            },
                            Operation::Stop => {
                                ensure!(role != Role::Service, Message::ManagedHostStopsWithService);
                                ensure!(role != Role::System || uid == 0, Message::MacosHostIdentity);
                            },
                        }
                        write_frame(&mut stream, &response).await?;
                        if matches!(request.operation, Operation::Stop) { stop.notify_one(); }
                        Ok::<_, crate::infra::error::Error>(())
                    }).await;
                });
            },
            _ = stop.notified() => break,
            _ = expiry.tick() => {
                if role == Role::System && fs::metadata("/dev/console").map(|m| m.uid()).ok() != Some(unsafe { libc::geteuid() }) { break; }
                if role != Role::Service && requests.is_empty() && last_request.elapsed() > Duration::from_secs(120) { break; }
            },
            _ = requests.join_next(), if !requests.is_empty() => {},
        }
    }
    Ok(())
}

pub async fn prepare(interactive: bool) -> Result<()> {
    let response = call(
        if interactive {
            Operation::Authorize
        } else {
            Operation::Snapshot
        },
        true,
    )
    .await?;
    response.location.check()
}

pub async fn location_permission(request: bool) -> Result<Status> {
    Ok(call(
        if request {
            Operation::Authorize
        } else {
            Operation::Status
        },
        request,
    )
    .await?
    .location)
}
pub async fn snapshot() -> Result<Option<String>> {
    let response = call(Operation::Snapshot, true).await?;
    response.location.check()?;
    Ok(response.session)
}

async fn call(operation: Operation, launch: bool) -> Result<Response> {
    let selected = scope();
    let session = session(selected)?;
    let app = bundle::resolve(selected)?;
    let mut stream = match connect(&session, selected, &app).await? {
        Some(stream) => stream,
        None if launch => {
            let _startup = tokio::time::timeout(Duration::from_secs(45), STARTUP.lock())
                .await
                .context(Message::MacosHostUnavailable)?;
            if let Some(stream) = connect(&session, selected, &app).await? {
                stream
            } else {
                launch_host(
                    &app,
                    selected,
                    session.uid,
                    matches!(operation, Operation::Authorize),
                )
                .await?;
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if let Some(stream) = connect(&session, selected, &app).await? {
                            return Ok::<_, crate::infra::error::Error>(stream);
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
                .await
                .context(Message::MacosHostUnavailable)??
            }
        }
        None => return Err(crate::infra::error::error!(Message::MacosHostUnavailable)),
    };
    let id = REQUEST.fetch_add(1, Ordering::Relaxed);
    let response: Response = tokio::time::timeout(
        Duration::from_secs(if matches!(operation, Operation::Authorize) {
            48
        } else {
            5
        }),
        async {
            write_frame(
                &mut stream,
                &Request {
                    version: VERSION,
                    id,
                    operation,
                },
            )
            .await?;
            read_frame(&mut stream).await
        },
    )
    .await
    .context(Message::MacosHostUnavailable)?
    .context(Message::MacosHostRequestFailed)?;
    ensure!(
        response.version == VERSION && response.id == id,
        Message::MacosHostProtocol
    );
    if selected == Scope::System {
        ensure!(
            session.uid == self::session(selected)?.uid,
            Message::MacosUserSessionUnavailable
        );
    }
    if let Some(error) = &response.error {
        return Err(crate::infra::error::Error::message(error.clone()));
    }
    Ok(response)
}

/// Stop transient hosts before changing their code identity. Managed hosts are
/// stopped by daemon control; no helper is launched during cleanup.
pub async fn stop_helpers(selected: Scope, app: &Path) -> Result<()> {
    let session = match session(selected) {
        Ok(session) => session,
        Err(_) if selected == Scope::System && fs::metadata("/dev/console")?.uid() == 0 => {
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let role = if selected == Scope::System {
        Role::System
    } else {
        Role::Foreground
    };
    let path = socket(&session.runtime, role);
    let mut stream = match UnixStream::connect(&path).await {
        Ok(stream) => stream,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    bundle::verify_peer(&stream, session.uid, app).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        let id = REQUEST.fetch_add(1, Ordering::Relaxed);
        write_frame(
            &mut stream,
            &Request {
                version: VERSION,
                id,
                operation: Operation::Stop,
            },
        )
        .await?;
        let response: Response = read_frame(&mut stream).await?;
        ensure!(
            response.version == VERSION && response.id == id && response.error.is_none(),
            Message::MacosHostProtocol
        );
        loop {
            match UnixStream::connect(&path).await {
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    return Ok::<_, crate::infra::error::Error>(());
                }
                Err(e) => return Err(e.into()),
                Ok(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .context(Message::MacosHostStopFailed)?
}

async fn connect(session: &Session, scope: Scope, app: &Path) -> Result<Option<UnixStream>> {
    let roles: &[Role] = if scope == Scope::System {
        &[Role::System]
    } else {
        &[Role::Service, Role::Foreground]
    };
    for role in roles {
        match UnixStream::connect(socket(&session.runtime, *role)).await {
            Ok(stream) => {
                bundle::verify_peer(&stream, session.uid, app).await?;
                return Ok(Some(stream));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(None)
}

fn host_launch(
    app: &Path,
    scope: Scope,
    uid: u32,
    interactive: bool,
) -> crate::infra::command::Invocation {
    use crate::infra::command::Invocation;
    use std::ffi::OsString;
    let mut args: Vec<OsString> = vec!["-n".into()];
    if !interactive {
        args.push("-g".into());
    }
    args.extend([
        "-a".into(),
        app.as_os_str().to_owned(),
        "--args".into(),
        "macos-host".into(),
    ]);
    if scope == Scope::System {
        args.push("--system-helper".into());
        let mut command: Vec<OsString> = vec![
            "asuser".into(),
            uid.to_string().into(),
            "/usr/bin/sudo".into(),
            "-u".into(),
            format!("#{uid}").into(),
            "/usr/bin/open".into(),
        ];
        command.extend(args);
        Invocation::new("/bin/launchctl", command)
    } else {
        Invocation::new("/usr/bin/open", args)
    }
}

async fn launch_host(app: &Path, scope: Scope, uid: u32, interactive: bool) -> Result<()> {
    host_launch(app, scope, uid, interactive).run().await
}

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

struct Session {
    uid: u32,
    runtime: PathBuf,
}
fn session(scope: Scope) -> Result<Session> {
    if scope == Scope::User {
        return Ok(Session {
            uid: unsafe { libc::geteuid() },
            runtime: Paths::new(Scope::User)?.runtime,
        });
    }
    let uid = fs::metadata("/dev/console")?.uid();
    ensure!(uid != 0, Message::MacosUserSessionUnavailable);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    ensure!(
        status == 0 && !result.is_null(),
        Message::MacosUserSessionUnavailable
    );
    let home = unsafe { std::ffi::CStr::from_ptr(record.pw_dir) };
    let home = Path::new(std::ffi::OsStr::from_bytes(home.to_bytes()));
    Ok(Session {
        uid,
        runtime: paths::app_dir(home.join("Library/Application Support")),
    })
}

async fn read_frame<T: serde::de::DeserializeOwned>(
    stream: &mut (impl AsyncReadExt + Unpin),
) -> Result<T> {
    let length = stream.read_u32().await? as usize;
    ensure!(
        length > 0 && length <= MAX_FRAME,
        Message::MacosHostProtocol
    );
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
async fn write_frame<T: Serialize>(
    stream: &mut (impl AsyncWriteExt + Unpin),
    value: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_FRAME, Message::MacosHostProtocol);
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_interactive_authorization_launches_in_the_foreground() {
        for scope in [Scope::User, Scope::System] {
            let foreground = host_launch(Path::new("/Applications/Test.app"), scope, 501, true);
            let background = host_launch(Path::new("/Applications/Test.app"), scope, 501, false);
            assert!(!foreground.args.iter().any(|arg| arg == "-g"));
            assert!(background.args.iter().any(|arg| arg == "-g"));
            assert!(foreground.args.iter().any(|arg| arg == "macos-host"));
            if scope == Scope::System {
                assert_eq!(foreground.program, "/bin/launchctl");
                assert!(foreground.args.iter().any(|arg| arg == "#501"));
                assert!(foreground.args.iter().any(|arg| arg == "--system-helper"));
            }
        }
    }
    #[tokio::test]
    async fn ipc_rejects_oversized_frames_without_allocating_the_payload() -> Result<()> {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer.write_u32(u32::MAX).await?;
        assert!(read_frame::<Request>(&mut reader).await.is_err());
        Ok(())
    }
    #[tokio::test]
    async fn response_preserves_request_identity_and_permission_failure() -> Result<()> {
        let (mut writer, mut reader) = tokio::io::duplex(1024);
        let response = Response {
            version: VERSION,
            id: 19,
            location: Status::Denied,
            session: None,
            error: Some("denied".into()),
        };
        write_frame(&mut writer, &response).await?;
        let decoded: Response = read_frame(&mut reader).await?;
        assert_eq!(decoded.id, 19);
        assert_eq!(decoded.location, Status::Denied);
        assert!(decoded.session.is_none());
        Ok(())
    }
}
