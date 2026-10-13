//! Authenticate the live peer and console user around every host request.
use super::*;
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
            session.uid == super::identity::session(selected)?.uid,
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
