//! Serve only Wi-Fi and permission requests inside the signed user session.
use super::*;
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
