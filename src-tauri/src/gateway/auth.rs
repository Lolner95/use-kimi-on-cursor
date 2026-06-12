use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::{json, Value};

/// Extract an API key from headers the way OpenAI clients (including Cursor) send it.
pub fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(auth) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let auth = auth.trim();
        if let Some(rest) = auth.strip_prefix("Bearer ") {
            return Some(rest.trim().to_string());
        }
        if let Some(rest) = auth.strip_prefix("bearer ") {
            return Some(rest.trim().to_string());
        }
        if auth.starts_with("sk-") {
            return Some(auth.to_string());
        }
    }

    for header_name in ["api-key", "x-api-key", "x-openai-api-key"] {
        if let Some(key) = headers.get(header_name).and_then(|v| v.to_str().ok()) {
            let key = key.trim();
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }

    None
}

pub fn gateway_key_matches(provided: &str, expected: &str) -> bool {
    provided.trim() == expected.trim()
}

/// OpenAI-compatible 401 body so Cursor's verifier accepts the response shape.
pub fn openai_invalid_key_response() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": {
                "message": "Incorrect API key provided. You can find your API key in Kimi Cursor Gateway.",
                "type": "invalid_request_error",
                "param": null,
                "code": "invalid_api_key"
            }
        })),
    )
}
