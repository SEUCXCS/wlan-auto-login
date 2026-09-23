use crate::{AuthState, Credentials, Error, Line, RequestOperation, Status};
use serde_json::Value;
use std::{
    net::Ipv4Addr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CALLBACK: &str = "dr1";
const MAX_RESPONSE: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct PortalConfig {
    pub status_url: String,
    pub action_url: String,
    pub ac_ip: String,
    pub ac_name: String,
    pub timeout: Duration,
}

impl Default for PortalConfig {
    fn default() -> Self {
        Self {
            status_url: "http://172.18.254.6/drcom/chkstatus".into(),
            action_url: "http://172.18.254.6:801/eportal/".into(),
            ac_ip: "172.18.255.6".into(),
            ac_name: "NAS".into(),
            timeout: Duration::from_secs(8),
        }
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    config: PortalConfig,
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
        let response = self
            .request(
                RequestOperation::Status,
                &self.config.status_url,
                &[("callback", CALLBACK.to_owned()), ("v", nonce())],
            )
            .await?;
        parse_status(&response).map_err(|error| error.during(RequestOperation::Status))
    }

    pub async fn login(&self, credentials: &Credentials, line: Line) -> Result<(), Error> {
        if credentials.username.trim().is_empty() || credentials.password.is_empty() {
            return Err(Error::EmptyCredentials);
        }
        let status = self.status().await?;
        let mut query = self.common_query("login", &status);
        query.extend([
            (
                "user_account",
                format!("{}{}", credentials.username.trim(), line.suffix()),
            ),
            ("user_password", credentials.password.clone()),
        ]);
        let result = self
            .request(RequestOperation::Login, &self.config.action_url, &query)
            .await;
        // Scrub the password from our owned query buffer once the request finishes.
        use zeroize::Zeroize;
        for (key, value) in &mut query {
            if *key == "user_password" {
                value.zeroize();
            }
        }
        check_action(&result?, Some(credentials))
    }

    pub async fn logout(&self) -> Result<(), Error> {
        let status = self.status().await?;
        let mut query = self.common_query("logout", &status);
        query.extend([
            ("user_account", "drcom".into()),
            ("user_password", "123".into()),
            ("ac_logout", "0".into()),
            ("register_mode", "1".into()),
            ("wlan_vlan_id", status.vlan.clone()),
        ]);
        check_action(
            &self
                .request(RequestOperation::Logout, &self.config.action_url, &query)
                .await?,
            None,
        )
    }

    fn common_query(&self, action: &str, status: &Status) -> Vec<(&'static str, String)> {
        vec![
            ("c", "Portal".into()),
            ("a", action.into()),
            ("callback", CALLBACK.into()),
            ("login_method", "1".into()),
            ("wlan_user_ip", status.ip.to_string()),
            ("wlan_user_ipv6", status.ipv6.clone()),
            ("wlan_user_mac", status.mac.clone()),
            (
                "wlan_ac_ip",
                status
                    .ac_ip
                    .clone()
                    .unwrap_or_else(|| self.config.ac_ip.clone()),
            ),
            (
                "wlan_ac_name",
                status
                    .ac_name
                    .clone()
                    .unwrap_or_else(|| self.config.ac_name.clone()),
            ),
            ("jsVersion", "3.3.3".into()),
            ("v", nonce()),
        ]
    }

    async fn request(
        &self,
        operation: RequestOperation,
        url: &str,
        query: &[(&str, String)],
    ) -> Result<Value, Error> {
        self.send_request(url, query)
            .await
            .map_err(|error| error.during(operation))
    }

    async fn send_request(&self, url: &str, query: &[(&str, String)]) -> Result<Value, Error> {
        let mut response = self
            .http
            .get(url)
            .query(query)
            .send()
            .await?
            .error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_RESPONSE {
                return Err(Error::Protocol("响应过大"));
            }
            body.extend_from_slice(&chunk);
        }
        parse_jsonp(std::str::from_utf8(&body).map_err(|_| Error::Protocol("响应不是 UTF-8"))?)
    }
}

fn nonce() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn parse_jsonp(body: &str) -> Result<Value, Error> {
    let body = body.trim().trim_start_matches('\u{feff}');
    let json = if body.starts_with('{') {
        body
    } else {
        let wrapped = body
            .strip_prefix(CALLBACK)
            .ok_or(Error::Protocol("JSONP 回调不匹配"))?
            .trim_start();
        wrapped
            .strip_prefix('(')
            .and_then(|v| v.trim_end_matches(';').trim_end().strip_suffix(')'))
            .ok_or(Error::Protocol("JSONP 包装无效"))?
            .trim()
    };
    let value: Value = serde_json::from_str(json).map_err(|_| Error::Protocol("JSON 无效"))?;
    if !value.is_object() {
        return Err(Error::Protocol("响应必须为对象"));
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
    let ip: Ipv4Addr = text_field(
        value,
        &["v46ip", "v4ip", "userip", "user_ip", "wlan_user_ip"],
    )
    .and_then(|v| v.parse().ok())
    .filter(|ip: &Ipv4Addr| !ip.is_unspecified() && !ip.is_broadcast() && !ip.is_multicast())
    .ok_or(Error::MissingIp)?;
    // Only explicit authentication flags are accepted. `result` and OS Internet
    // connectivity do not establish whether this campus session is authenticated.
    let auth = match value.get("online").or_else(|| value.get("is_login")) {
        Some(Value::Bool(true)) => AuthState::Online,
        Some(Value::Bool(false)) => AuthState::Offline,
        Some(v) if v == 1 || v == "1" => AuthState::Online,
        Some(v) if v == 0 || v == "0" => AuthState::Offline,
        _ => AuthState::Unknown,
    };
    Ok(Status {
        auth,
        ip,
        ipv6: text_field(value, &["wlan_user_ipv6", "myv6ip"]).unwrap_or_default(),
        mac: text_field(value, &["wlan_user_mac", "mac"]).unwrap_or_default(),
        vlan: text_field(value, &["wlan_vlan_id", "vlan"]).unwrap_or_else(|| "1".into()),
        ac_ip: text_field(value, &["wlan_ac_ip", "ac_ip"]),
        ac_name: text_field(value, &["wlan_ac_name", "acname"]),
    })
}

fn check_action(value: &Value, credentials: Option<&Credentials>) -> Result<(), Error> {
    if value["result"] == 1 || value["result"] == "1" || value["result"] == "ok" {
        return Ok(());
    }
    if value.get("result").is_none() {
        return Err(Error::Protocol("缺少 result 字段"));
    }
    let mut message = text_field(value, &["msg", "message", "ret_msg"])
        .unwrap_or_else(|| "服务器未提供错误说明".into());
    let lowered = message.to_lowercase();
    let credential_rejected = ["password", "密码", "账号不存在", "account not exist"]
        .iter()
        .any(|word| lowered.contains(word));
    if let Some(credentials) = credentials {
        for secret in [&credentials.password, &credentials.username] {
            if !secret.is_empty() {
                message = message.replace(secret.as_str(), "<redacted>");
            }
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
        assert_eq!(parse_jsonp("dr1({\"result\":1});")?["result"], 1);
        assert!(parse_jsonp("dr2({\"result\":1});").is_err());
        assert!(parse_jsonp("dr1({});alert(1)").is_err());
        assert!(parse_jsonp("dr1([])").is_err());
        Ok(())
    }

    #[test]
    fn unfamiliar_status_is_never_treated_as_logged_out() -> Result<(), Error> {
        let data = serde_json::json!({"result":1,"v46ip":"10.0.0.8"});
        assert_eq!(parse_status(&data)?.auth, AuthState::Unknown);
        assert_eq!(
            parse_status(&serde_json::json!({"online":0,"userip":"10.0.0.8"}))?.auth,
            AuthState::Offline
        );
        assert!(parse_status(&serde_json::json!({"online":0,"userip":"0.0.0.0"})).is_err());
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
            Some(&c),
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
