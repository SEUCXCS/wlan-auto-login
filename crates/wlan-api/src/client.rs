use crate::{
    AuthState, Credentials, Error, Line, LoginOutcome, ProtocolError, RequestOperation, Status,
    diagnostics::Exchange,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use std::{
    net::Ipv4Addr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

const CALLBACK: &str = "dr1";
const PADDED_PORTAL_CALLBACK: &str = "dr0001";
const MAX_RESPONSE: usize = 64 * 1024;
const PAGE_JS_VERSION: &str = "4.X";
const ACTION_JS_VERSION: &str = "4.5.1";
const ZERO_MAC: &str = "000000000000";

#[derive(Clone, Debug)]
pub struct PortalConfig {
    pub status_url: String,
    pub page_config_url: String,
    pub login_url: String,
    pub logout_url: String,
    pub timeout: Duration,
}

impl Default for PortalConfig {
    fn default() -> Self {
        Self {
            status_url: "http://172.18.254.6/drcom/chkstatus".into(),
            page_config_url: "http://172.18.254.6:803/eportal/portal/page/loadConfig".into(),
            login_url: "http://172.18.254.6:803/eportal/portal/login".into(),
            logout_url: "http://172.18.254.6:803/eportal/portal/logout".into(),
            timeout: Duration::from_secs(8),
        }
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    config: PortalConfig,
}

/// The portal state observed during a login preflight.
pub enum LoginPreparation<'a> {
    AlreadyOnline,
    Unknown,
    Offline(Box<PreparedLogin<'a>>),
}

/// An offline device and its page configuration, ready for a single submission.
pub struct PreparedLogin<'a> {
    client: &'a Client,
    status: Status,
    page: PortalPage,
}

struct PortalPage {
    program_index: String,
    page_index: String,
    account_prefix: bool,
    enable_r3: String,
    rcn: String,
    ac_logout: String,
    register_mode: String,
}

impl Client {
    pub fn new(config: PortalConfig) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(config.timeout)
            .build()?;
        Ok(Self { http, config })
    }

    pub async fn status(&self) -> Result<Status, Error> {
        self.status_for(RequestOperation::Status).await
    }

    async fn status_for(&self, operation: RequestOperation) -> Result<Status, Error> {
        self.request(
            RequestOperation::Status,
            operation,
            &self.config.status_url,
            &[("callback", CALLBACK.to_owned()), ("v", nonce())],
            &[],
            parse_status,
        )
        .await
    }

    pub async fn login(
        &self,
        credentials: &Credentials,
        line: Line,
    ) -> Result<LoginOutcome, Error> {
        if credentials.username.trim().is_empty() || credentials.password.is_empty() {
            return Err(Error::EmptyCredentials);
        }
        match self.prepare_login().await? {
            LoginPreparation::AlreadyOnline => Ok(LoginOutcome::AlreadyOnline),
            LoginPreparation::Unknown => Err(Error::Protocol(ProtocolError::LoginStateUnknown)),
            LoginPreparation::Offline(prepared) => prepared.submit(credentials, line).await,
        }
    }

    /// Resolve all portal prerequisites before the caller checks its local session.
    pub async fn prepare_login(&self) -> Result<LoginPreparation<'_>, Error> {
        let status = self.status_for(RequestOperation::Login).await?;
        match status.auth {
            AuthState::Online => return Ok(LoginPreparation::AlreadyOnline),
            AuthState::Unknown => return Ok(LoginPreparation::Unknown),
            AuthState::Offline => {}
        }
        let page = self
            .load_page(RequestOperation::Login, &status, &status.vlan, &status.ipv6)
            .await?;
        Ok(LoginPreparation::Offline(Box::new(PreparedLogin {
            client: self,
            status,
            page,
        })))
    }
}

impl PreparedLogin<'_> {
    /// Submit credentials without another portal preflight request.
    pub async fn submit(
        self,
        credentials: &Credentials,
        line: Line,
    ) -> Result<LoginOutcome, Error> {
        let username = credentials.username.trim();
        if username.is_empty() || credentials.password.is_empty() {
            return Err(Error::EmptyCredentials);
        }
        let Self {
            client,
            status,
            page,
        } = self;
        let prefix = if page.account_prefix { ",0," } else { "" };
        let account = Zeroizing::new(format!("{prefix}{username}{}", line.suffix()));
        let encoded_account = Zeroizing::new(BASE64.encode(account.as_bytes()));
        let encoded_password = Zeroizing::new(BASE64.encode(credentials.password.as_bytes()));
        let ip = status.ip.to_string();
        let mac = portal_mac(&status.mac);
        let query = [
            ("callback", CALLBACK),
            ("login_method", "1"),
            ("is_base64encode", "1"),
            ("user_account", encoded_account.as_str()),
            ("user_password", encoded_password.as_str()),
            ("wlan_user_ip", ip.as_str()),
            ("wlan_user_ipv6", status.ipv6.as_str()),
            ("wlan_user_mac", mac.as_str()),
            ("wlan_vlan_id", status.vlan.as_str()),
            ("wlan_ac_ip", status.ac_ip.as_deref().unwrap_or_default()),
            (
                "wlan_ac_name",
                status.ac_name.as_deref().unwrap_or_default(),
            ),
            ("authex_enable", ""),
            ("jsVersion", ACTION_JS_VERSION),
            ("uuid", ""),
            ("terminal_type", "1"),
            ("lang", "zh-cn"),
            (
                "user_agent",
                concat!("wlanlogin/", env!("CARGO_PKG_VERSION")),
            ),
            ("enable_r3", page.enable_r3.as_str()),
            ("mac_type", "0"),
            ("rcn", page.rcn.as_str()),
            ("operate", "portal_login"),
            ("business_type", "1"),
            ("program_index", page.program_index.as_str()),
            ("page_index", page.page_index.as_str()),
            ("v", &nonce()),
            ("lang", "zh"),
        ];
        let secrets = [
            encoded_account.as_str(),
            encoded_password.as_str(),
            account.as_str(),
            credentials.password.as_str(),
            username,
        ];
        client
            .request(
                RequestOperation::Login,
                RequestOperation::Login,
                &client.config.login_url,
                &query,
                &secrets,
                |value| check_action(value, &secrets),
            )
            .await
            .map(|()| LoginOutcome::Authenticated)
    }
}

impl Client {
    async fn load_page(
        &self,
        operation: RequestOperation,
        status: &Status,
        vlan: &str,
        ipv6: &str,
    ) -> Result<PortalPage, Error> {
        let query = [
            ("callback", CALLBACK.into()),
            ("program_index", String::new()),
            ("page_index", String::new()),
            ("wlan_vlan_id", vlan.into()),
            ("wlan_user_ip", BASE64.encode(status.ip.to_string())),
            ("wlan_user_ipv6", BASE64.encode(ipv6)),
            ("wlan_user_ssid", String::new()),
            ("wlan_user_areaid", String::new()),
            (
                "wlan_ac_ip",
                BASE64.encode(status.ac_ip.as_deref().unwrap_or_default()),
            ),
            ("wlan_ap_mac", ZERO_MAC.into()),
            ("gw_id", ZERO_MAC.into()),
            ("jsVersion", PAGE_JS_VERSION.into()),
            ("v", nonce()),
            ("lang", "zh".into()),
        ];
        self.request(
            RequestOperation::PageConfig,
            operation,
            &self.config.page_config_url,
            &query,
            &[],
            parse_page,
        )
        .await
    }

    pub async fn logout(&self) -> Result<(), Error> {
        let status = self.status_for(RequestOperation::Logout).await?;
        if status.auth == AuthState::Offline {
            return Ok(());
        }

        // The campus portal's online page uses VLAN 1 and :: for the
        // zero-VLAN IPv4 session. Preserve explicit device context otherwise.
        let vlan = if status.vlan == "0" {
            "1"
        } else {
            &status.vlan
        };
        let ipv6 = if status.ipv6.is_empty() {
            "::"
        } else {
            &status.ipv6
        };
        let page = self
            .load_page(RequestOperation::Logout, &status, vlan, ipv6)
            .await?;
        let ip = status.ip.to_string();
        let mac = portal_mac(&status.mac);
        let query = [
            ("callback", CALLBACK),
            ("login_method", "1"),
            ("user_account", "drcom"),
            ("user_password", "123"),
            ("ac_logout", page.ac_logout.as_str()),
            ("register_mode", page.register_mode.as_str()),
            ("wlan_user_ip", ip.as_str()),
            ("wlan_user_ipv6", ipv6),
            ("wlan_vlan_id", vlan),
            ("wlan_user_mac", mac.as_str()),
            ("wlan_ac_ip", status.ac_ip.as_deref().unwrap_or_default()),
            (
                "wlan_ac_name",
                status.ac_name.as_deref().unwrap_or_default(),
            ),
            ("jsVersion", ACTION_JS_VERSION),
            ("program_index", page.program_index.as_str()),
            ("page_index", page.page_index.as_str()),
            ("v", &nonce()),
            ("lang", "zh"),
        ];
        self.request(
            RequestOperation::Logout,
            RequestOperation::Logout,
            &self.config.logout_url,
            &query,
            &[],
            |value| check_action(value, &[]),
        )
        .await?;

        match self.status_for(RequestOperation::Logout).await {
            Ok(after) if after.ip == status.ip && after.auth == AuthState::Offline => Ok(()),
            Ok(_) => Err(Error::LogoutUnconfirmed { source: None }),
            Err(error) => Err(Error::LogoutUnconfirmed {
                source: Some(Box::new(error)),
            }),
        }
    }

    async fn request<T>(
        &self,
        endpoint: RequestOperation,
        operation: RequestOperation,
        url: &str,
        query: &impl serde::Serialize,
        secrets: &[&str],
        interpret: impl FnOnce(&Value) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut exchange = Exchange::new(endpoint, operation, secrets);
        let result = self
            .send_request(endpoint, url, query, &mut exchange)
            .await
            .and_then(|value| interpret(&value));
        exchange.finish(result.as_ref().err());
        result.map_err(|error| error.during(endpoint))
    }

    async fn send_request(
        &self,
        endpoint: RequestOperation,
        url: &str,
        query: &impl serde::Serialize,
        exchange: &mut Exchange<'_>,
    ) -> Result<Value, Error> {
        let request = self.http.get(url).query(query).build()?;
        exchange.request(&request);
        let mut response = self.http.execute(request).await?;
        exchange.response(&response);
        let http_error = response.error_for_status_ref().err().map(Error::from);
        let parsed = read_payload(&mut response, exchange, endpoint).await;
        if let Some(error) = http_error {
            return Err(error);
        }
        parsed
    }
}

async fn read_payload(
    response: &mut reqwest::Response,
    exchange: &mut Exchange<'_>,
    endpoint: RequestOperation,
) -> Result<Value, Error> {
    let mut body = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                exchange.body(&body, false);
                return Err(error.into());
            }
        };
        exchange.received_bytes(chunk.len());
        let remaining = MAX_RESPONSE - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            exchange.body(&body, false);
            return Err(Error::Protocol(ProtocolError::BodyTooLarge));
        }
    }
    exchange.body(&body, true);
    let text =
        std::str::from_utf8(&body).map_err(|_| Error::Protocol(ProtocolError::InvalidUtf8))?;
    parse_jsonp(text, endpoint)
}

fn parse_page(response: &Value) -> Result<PortalPage, Error> {
    if numeric_field(response, "code") == Some(0) {
        return Err(Error::Protocol(ProtocolError::PageConfigRejected));
    }
    let data = &response["data"];
    Ok(PortalPage {
        program_index: text_field(data, &["program_index"])
            .ok_or(Error::Protocol(ProtocolError::MissingProgramIndex))?,
        page_index: text_field(data, &["page_index"])
            .ok_or(Error::Protocol(ProtocolError::MissingPageIndex))?,
        // The captured campus requests use the account prefix.
        account_prefix: numeric_field(data, "account_prefix") != Some(0),
        enable_r3: numeric_field(data, "enable_r3")
            .unwrap_or_default()
            .to_string(),
        rcn: text_field(data, &["rcn"]).unwrap_or_default(),
        ac_logout: numeric_field(data, "ac_logout")
            .unwrap_or_default()
            .to_string(),
        register_mode: numeric_field(data, "register_mode")
            .unwrap_or_default()
            .to_string(),
    })
}

fn portal_mac(mac: &str) -> String {
    let mac = mac.replace(['-', ':'], "");
    if mac.is_empty() { ZERO_MAC.into() } else { mac }
}

fn nonce() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn parse_jsonp(body: &str, endpoint: RequestOperation) -> Result<Value, Error> {
    let body = body.trim().trim_start_matches('\u{feff}');
    if body.starts_with('<') {
        return Err(Error::Protocol(ProtocolError::UnexpectedHtml));
    }
    let json = if body.starts_with('{') {
        body
    } else {
        let (callback, wrapped) = body
            .split_once('(')
            .ok_or(Error::Protocol(ProtocolError::CallbackMismatch))?;
        let callback = callback.trim_end();
        // The new portal pads the numeric suffix of dr1 to four digits.
        // The status endpoint echoes the original callback without padding.
        if callback != CALLBACK
            && !(endpoint != RequestOperation::Status && callback == PADDED_PORTAL_CALLBACK)
        {
            return Err(Error::Protocol(ProtocolError::CallbackMismatch));
        }
        wrapped
            .trim_end_matches(';')
            .trim_end()
            .strip_suffix(')')
            .ok_or(Error::Protocol(ProtocolError::InvalidJsonp))?
            .trim()
    };
    let value: Value =
        serde_json::from_str(json).map_err(|_| Error::Protocol(ProtocolError::InvalidJson))?;
    if !value.is_object() {
        return Err(Error::Protocol(ProtocolError::ExpectedObject));
    }
    Ok(value)
}

fn text_field(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| match &value[*name] {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn parse_status(value: &Value) -> Result<Status, Error> {
    let ip = ["v46ip", "v4ip", "userip", "user_ip", "wlan_user_ip"]
        .into_iter()
        .filter_map(|name| value[name].as_str()?.parse::<Ipv4Addr>().ok())
        .find(|ip| !ip.is_unspecified() && !ip.is_broadcast() && !ip.is_multicast())
        .ok_or(Error::MissingIp)?;
    // Dr.COM a41.js checkStatus defines result: 0 offline, 1 online. Keep
    // explicit compatibility flags first; an invalid present flag stays unknown.
    let auth = match value.get("online").or_else(|| value.get("is_login")) {
        Some(Value::Bool(true)) => AuthState::Online,
        Some(Value::Bool(false)) => AuthState::Offline,
        Some(v) if v == 1 || v == "1" => AuthState::Online,
        Some(v) if v == 0 || v == "0" => AuthState::Offline,
        None => match numeric_field(value, "result") {
            Some(1) => AuthState::Online,
            Some(0) => AuthState::Offline,
            _ => AuthState::Unknown,
        },
        _ => AuthState::Unknown,
    };
    Ok(Status {
        auth,
        ip,
        ipv6: text_field(value, &["wlan_user_ipv6", "myv6ip"]).unwrap_or_default(),
        mac: text_field(value, &["wlan_user_mac", "mac", "ss4"]).unwrap_or_default(),
        vlan: text_field(value, &["wlan_vlan_id", "vlan", "vid"]).unwrap_or_else(|| "1".into()),
        ac_ip: text_field(value, &["wlan_ac_ip", "ac_ip"]),
        ac_name: text_field(value, &["wlan_ac_name", "acname"]),
    })
}

fn numeric_field(value: &Value, name: &str) -> Option<i64> {
    value[name]
        .as_i64()
        .or_else(|| value[name].as_str()?.parse().ok())
}

fn check_action(value: &Value, secrets: &[&str]) -> Result<(), Error> {
    if value["result"] == 1 || value["result"] == "1" || value["result"] == "ok" {
        return Ok(());
    }
    if value.get("result").is_none() {
        return Err(Error::Protocol(ProtocolError::MissingResult));
    }
    // Codes come from a42.js portal_login_err, not from the status endpoint.
    let ret_code = numeric_field(value, "ret_code");
    let explanation = match ret_code {
        Some(1) => Some("账号或密码不正确"),
        Some(2) => Some("终端 IP 已经在线；无需重复登录"),
        Some(3) => Some("系统繁忙，请稍后重试"),
        Some(4) => Some("认证服务器发生未知错误"),
        Some(5) => Some("REQ_CHALLENGE 失败"),
        Some(6) => Some("REQ_CHALLENGE 超时"),
        Some(7) => Some("Radius 认证失败"),
        Some(8) => Some("Radius 认证超时"),
        Some(9) => Some("Radius 下线失败"),
        Some(10) => Some("Radius 下线超时"),
        Some(11) => Some("认证服务器发生其他错误"),
        Some(998) => Some("Portal 协议参数不完整"),
        _ => None,
    };
    let detail = text_field(value, &["msg", "message", "ret_msg"]);
    let mut message = match (explanation, detail) {
        (Some(explanation), Some(detail)) => format!("{explanation}；{detail}"),
        (Some(explanation), None) => explanation.into(),
        (None, Some(detail)) => detail,
        (None, None) => "服务器未提供错误说明".into(),
    };
    for name in ["result", "ret_code"] {
        if let Some(code) = numeric_field(value, name) {
            message.push_str(&format!(" ({name}={code})"));
        }
    }
    let lowered = message.to_lowercase();
    let credential_rejected = ret_code == Some(1)
        || ["password", "密码", "账号不存在", "account not exist"]
            .iter()
            .any(|word| lowered.contains(word));
    for secret in secrets {
        if !secret.is_empty() {
            message = message.replace(secret, "<redacted>");
        }
    }
    // Do not pass terminal control sequences from the portal into CLI output.
    message.retain(|c| !c.is_control());
    message = message.chars().take(300).collect();
    Err(Error::Rejected {
        message,
        credential_rejected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonp_is_parsed_without_executing_code() -> Result<(), Error> {
        let status = RequestOperation::Status;
        let config = RequestOperation::PageConfig;
        assert_eq!(parse_jsonp("dr1({\"result\":1});", status)?["result"], 1);
        assert_eq!(parse_jsonp("dr1({\"code\":1});", config)?["code"], 1);
        assert_eq!(parse_jsonp("dr0001({\"code\":1});", config)?["code"], 1);
        assert!(parse_jsonp("dr0001({\"result\":1});", status).is_err());
        assert!(parse_jsonp("dr1001({\"code\":1});", config).is_err());
        assert!(parse_jsonp("dr0002({\"code\":1});", config).is_err());
        assert!(parse_jsonp("dr1({});alert(1)", status).is_err());
        assert!(parse_jsonp("dr0001({});alert(1)", config).is_err());
        assert!(parse_jsonp("dr1([])", status).is_err());
        Ok(())
    }

    #[test]
    fn unfamiliar_status_is_never_treated_as_logged_out() -> Result<(), Error> {
        let data = serde_json::json!({"result":99,"v46ip":"10.0.0.8"});
        assert_eq!(parse_status(&data)?.auth, AuthState::Unknown);
        assert_eq!(
            parse_status(&serde_json::json!({"online":0,"userip":"10.0.0.8"}))?.auth,
            AuthState::Offline
        );
        assert!(parse_status(&serde_json::json!({"online":0,"userip":"0.0.0.0"})).is_err());
        Ok(())
    }

    #[test]
    fn drcom_result_is_the_authentication_flag_without_online_aliases() -> Result<(), Error> {
        for (result, auth) in [
            (serde_json::json!(1), AuthState::Online),
            (serde_json::json!("1"), AuthState::Online),
            (serde_json::json!(0), AuthState::Offline),
            (serde_json::json!("0"), AuthState::Offline),
            (serde_json::json!(true), AuthState::Unknown),
            (serde_json::json!("ok"), AuthState::Unknown),
            (serde_json::json!(null), AuthState::Unknown),
        ] {
            assert_eq!(
                parse_status(&serde_json::json!({"result":result,"v46ip":"10.0.0.8"}))?.auth,
                auth
            );
        }
        assert_eq!(
            parse_status(&serde_json::json!({"result":1,"online":null,"v46ip":"10.0.0.8"}))?.auth,
            AuthState::Unknown
        );
        assert_eq!(
            parse_status(&serde_json::json!({"result":1,"online":0,"v46ip":"10.0.0.8"}))?.auth,
            AuthState::Offline
        );
        Ok(())
    }

    #[test]
    fn rejection_codes_explain_missing_messages_and_identify_bad_credentials() -> Result<(), Error>
    {
        for (code, message, rejected) in [
            (1, "账号或密码不正确", true),
            (2, "IP 已经在线", false),
            (3, "系统繁忙", false),
            (998, "参数不完整", false),
            (9999, "服务器未提供错误说明", false),
        ] {
            let error = check_action(&serde_json::json!({"result":0,"ret_code":code}), &[])
                .err()
                .ok_or(Error::Protocol(ProtocolError::MissingResult))?
                .during(RequestOperation::Login);
            assert_eq!(error.is_credential_rejected(), rejected);
            let chain = format!("{error:?}");
            assert!(chain.contains(message));
            assert!(chain.contains(&format!("ret_code={code}")));
            assert!(error.to_string().contains("登录请求"));
        }
        Ok(())
    }

    #[test]
    fn rejects_and_redacts_password_errors() {
        let c = Credentials {
            username: "student".into(),
            password: "p@ssword".into(),
        };
        let result = check_action(
            &serde_json::json!({"result":0,"msg":"password p@ssword for student invalid"}),
            &[&c.password, &c.username],
        );
        assert!(matches!(
            &result,
            Err(Error::Rejected {
                credential_rejected: true,
                ..
            })
        ));
        assert!(!format!("{result:?}").contains("p@ssword"));
        assert!(!format!("{c:?}").contains("student"));
    }
}
