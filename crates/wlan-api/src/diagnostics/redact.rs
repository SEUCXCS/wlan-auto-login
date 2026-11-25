//! Redact values in the original portal text without reformatting the envelope.
use regex::{Captures, Regex};
use serde::de::IgnoredAny;
use std::{borrow::Cow, sync::LazyLock};
use zeroize::Zeroizing;

pub(super) const REDACTED: &str = "[redacted]";

static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:"((?:\\.|[^"\\\r\n])*)"|'((?:\\.|[^'\\\r\n])*)'|([\w-]+))\s*[:=]\s*"#)
        .unwrap_or_else(|error| panic!("invalid portal redaction pattern: {error}"))
});

static INPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<input\b(?:"[^"]*"|'[^']*'|[^"'>])*>"#)
        .unwrap_or_else(|error| panic!("invalid form input pattern: {error}"))
});

pub(super) fn sensitive(name: &str) -> bool {
    let name: String = name
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| char::from(byte.to_ascii_lowercase()))
        .collect();
    matches!(
        name.as_str(),
        "account"
            | "useraccount"
            | "username"
            | "uid"
            | "userid"
            | "ac"
            | "pwd"
            | "passwd"
            | "password"
            | "userpassword"
            | "rcn"
            | "uuid"
            | "programindex"
            | "pageindex"
            | "gwid"
            | "sid"
            | "session"
            | "phpsessid"
            | "csrf"
            | "xsrf"
            | "v46ip"
            | "v4ip"
            | "v6ip"
            | "myv6ip"
            | "userip"
            | "ip"
            | "mac"
            | "ss1"
            | "ss4"
            | "ss5"
            | "ss6"
            | "olmac"
            | "lip"
            | "ssid"
            | "wlanacip"
            | "wlanacname"
            | "wlanapmac"
            | "wlanuserareaid"
            | "wlanuserip"
            | "wlanuseripv6"
            | "wlanusermac"
            | "wlanuserssid"
    ) || [
        "cookie",
        "authorization",
        "sessionid",
        "token",
        "secret",
        "apikey",
        "password",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
}

pub(super) fn headers_value(name: &str, value: &str, secrets: &[&str]) -> String {
    if sensitive(name) {
        REDACTED.into()
    } else {
        text(value, secrets)
    }
}

pub(super) fn target(url: &reqwest::Url, secrets: &[&str]) -> String {
    let mut output = url.path().to_owned();
    if let Some(query) = url.query() {
        output.push('?');
        let mut decoded = url.query_pairs();
        for (index, pair) in query.split('&').enumerate() {
            if index > 0 {
                output.push('&');
            }
            if pair.is_empty() {
                continue;
            }
            let Some((key, value)) = decoded.next() else {
                output.push_str(REDACTED);
                continue;
            };
            if sensitive(&key) && !value.is_empty() {
                let raw_key = pair.split_once('=').map_or(pair, |(key, _)| key);
                output.push_str(raw_key);
                output.push('=');
                output.push_str(REDACTED);
            } else {
                output.push_str(pair);
            }
        }
    }
    literals(output, secrets)
}

pub(super) fn text(text: &str, secrets: &[&str]) -> String {
    let form_redacted = INPUT.replace_all(text, |capture: &Captures<'_>| redact_input(&capture[0]));
    let text = form_redacted.as_ref();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for fields in ASSIGNMENT.captures_iter(text) {
        let Some(assignment) = fields.get(0) else {
            continue;
        };
        if assignment.start() < cursor {
            continue;
        }
        let key = assignment_name(text, &fields);
        if !sensitive(&key) {
            continue;
        }

        let start = assignment.end();
        let value = &text[start..];
        let length = if fields.get(3).is_some()
            && [
                "cookie",
                "set-cookie",
                "authorization",
                "proxy-authorization",
            ]
            .iter()
            .any(|name| key.eq_ignore_ascii_case(name))
        {
            value.find(['\r', '\n']).unwrap_or(value.len())
        } else {
            value_length(value)
        };
        if length == 0 {
            continue;
        }
        output.push_str(&text[cursor..start]);
        output.push_str(replacement(value));
        cursor = start + length;
    }
    output.push_str(&text[cursor..]);
    literals(output, secrets)
}

fn assignment_name<'a>(text: &'a str, fields: &Captures<'a>) -> Cow<'a, str> {
    if let Some(key) = fields.get(1) {
        let quoted = &text[key.start() - 1..key.end() + 1];
        serde_json::from_str::<String>(quoted)
            .map(Cow::Owned)
            .unwrap_or_else(|_| Cow::Borrowed(key.as_str()))
    } else {
        fields
            .get(2)
            .or_else(|| fields.get(3))
            .map_or(Cow::Borrowed(""), |field| Cow::Borrowed(field.as_str()))
    }
}

fn redact_input(input: &str) -> String {
    let mut should_redact = false;
    let mut value_range = None;
    for attribute in ASSIGNMENT.captures_iter(input) {
        let Some(field) = attribute.get(0) else {
            continue;
        };
        let start = field.end();
        let rest = &input[start..];
        let end = start + value_length(rest);
        let value = input[start..end].trim_matches(['\'', '"']);
        match assignment_name(input, &attribute)
            .to_ascii_lowercase()
            .as_str()
        {
            "name" | "id" => should_redact |= sensitive(value),
            "type" => should_redact |= value.eq_ignore_ascii_case("password"),
            "value" => value_range = Some(start..end),
            _ => {}
        }
    }
    if should_redact && let Some(range) = value_range {
        let mut output = input.to_owned();
        output.replace_range(range.clone(), replacement(&input[range]));
        return output;
    }
    input.into()
}

fn replacement(value: &str) -> &'static str {
    match value.as_bytes().first() {
        Some(b'"' | b'{' | b'[') => "\"[redacted]\"",
        Some(b'\'') => "'[redacted]'",
        _ => REDACTED,
    }
}

fn value_length(value: &str) -> usize {
    match value.as_bytes().first() {
        Some(b'"' | b'\'') => {
            let quote = value.as_bytes()[0];
            let mut escaped = false;
            for (index, byte) in value.bytes().enumerate().skip(1) {
                if !escaped && byte == quote {
                    return index + 1;
                }
                escaped = !escaped && byte == b'\\';
            }
            value.len()
        }
        Some(b'{' | b'[') => {
            let mut decoder = serde_json::Deserializer::from_str(value).into_iter::<IgnoredAny>();
            match decoder.next() {
                Some(Ok(_)) => decoder.byte_offset(),
                _ => value.len(),
            }
        }
        _ => value
            .find(|character: char| character.is_whitespace() || ",;}])&<>".contains(character))
            .unwrap_or(value.len()),
    }
}

fn literals(mut text: String, secrets: &[&str]) -> String {
    // Replace the longest representations first so prefixes cannot split an encoded value.
    let mut values: Vec<_> = secrets
        .iter()
        .copied()
        .filter(|value| !value.is_empty())
        .collect();
    values.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
    for raw in values {
        if let Ok(json) = serde_json::to_string(raw) {
            let json = Zeroizing::new(json);
            text = text.replace(&json[1..json.len() - 1], REDACTED);
        }
        if let Ok(mut url) = reqwest::Url::parse("http://localhost/") {
            url.query_pairs_mut().append_pair("value", raw);
            if let Some(encoded) = url.query().and_then(|query| query.strip_prefix("value=")) {
                text = text.replace(encoded, REDACTED);
            }
        }
        text = text.replace(raw, REDACTED);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_keeps_the_wrapper_and_line_breaks() {
        let original = r#"dr2001({"user\u005fpassword":
  "example-password", "session":{"key":"nested-secret"}, "result":1});
PHPSESSID=example-session; Path=/"#;
        let expected = r#"dr2001({"user\u005fpassword":
  "[redacted]", "session":"[redacted]", "result":1});
PHPSESSID=[redacted]; Path=/"#;
        assert_eq!(text(original, &[]), expected);
    }
}
