//! Record portal exchanges without logging raw URLs, credentials, or payloads.
use crate::{Error, ProtocolError, RequestOperation};
use reqwest::{Request, Response, header::HeaderMap};
use serde_json::{Map, Value, json};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

const REDACTED: &str = "[redacted]";
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

pub(super) struct Exchange<'a> {
    endpoint: RequestOperation,
    operation: RequestOperation,
    request_id: u64,
    started: Instant,
    secrets: &'a [&'a str],
    request: Value,
    status: Option<u16>,
    response_headers: Value,
    response_body: Value,
    bytes: usize,
}

impl<'a> Exchange<'a> {
    pub(super) fn new(
        endpoint: RequestOperation,
        operation: RequestOperation,
        secrets: &'a [&'a str],
    ) -> Self {
        Self {
            endpoint,
            operation,
            request_id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
            secrets,
            request: Value::Null,
            status: None,
            response_headers: Value::Null,
            response_body: Value::Null,
            bytes: 0,
        }
    }

    pub(super) fn request(&mut self, request: &Request) {
        let query = request
            .url()
            .query_pairs()
            .map(|(name, value)| match name.as_ref() {
                key if request_field(key) && is_secret(&value, self.secrets) => {
                    json!([name, REDACTED])
                }
                "callback" if value == "dr1" => json!([name, value]),
                "jsVersion" if matches!(value.as_ref(), "4.X" | "4.5.1") => json!([name, value]),
                "lang" if matches!(value.as_ref(), "zh" | "zh-cn") => json!([name, value]),
                "operate" if value == "portal_login" => json!([name, value]),
                "ac_logout" | "business_type" | "enable_r3" | "is_base64encode"
                | "login_method" | "mac_type" | "register_mode" | "terminal_type"
                | "wlan_vlan_id"
                    if small_number(&value) =>
                {
                    json!([name, value])
                }
                key if request_field(key) => json!([name, hidden(&value)]),
                _ => json!([REDACTED, REDACTED]),
            })
            .collect::<Vec<_>>();
        self.request = json!({
            "method": request.method().as_str(),
            "origin": request.url().origin().ascii_serialization(),
            "path": endpoint_path(self.endpoint),
            "query": query,
            "headers": headers(request.headers()),
        });
    }

    pub(super) fn response(&mut self, response: &Response) {
        self.status = Some(response.status().as_u16());
        self.response_headers = headers(response.headers());
    }

    pub(super) fn received_bytes(&mut self, bytes: usize) {
        self.bytes += bytes;
    }

    pub(super) fn payload(&mut self, value: &Value) {
        self.response_body = payload(value, self.secrets);
    }

    pub(super) fn finish(self, error: Option<&Error>) {
        let (stage, reason) = error.map_or(("complete", ""), failure);
        let response = self.status.map_or(Value::Null, |status| {
            json!({
                "status": status,
                "headers": self.response_headers,
                "body_bytes": self.bytes,
                "body": self.response_body,
            })
        });

        macro_rules! emit {
            ($target:literal, $level:expr) => {
                tracing::event!(
                    target: $target,
                    $level,
                    request_id = self.request_id,
                    operation = operation_name(self.operation),
                    endpoint = endpoint_path(self.endpoint),
                    elapsed_ms = self.started.elapsed().as_millis() as u64,
                    stage,
                    reason,
                    request = %self.request,
                    response = %response,
                    "Portal HTTP exchange"
                )
            };
        }

        macro_rules! record {
            ($target:literal) => {
                if error.is_some() {
                    emit!($target, tracing::Level::WARN);
                } else {
                    emit!($target, tracing::Level::DEBUG);
                }
            };
        }

        match self.endpoint {
            RequestOperation::Status => record!("wlanlogin::portal::status"),
            RequestOperation::PageConfig => record!("wlanlogin::portal::page_config"),
            RequestOperation::Login => record!("wlanlogin::portal::login"),
            RequestOperation::Logout => record!("wlanlogin::portal::logout"),
        }
    }
}

fn operation_name(operation: RequestOperation) -> &'static str {
    match operation {
        RequestOperation::Status => "status",
        RequestOperation::PageConfig => "page_config",
        RequestOperation::Login => "login",
        RequestOperation::Logout => "logout",
    }
}

fn endpoint_path(operation: RequestOperation) -> &'static str {
    match operation {
        RequestOperation::Status => "/drcom/chkstatus",
        RequestOperation::PageConfig => "/eportal/portal/page/loadConfig",
        RequestOperation::Login => "/eportal/portal/login",
        RequestOperation::Logout => "/eportal/portal/logout",
    }
}

fn failure(error: &Error) -> (&'static str, &'static str) {
    match error {
        Error::Request { source, .. } => failure(source),
        Error::Transport(error) if error.is_status() => ("http", "http_error"),
        Error::Transport(error) if error.is_builder() => ("request", "invalid_request"),
        Error::Transport(error) if error.is_timeout() => ("transport", "timeout"),
        Error::Transport(error) if error.is_connect() => ("transport", "connection_failed"),
        Error::Transport(error) if error.is_body() => ("response", "body_read_failed"),
        Error::Transport(_) => ("transport", "request_failed"),
        Error::Protocol(reason) => match reason {
            ProtocolError::BodyTooLarge => ("response", "body_too_large"),
            ProtocolError::InvalidUtf8 => ("decode", "invalid_utf8"),
            ProtocolError::UnexpectedHtml => ("decode", "unexpected_html"),
            ProtocolError::CallbackMismatch => ("decode", "invalid_jsonp_callback"),
            ProtocolError::InvalidJsonp => ("decode", "invalid_jsonp"),
            ProtocolError::InvalidJson => ("decode", "invalid_json"),
            ProtocolError::ExpectedObject => ("decode", "expected_json_object"),
            ProtocolError::MissingResult => ("response", "missing_result"),
            ProtocolError::PageConfigRejected => ("portal", "page_config_rejected"),
            ProtocolError::MissingProgramIndex => ("response", "missing_program_index"),
            ProtocolError::MissingPageIndex => ("response", "missing_page_index"),
        },
        Error::MissingIp => ("response", "missing_device_ipv4"),
        Error::Rejected { .. } => ("portal", "operation_rejected"),
        Error::LogoutUnconfirmed { .. } => ("status", "logout_unconfirmed"),
        Error::EmptyCredentials => ("request", "empty_credentials"),
    }
}

fn request_field(name: &str) -> bool {
    matches!(
        name,
        "callback"
            | "jsVersion"
            | "lang"
            | "operate"
            | "v"
            | "ac_logout"
            | "business_type"
            | "enable_r3"
            | "is_base64encode"
            | "login_method"
            | "mac_type"
            | "register_mode"
            | "terminal_type"
            | "wlan_vlan_id"
            | "authex_enable"
            | "gw_id"
            | "page_index"
            | "program_index"
            | "rcn"
            | "user_account"
            | "user_agent"
            | "user_password"
            | "uuid"
            | "wlan_ac_ip"
            | "wlan_ac_name"
            | "wlan_ap_mac"
            | "wlan_user_areaid"
            | "wlan_user_ip"
            | "wlan_user_ipv6"
            | "wlan_user_mac"
            | "wlan_user_ssid"
    )
}

fn small_number(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn hidden(value: &str) -> &str {
    if value.is_empty() { "" } else { REDACTED }
}

fn is_secret(value: &str, secrets: &[&str]) -> bool {
    !value.is_empty() && secrets.contains(&value)
}

fn headers(input: &HeaderMap) -> Value {
    let mut result = Map::new();
    let mut redacted = 0;
    for (name, value) in input {
        let name = name.as_str();
        let value = value.to_str().unwrap_or_default();
        let safe = match name {
            "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "referer"
            | "host"
            | "location"
            | "server"
            | "date"
            | "user-agent" => REDACTED,
            "accept" => {
                if value == "*/*" {
                    value
                } else {
                    REDACTED
                }
            }
            "content-type" => match value.split(';').next() {
                Some(
                    value @ ("text/html"
                    | "text/plain"
                    | "application/json"
                    | "application/javascript"
                    | "text/javascript"
                    | "application/octet-stream"),
                ) => value,
                _ => REDACTED,
            },
            "content-encoding" | "transfer-encoding" => match value {
                "identity" | "gzip" | "deflate" | "br" | "zstd" | "chunked" => value,
                _ => REDACTED,
            },
            "content-length"
                if value.len() <= 5 && value.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                value
            }
            _ => {
                redacted += 1;
                continue;
            }
        };
        let field = result.entry(name.to_owned()).or_insert_with(|| json!([]));
        if let Value::Array(values) = field {
            values.push(json!(safe));
        }
    }
    if redacted > 0 {
        result.insert("redacted_headers".into(), json!(redacted));
    }
    Value::Object(result)
}

fn payload(value: &Value, secrets: &[&str]) -> Value {
    let Some(fields) = value.as_object() else {
        return json!(REDACTED);
    };
    let mut safe = Map::new();
    let mut redacted = 0;
    for (name, value) in fields {
        let field = match name.as_str() {
            "data" => payload(value, secrets),
            "list" => value.as_array().map_or_else(
                || json!(REDACTED),
                |entries| {
                    Value::Array(
                        entries
                            .iter()
                            .map(|entry| payload(entry, secrets))
                            .collect(),
                    )
                },
            ),
            "result" if value.as_str() == Some("ok") && !is_secret("ok", secrets) => value.clone(),
            "result"
            | "online"
            | "is_login"
            | "code"
            | "ret_code"
            | "total"
            | "wlan_vlan_id"
            | "vlan"
            | "vid"
            | "login_method"
            | "account_prefix"
            | "ac_logout"
            | "register_mode"
            | "enable_r3"
            | "check_online_method" => {
                let safe_string = value
                    .as_str()
                    .is_some_and(|value| small_number(value) && !is_secret(value, secrets));
                let safe_integer = value.as_u64().is_some_and(|number| {
                    number <= 9999 && !is_secret(&number.to_string(), secrets)
                });
                if value.is_boolean() || safe_string || safe_integer {
                    value.clone()
                } else {
                    json!(REDACTED)
                }
            }
            "msg" | "message" | "ret_msg" => match value.as_str() {
                Some(
                    message @ ("Portal协议认证成功！"
                    | "Portal协议认证超时！"
                    | "终端IP已在线！"
                    | "Radius注销成功！"
                    | "获取用户在线信息数据为空！"
                    | "注销失败，可能不在线或者页面已过期，请刷新页面重试！"),
                ) if !is_secret(message, secrets) => json!(message),
                _ => json!(REDACTED),
            },
            "uid" | "v46ip" | "v4ip" | "myv6ip" | "ss4" | "ss5" | "password" | "token"
            | "cookie" | "session" | "account_suffix" => {
                json!(hidden(value.as_str().unwrap_or(REDACTED)))
            }
            key if request_field(key) => json!(hidden(value.as_str().unwrap_or(REDACTED))),
            _ => {
                redacted += 1;
                continue;
            }
        };
        safe.insert(name.clone(), field);
    }
    if redacted > 0 {
        safe.insert("redacted_fields".into(), json!(redacted));
    }
    Value::Object(safe)
}
