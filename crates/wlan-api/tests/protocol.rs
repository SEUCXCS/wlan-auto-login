//! These tests exclusively contact a disposable loopback HTTP server.
use cxxy_wlanlogin_lib::{Client, Credentials, Error, Line, PortalConfig, RequestOperation};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn server(
    responses: Vec<&'static str>,
) -> Result<(Client, JoinHandle<Vec<String>>), Box<dyn std::error::Error + Send + Sync>> {
    server_with_disconnects(responses.into_iter().map(Some).collect()).await
}

// None closes the accepted connection without sending an HTTP response.
async fn server_with_disconnects(
    responses: Vec<Option<&'static str>>,
) -> Result<(Client, JoinHandle<Vec<String>>), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener
                .accept()
                .await
                .unwrap_or_else(|e| panic!("mock accept: {e}"));
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                if stream.read(&mut byte).await.unwrap_or_default() == 0 {
                    break;
                }
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 16_384);
            }
            requests.push(String::from_utf8_lossy(&request).into_owned());
            let Some(response) = response else { continue };
            let http = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            );
            stream
                .write_all(http.as_bytes())
                .await
                .unwrap_or_else(|e| panic!("mock write: {e}"));
        }
        requests
    });
    let client = Client::new(PortalConfig {
        status_url: format!("{url}/status"),
        action_url: format!("{url}/action"),
        timeout: Duration::from_secs(2),
        ..PortalConfig::default()
    })?;
    Ok((client, task))
}

fn query(request: &str) -> std::collections::HashMap<String, String> {
    let target = request.split_whitespace().nth(1).unwrap_or_default();
    reqwest::Url::parse(&format!("http://localhost{target}"))
        .unwrap_or_else(|e| panic!("mock URL: {e}"))
        .query_pairs()
        .into_owned()
        .collect()
}

#[tokio::test]
async fn login_encodes_credentials_and_uses_fresh_device_ip() -> TestResult {
    let (client, task) = server(vec![
        r#"dr1({"v46ip":"10.0.0.12","online":0});"#,
        r#"dr1({"result":1});"#,
    ])
    .await?;
    client
        .login(
            &Credentials {
                username: "2026000000".into(),
                password: "x@&=+ 中".into(),
            },
            Line::Free2,
        )
        .await?;
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    let params = query(&requests[1]);
    assert_eq!(params["user_account"], "2026000000@free");
    assert_eq!(params["user_password"], "x@&=+ 中");
    assert_eq!(params["wlan_user_ip"], "10.0.0.12");
    assert_eq!(params["a"], "login");
    Ok(())
}

#[tokio::test]
async fn logout_uses_placeholders_and_device_context() -> TestResult {
    let (client, task) = server(vec![
        r#"dr1({"userip":"10.0.0.12","online":1,"vlan":42,"mac":"001122334455"});"#,
        r#"dr1({"result":"ok"});"#,
    ])
    .await?;
    client.logout().await?;
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    let params = query(&requests[1]);
    assert_eq!(params["user_account"], "drcom");
    assert_eq!(params["user_password"], "123");
    assert_eq!(params["wlan_vlan_id"], "42");
    assert_eq!(params["wlan_user_mac"], "001122334455");
    assert_eq!(params["a"], "logout");
    Ok(())
}

#[tokio::test]
async fn invalid_device_status_prevents_action_request() -> TestResult {
    let (client, task) = server(vec![r#"dr1({"result":1});"#]).await?;
    assert!(client.logout().await.is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), task)
            .await??
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn password_is_absent_from_transport_errors() -> TestResult {
    let (client, task) =
        server_with_disconnects(vec![Some(r#"dr1({"userip":"10.0.0.12"});"#), None]).await?;
    let error = client
        .login(
            &Credentials {
                username: "student".into(),
                password: "do-not-log-this".into(),
            },
            Line::Free1,
        )
        .await
        .err()
        .ok_or("login unexpectedly succeeded")?;
    assert!(matches!(
        error,
        Error::Request {
            operation: RequestOperation::Login,
            ..
        }
    ));
    assert!(!error.is_transient_status_failure());
    assert!(error.to_string().contains("登录请求"));
    assert!(!format!("{error:?}").contains("do-not-log-this"));
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    assert_eq!(requests.len(), 2);
    Ok(())
}

#[tokio::test]
async fn status_disconnect_can_recover_without_submitting_credentials() -> TestResult {
    let (client, task) = server_with_disconnects(vec![
        None,
        Some(r#"dr1({"userip":"10.0.0.12","online":1});"#),
    ])
    .await?;
    let error = client
        .status()
        .await
        .err()
        .ok_or("status unexpectedly succeeded")?;
    assert!(error.is_transient_status_failure());
    assert!(error.to_string().contains("状态查询"));
    assert_eq!(
        client.status().await?.auth,
        cxxy_wlanlogin_lib::AuthState::Online
    );
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.starts_with("GET /status?"))
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("user_password"))
    );
    Ok(())
}

#[tokio::test]
async fn login_preserves_status_failure_stage_and_never_sends_action() -> TestResult {
    let (client, task) = server_with_disconnects(vec![None]).await?;
    let error = client
        .login(
            &Credentials {
                username: "student".into(),
                password: "do-not-log-this".into(),
            },
            Line::Free1,
        )
        .await
        .err()
        .ok_or("login unexpectedly succeeded")?;
    assert!(error.is_transient_status_failure());
    assert!(matches!(
        error,
        Error::Request {
            operation: RequestOperation::Status,
            ..
        }
    ));
    assert!(!format!("{error:?}").contains("do-not-log-this"));
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /status?"));
    Ok(())
}

#[tokio::test]
async fn invalid_status_payloads_do_not_qualify_for_fast_retries() -> TestResult {
    let (client, task) = server(vec!["not JSONP", r#"dr1({"result":1});"#]).await?;
    for _ in 0..2 {
        let error = client
            .status()
            .await
            .err()
            .ok_or("status unexpectedly succeeded")?;
        assert!(error.to_string().contains("状态查询"));
        assert!(!error.is_transient_status_failure());
    }
    tokio::time::timeout(Duration::from_secs(3), task).await??;
    Ok(())
}
