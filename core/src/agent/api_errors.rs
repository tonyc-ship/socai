//! Pretty-print provider HTTP errors. Mirrors
//! `socai/agent/api_errors.py::format_api_error` but reads JSON bodies
//! directly (we don't have an SDK exception object to introspect).
//!
//! Surfaces, in this order: provider tag, status, top-level `error.message`,
//! `error.code`, `error.type`, `error.param`, `request_id`. Falls back to
//! a truncated body. Adds a hint for empty 4xx (Cloudflare-edge auth
//! rejection).

use serde_json::Value;

const EMPTY_BODY_HINT: &str = "empty response body — usually means a malformed/empty API key or auth header. Check ~/.socai/auth.json or run `socai` and use `/model` to re-enter.";

pub fn format_http_error(provider: &str, status: u16, body_text: &str) -> String {
    let mut parts: Vec<String> = vec![format!("{provider} API error"), format!("status={status}")];

    let trimmed = body_text.trim();
    let parsed: Option<Value> = if trimmed.is_empty() {
        None
    } else {
        serde_json::from_str(trimmed).ok()
    };

    let mut had_structured = false;
    if let Some(value) = parsed.as_ref() {
        had_structured = true;
        let err_obj = value
            .get("error")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_else(|| value.as_object().cloned().unwrap_or_default());
        for key in ["message", "code", "type", "param"] {
            if let Some(v) = err_obj.get(key) {
                let rendered = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if !rendered.is_empty() {
                    parts.push(format!("{key}={rendered}"));
                }
            }
        }
        if let Some(request_id) = value.get("request_id").and_then(Value::as_str) {
            parts.push(format!("request_id={request_id}"));
        }
    }

    if !had_structured && !trimmed.is_empty() {
        let snippet = if trimmed.chars().count() > 500 {
            let mut s: String = trimmed.chars().take(500).collect();
            s.push('…');
            s
        } else {
            trimmed.to_string()
        };
        parts.push(format!("body={snippet}"));
    }

    if matches!(status, 400 | 401 | 403) && trimmed.is_empty() {
        parts.push(EMPTY_BODY_HINT.to_string());
    }

    let mut seen = std::collections::HashSet::new();
    let mut cleaned: Vec<String> = Vec::new();
    for part in parts {
        if !part.is_empty() && seen.insert(part.clone()) {
            cleaned.push(part);
        }
    }
    cleaned.join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_style() {
        let body = r#"{"error":{"message":"Invalid api key","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        let out = format_http_error("openai", 401, body);
        assert!(out.contains("message=Invalid api key"));
        assert!(out.contains("code=invalid_api_key"));
        assert!(out.contains("status=401"));
    }

    #[test]
    fn parses_dashscope_arrearage() {
        let body = r#"{"error":{"message":"Access denied","type":"Arrearage","code":"Arrearage"},"request_id":"abc-123"}"#;
        let out = format_http_error("qwen", 400, body);
        assert!(out.contains("type=Arrearage"));
        assert!(out.contains("request_id=abc-123"));
    }

    #[test]
    fn empty_body_hint_for_4xx() {
        let out = format_http_error("anthropic", 401, "");
        assert!(out.contains("empty response body"));
    }
}
