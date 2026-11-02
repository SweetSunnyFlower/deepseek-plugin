use axum::http::HeaderMap;
use serde_json::Value;

pub fn cli_session_id(headers: &HeaderMap, body: &Value) -> Option<String> {
    if let Some(v) = headers.get("x-claude-code-session-id") {
        if let Ok(s) = v.to_str() {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    let user_id = body
        .get("metadata")?
        .get("user_id")?
        .as_str()?;
    let parsed: Value = serde_json::from_str(user_id).ok()?;
    let id = parsed.get("session_id")?.as_str()?.trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn prefers_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-claude-code-session-id", HeaderValue::from_static("abc"));
        let body = serde_json::json!({
            "metadata": { "user_id": "{\"session_id\":\"def\"}" }
        });
        assert_eq!(cli_session_id(&headers, &body).unwrap(), "abc");
    }

    #[test]
    fn falls_back_to_metadata_user_id() {
        let headers = HeaderMap::new();
        let body = serde_json::json!({
            "metadata": { "user_id": "{\"session_id\":\"def\"}" }
        });
        assert_eq!(cli_session_id(&headers, &body).unwrap(), "def");
    }

    #[test]
    fn returns_none_without_id() {
        let headers = HeaderMap::new();
        let body = serde_json::json!({ "metadata": { "user_id": "not-json" } });
        assert!(cli_session_id(&headers, &body).is_none());
        assert!(cli_session_id(&headers, &serde_json::json!({})).is_none());
    }

    #[test]
    fn ignores_empty_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-claude-code-session-id", HeaderValue::from_static("  "));
        let body = serde_json::json!({
            "metadata": { "user_id": "{\"session_id\":\"def\"}" }
        });
        assert_eq!(cli_session_id(&headers, &body).unwrap(), "def");
    }
}
