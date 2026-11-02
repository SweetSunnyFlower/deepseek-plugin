use crate::settings::AppSettings;
use axum::http::HeaderMap;

pub fn authorized(headers: &HeaderMap, settings: &AppSettings) -> bool {
    let expected = settings.token.as_str();
    ["x-api-key", "authorization"].iter().any(|name| {
        headers
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v == expected || v == format!("Bearer {expected}"))
            .unwrap_or(false)
    })
}
