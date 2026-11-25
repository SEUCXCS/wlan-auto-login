//! These tests exclusively contact a disposable loopback HTTP server.
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use wlanlogin_lib::{
    AuthState, Client, Credentials, Error, Line, LoginOutcome, PortalConfig, RequestOperation,
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

// Captured status shape; account, IPs and MACs replaced with synthetic values.
const ONLINE: &str = include_str!("fixtures/chkstatus-online.jsonp");
const OFFLINE: &str = r#"dr1({"result":0,"v46ip":"10.0.0.12","vid":0,"ss4":"000000000000"});"#;
const PAGE_CONFIG: &str = include_str!("fixtures/login-page-config.jsonp");

#[tokio::test]
async fn real_status_shape_is_online_and_repeat_login_never_submits_credentials() -> TestResult {
    let (client, task) = server(vec![ONLINE, ONLINE]).await?;
    assert_eq!(client.status().await?.auth, AuthState::Online);
    let outcome = client
        .login(
            &Credentials {
                username: "student".into(),
                password: "not-validated-or-sent".into(),
            },
            Line::Free1,
        )
        .await?;
    assert_eq!(outcome, LoginOutcome::AlreadyOnline);
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    assert_eq!(requests.len(), 2);
    assert!(
        requests.iter().all(
            |request| request.starts_with("GET /status?") && !request.contains("user_password")
        )
    );
    Ok(())
}

#[tokio::test]
async fn offline_result_allows_login_and_code_only_rejections_are_explained() -> TestResult {
    for (response, rejected, detail) in [
        (
            r#"dr1({"result":0,"ret_code":1,"msg":"student and test-secret: LDAsc3R1ZGVudEBkaWFueGlu dGVzdC1zZWNyZXQ="});"#,
            true,
            "账号或密码不正确",
        ),
        (r#"dr1({"result":0,"ret_code":"2"});"#, false, "IP 已经在线"),
    ] {
        let (client, task) = server(vec![OFFLINE, PAGE_CONFIG, response]).await?;
        let error = client
            .login(
                &Credentials {
                    username: "student".into(),
                    password: "test-secret".into(),
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
        assert_eq!(error.is_credential_rejected(), rejected);
        let diagnostic = format!("{error:?}");
        assert!(diagnostic.contains(detail));
        for secret in [
            "student",
            "test-secret",
            "LDAsc3R1ZGVudEBkaWFueGlu",
            "dGVzdC1zZWNyZXQ=",
        ] {
            assert!(!diagnostic.contains(secret));
        }
        let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
        assert_eq!(requests.len(), 3);
        assert!(requests[2].starts_with("GET /eportal/portal/login?"));
    }
    Ok(())
}

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
        page_config_url: format!("{url}/eportal/portal/page/loadConfig"),
        login_url: format!("{url}/eportal/portal/login"),
        logout_url: format!("{url}/eportal/portal/logout"),
        timeout: Duration::from_secs(2),
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
    let (client, task) = server(vec![OFFLINE, PAGE_CONFIG, r#"dr1({"result":1});"#]).await?;
    assert_eq!(
        client
            .login(
                &Credentials {
                    username: " 2026000000 ".into(),
                    password: "x@&=+ 中!".into(),
                },
                Line::Free2,
            )
            .await?,
        LoginOutcome::Authenticated
    );
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    assert!(requests[0].starts_with("GET /status?"));
    assert!(requests[1].starts_with("GET /eportal/portal/page/loadConfig?"));
    let config = query(&requests[1]);
    assert_eq!(BASE64.decode(&config["wlan_user_ip"])?, b"10.0.0.12");
    assert_eq!(config["wlan_vlan_id"], "0");
    assert_eq!(config["program_index"], "");
    assert_eq!(config["page_index"], "");
    assert_eq!(config["jsVersion"], "4.X");
    assert!(!config.contains_key("user_account") && !config.contains_key("user_password"));

    assert!(requests[2].starts_with("GET /eportal/portal/login?"));
    let params = query(&requests[2]);
    assert_eq!(
        BASE64.decode(&params["user_account"])?,
        b",0,2026000000@free"
    );
    assert_eq!(
        BASE64.decode(&params["user_password"])?,
        "x@&=+ 中!".as_bytes()
    );
    assert_eq!(params["is_base64encode"], "1");
    assert_eq!(params["wlan_user_ip"], "10.0.0.12");
    assert_eq!(params["wlan_vlan_id"], "0");
    assert_eq!(params["wlan_user_mac"], "000000000000");
    assert_eq!(params["wlan_ac_ip"], "");
    assert_eq!(params["wlan_ac_name"], "");
    assert_eq!(params["program_index"], "test-program");
    assert_eq!(params["page_index"], "test-page");
    assert_eq!(params["operate"], "portal_login");
    assert_eq!(params["business_type"], "1");
    assert_eq!(params["jsVersion"], "4.5.1");
    assert!(requests[2].contains("&lang=zh-cn&"));
    assert_eq!(params["lang"], "zh");
    assert!(!params.contains_key("a") && !params.contains_key("c"));
    Ok(())
}

#[tokio::test]
async fn failed_page_config_never_submits_credentials() -> TestResult {
    for (config, detail) in [
        (
            r#"dr1({"code":"0","msg":"raw response"});"#,
            "门户页面配置加载失败",
        ),
        (
            r#"dr1({"code":1,"data":{"program_index":"p"}});"#,
            "缺少 page_index",
        ),
    ] {
        let (client, task) = server(vec![OFFLINE, config]).await?;
        let error = client
            .login(
                &Credentials {
                    username: "student".into(),
                    password: "test-secret".into(),
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
        assert!(format!("{error:?}").contains(detail));
        let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| !request.contains("user_password"))
        );
    }
    Ok(())
}

#[tokio::test]
async fn logout_loads_the_online_page_and_confirms_the_same_device_is_offline() -> TestResult {
    const CONFIGURED_PAGE: &str = r#"dr1({"code":1,"data":{"program_index":"other-program","page_index":"other-page","ac_logout":2,"register_mode":"3"}});"#;
    for (status, page_response, indexes, vlan, ipv6, mac, mode) in [
        (
            ONLINE,
            PAGE_CONFIG,
            ("test-program", "test-page"),
            "1",
            "::",
            "000000000000",
            ("1", "1"),
        ),
        (
            r#"dr1({"userip":"10.0.0.12","online":1,"vlan":42,"myv6ip":"2001:db8::8","mac":"00:11:22:33:44:55"});"#,
            CONFIGURED_PAGE,
            ("other-program", "other-page"),
            "42",
            "2001:db8::8",
            "001122334455",
            ("2", "3"),
        ),
    ] {
        let (client, task) = server(vec![
            status,
            page_response,
            r#"dr1({"result":1,"msg":"Radius注销成功！"});"#,
            OFFLINE,
        ])
        .await?;
        client.logout().await?;
        let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
        assert!(requests[0].starts_with("GET /status?"));
        assert!(requests[1].starts_with("GET /eportal/portal/page/loadConfig?"));
        assert!(requests[2].starts_with("GET /eportal/portal/logout?"));
        assert!(requests[3].starts_with("GET /status?"));
        let config = query(&requests[1]);
        assert_eq!(config["wlan_vlan_id"], vlan);
        assert_eq!(BASE64.decode(&config["wlan_user_ipv6"])?, ipv6.as_bytes());
        assert_eq!(config["jsVersion"], "4.X");
        let params = query(&requests[2]);
        assert_eq!(params["user_account"], "drcom");
        assert_eq!(params["user_password"], "123");
        assert_eq!(params["wlan_user_ip"], "10.0.0.12");
        assert_eq!(params["wlan_user_ipv6"], ipv6);
        assert_eq!(params["wlan_vlan_id"], vlan);
        assert_eq!(params["wlan_user_mac"], mac);
        assert_eq!(params["wlan_ac_ip"], "");
        assert_eq!(params["wlan_ac_name"], "");
        assert_eq!(params["ac_logout"], mode.0);
        assert_eq!(params["register_mode"], mode.1);
        assert_eq!(params["jsVersion"], "4.5.1");
        assert_eq!(params["lang"], "zh");
        assert_eq!(params["program_index"], indexes.0);
        assert_eq!(params["page_index"], indexes.1);
        assert!(!params.contains_key("a") && !params.contains_key("c"));
    }
    Ok(())
}

#[tokio::test]
async fn acknowledged_logout_reports_unconfirmed_status_without_resending_logout() -> TestResult {
    for status in [
        Some(ONLINE),
        Some(r#"dr1({"result":0,"v46ip":"10.0.0.13"});"#),
        None,
    ] {
        let (client, task) = server_with_disconnects(vec![
            Some(ONLINE),
            Some(PAGE_CONFIG),
            Some(r#"dr1({"result":1});"#),
            status,
        ])
        .await?;
        assert!(matches!(
            client.logout().await,
            Err(Error::LogoutUnconfirmed { .. })
        ));
        let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
        assert!(requests[3].starts_with("GET /status?"));
    }
    Ok(())
}

#[tokio::test]
async fn already_offline_logout_avoids_portal_action() -> TestResult {
    let (client, task) = server(vec![OFFLINE]).await?;
    client.logout().await?;
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /status?"));
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
        server_with_disconnects(vec![Some(OFFLINE), Some(PAGE_CONFIG), None]).await?;
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
    assert!(!format!("{error:?}").contains(&BASE64.encode("do-not-log-this")));
    let requests = tokio::time::timeout(Duration::from_secs(3), task).await??;
    assert_eq!(requests.len(), 3);
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
        wlanlogin_lib::AuthState::Online
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
