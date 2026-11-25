//! Render redacted HTTP messages independently of the portal response parser.
mod redact;
use crate::{Error, ProtocolError, RequestOperation};
use reqwest::{
    Request, Response,
    header::{ACCEPT, HOST, HeaderMap, SET_COOKIE},
};
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
use zeroize::Zeroizing;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

pub(super) struct Exchange<'a> {
    endpoint: RequestOperation,
    operation: RequestOperation,
    request_id: u64,
    started: Instant,
    secrets: &'a [&'a str],
    redaction_values: Vec<Zeroizing<String>>,
    request: Value,
    status: Option<u16>,
    response_head: String,
    response_body: String,
    bytes: usize,
    body_complete: bool,
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
            redaction_values: Vec::new(),
            request: Value::Null,
            status: None,
            response_head: String::new(),
            response_body: String::new(),
            bytes: 0,
            body_complete: false,
        }
    }

    pub(super) fn request(&mut self, request: &Request) {
        let url = request.url();
        self.redaction_values.extend(
            url.query_pairs()
                .filter(|(name, value)| redact::sensitive(name) && !value.is_empty())
                .map(|(_, value)| Zeroizing::new(value.into_owned())),
        );
        let secrets = self.redaction_values();
        let origin = url.origin().ascii_serialization();
        let target = redact::target(url, &secrets);
        let mut head = format!("{} {target} {:?}\r\n", request.method(), request.version());
        if !request.headers().contains_key(HOST)
            && let Some((_, authority)) = origin.split_once("://")
        {
            head.push_str(&format!("host: {authority}\r\n"));
        }
        if !request.headers().contains_key(ACCEPT) {
            head.push_str("accept: */*\r\n");
        }
        append_headers(&mut head, request.headers(), &secrets);
        self.request = json!({ "origin": origin, "head": head });
    }

    pub(super) fn response(&mut self, response: &Response) {
        self.redaction_values.extend(
            response
                .headers()
                .get_all(SET_COOKIE)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .filter_map(|value| value.split(';').next()?.split_once('='))
                .map(|(_, value)| value.trim().trim_matches('"'))
                .filter(|value| !value.is_empty())
                .map(|value| Zeroizing::new(value.to_owned())),
        );
        let status = response.status();
        self.status = Some(status.as_u16());
        let mut head = format!(
            "{:?} {} {}\r\n",
            response.version(),
            status.as_u16(),
            status.canonical_reason().unwrap_or_default()
        );
        append_headers(&mut head, response.headers(), &self.redaction_values());
        self.response_head = head;
    }

    pub(super) fn received_bytes(&mut self, bytes: usize) {
        self.bytes += bytes;
    }

    pub(super) fn body(&mut self, bytes: &[u8], complete: bool) {
        self.body_complete = complete;
        // Keep text verbatim before JSONP parsing. Binary data can contain
        // opaque secrets that a text redactor cannot inspect.
        self.response_body = match std::str::from_utf8(bytes) {
            Ok(text) if complete => redact::text(text, &self.redaction_values()),
            Ok(text) => {
                let prefix = text.rsplit_once('\n').map_or("", |(prefix, _)| prefix);
                let mut redacted = redact::text(prefix, &self.redaction_values());
                if !redacted.is_empty() {
                    redacted.push('\n');
                }
                redacted.push_str("[incomplete response tail]");
                redacted
            }
            Err(_) => "[non-UTF-8 response body]".into(),
        };
    }

    fn redaction_values(&self) -> Vec<&str> {
        self.secrets
            .iter()
            .copied()
            .chain(self.redaction_values.iter().map(|value| value.as_str()))
            .collect()
    }

    pub(super) fn finish(self, error: Option<&Error>) {
        let (stage, reason) = error.map_or(("complete", ""), failure);
        let response = self.status.map_or(Value::Null, |status| {
            json!({
                "status": status,
                "head": self.response_head,
                "body_bytes": self.bytes,
                "body_complete": self.body_complete,
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

fn append_headers(head: &mut String, headers: &HeaderMap, secrets: &[&str]) {
    for (name, value) in headers {
        let value = value.to_str().unwrap_or("[non-ASCII header]");
        let value = redact::headers_value(name.as_str(), value, secrets);
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
}
