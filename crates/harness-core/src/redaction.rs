//! Bounded diagnostic text. Private prompts are never a log source.
use regex::Regex;
use std::sync::{Mutex, OnceLock};
pub const MAX_LOG_BYTES: usize = 64 * 1024;
static SECRETS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// Register runtime credentials for exact masking, in addition to pattern masking.
pub fn register_secret(secret: &str) {
    if secret.is_empty() || secret.len() > 4096 {
        return;
    }
    let mut secrets = SECRETS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !secrets.iter().any(|v| v == secret) {
        if secrets.len() == 128 {
            secrets.remove(0);
        }
        secrets.push(secret.into());
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }
}

pub fn redact(input: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| Regex::new(r#"(?i)(bearer\s+[^\s"',;]+|sk-[a-z0-9_-]{8,}|(?:api[_-]?key|access[_-]?token|password|secret)\s*["']?\s*[:=]\s*["']?[^\s"',;}]+)"#).expect("static redaction expression"));
    let mut result = pattern.replace_all(input, "[REDACTED]").into_owned();
    if let Some(secrets) = SECRETS.get() {
        for secret in secrets.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            result = result.replace(secret, "[REDACTED]");
        }
    }
    if result.len() > MAX_LOG_BYTES {
        let mut end = MAX_LOG_BYTES - 12;
        while !result.is_char_boundary(end) {
            end -= 1;
        }
        result.truncate(end);
        result.push_str(" [truncated]");
    }
    result
}

pub fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => *text = redact(text),
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_json),
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "prompt"
                        | "messages"
                        | "authorization"
                        | "api_key"
                        | "password"
                        | "secret"
                        | "token"
                ) {
                    *value = "[REDACTED]".into();
                } else {
                    redact_json(value);
                }
            }
        }
        _ => {}
    }
}
