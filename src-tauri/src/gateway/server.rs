use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Multipart, Path, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::{any, get, post},
    Router,
};
use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, RwLock};
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::auth::{extract_api_key, gateway_key_matches, openai_invalid_key_response};
use super::metrics::MetricsStore;
use super::sanitizer::{sanitize_request, SanitizerConfig};
use super::responses_adapter::repair_tool_call_ids;
use super::usage_store::{SseUsageScanner, TokenUsageStore};
use futures::StreamExt;
use crate::config::{AppSettings, MAX_CONTEXT_TOKENS, MOONSHOT_API_URL, MOONSHOT_BASE_URL, MOONSHOT_EMBEDDINGS_URL, MOONSHOT_FILES_URL};
use crate::logging::{append_log, redact_secrets};

const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone)]
pub struct GatewayContext {
    pub settings: Arc<RwLock<AppSettings>>,
    pub metrics: MetricsStore,
    pub usage: TokenUsageStore,
    pub public_url: Arc<RwLock<Option<String>>>,
    pub logs_dir: PathBuf,
    pub http_client: Client,
    pub bound_port: u16,
}

pub struct GatewayServer {
    shutdown_tx: Option<oneshot::Sender<()>>,
    port: u16,
}

impl GatewayServer {
    pub async fn start(mut ctx: GatewayContext, port: u16) -> Result<Self, String> {
        ctx.bound_port = port;
        let app = build_router(ctx.clone());

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("Could not start local gateway on port {port}: {e}"))?;

        ctx.metrics.mark_started();
        info!("Gateway listening on http://127.0.0.1:{port}");

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            if let Err(e) = server.await {
                error!("Gateway server error: {e}");
            }
        });

        Ok(Self {
            shutdown_tx: Some(shutdown_tx),
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn build_router(ctx: GatewayContext) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/dashboard", get(dashboard_handler))
        .route("/v1/models", get(models_handler).post(models_handler))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/responses", post(chat_completions_handler))
        .route("/v1/completions", post(completions_handler))
        .route("/v1/embeddings", post(embeddings_handler))
        .route("/v1/audio/transcriptions", post(audio_transcriptions_handler))
        .route("/v1/uploads", post(uploads_handler))
        .route("/v1/batches", get(list_batches_handler).post(create_batch_handler))
        .route("/v1/batches/{batch_id}", get(retrieve_batch_handler))
        .route("/v1/batches/{batch_id}/cancel", post(cancel_batch_handler))
        .route("/v1/files", get(list_files_handler).post(upload_file_handler))
        .route("/v1/files/{file_id}", get(retrieve_file_handler).delete(delete_file_handler))
        .route("/v1/files/{file_id}/content", get(retrieve_file_content_handler))
        // Catch-all for any other /v1/* endpoints Cursor might send (assistants, threads, etc.)
        .route("/v1/{*path}", any(generic_proxy_handler))
        .layer(CorsLayer::permissive())
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .with_state(ctx)
}

async fn health_handler(State(ctx): State<GatewayContext>) -> Json<Value> {
    let settings = ctx.settings.read().await;
    let metrics = ctx.metrics.snapshot();
    let usage = ctx.usage.snapshot();
    let public_root = ctx.public_url.read().await.clone();
    let local_base = format!("http://127.0.0.1:{}/v1", ctx.bound_port);
    let public_base = public_root
        .as_ref()
        .map(|u| format!("{u}/v1"))
        .unwrap_or_default();

    Json(json!({
        "ok": true,
        "app": "Kimi Cursor Gateway",
        "timestamp": Utc::now().to_rfc3339(),
        "publicRootUrl": public_root.unwrap_or_default(),
        "publicBaseUrl": public_base,
        "localBaseUrl": local_base,
        "model": settings.alias_model,
        "realModel": settings.real_model,
        "metrics": metrics,
        "usage": usage,
    }))
}

async fn models_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await;
    if let Some(provided) = extract_api_key(&headers) {
        if !gateway_key_matches(&provided, &settings.gateway_key) {
            return openai_invalid_key_response().into_response();
        }
    }

    Json(json!({
        "object": "list",
        "data": [
            { "id": "gpt-5-high-max", "object": "model", "owned_by": "openai", "context_length": 262144, "capabilities": { "vision": true, "function_calling": true, "json_mode": true } },
            { "id": "gpt-5.5-high", "object": "model", "owned_by": "openai", "context_length": 262144, "capabilities": { "vision": true, "function_calling": true, "json_mode": true } },
            { "id": "gpt-4-turbo", "object": "model", "owned_by": "openai", "context_length": 262144, "capabilities": { "vision": true, "function_calling": true, "json_mode": true } },
            { "id": "gpt-4o", "object": "model", "owned_by": "openai", "context_length": 262144, "capabilities": { "vision": true, "function_calling": true, "json_mode": true } },
            { "id": "kimi-k2.6", "object": "model", "owned_by": "moonshot", "context_length": 262144, "capabilities": { "vision": true, "function_calling": true, "json_mode": true } }
        ]
    }))
    .into_response()
}

async fn chat_completions_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let request_id = Uuid::new_v4().to_string();
    let started = Instant::now();
    ctx.metrics.record_request();

    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        let msg = "Incorrect API key. Use the gateway key from Kimi Cursor Gateway (starts with sk-kimi-), not your Moonshot key.";
        ctx.metrics.record_error(
            Some(401),
            started.elapsed().as_millis() as u64,
            msg.to_string(),
        );
        append_log(&ctx.logs_dir, "errors.log", &format!("[{request_id}] {msg}"));
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        Ok(None) => {
            let msg = "Moonshot API key is not configured. Open Kimi Cursor Gateway and add your key.";
            ctx.metrics.record_error(
                Some(500),
                started.elapsed().as_millis() as u64,
                msg.to_string(),
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": msg } })),
            )
                .into_response();
        }
        Err(_) => {
            let msg = "Could not read your saved Moonshot key. Re-enter it in Settings.";
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": msg } })),
            )
                .into_response();
        }
    };

    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("Request body is not valid JSON: {e}");
            ctx.metrics.record_error(
                Some(400),
                started.elapsed().as_millis() as u64,
                msg.clone(),
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": { "message": msg } })),
            )
                .into_response();
        }
    };

    let input_count = match parsed.get("input") {
        Some(Value::Array(arr)) => arr.len(),
        Some(Value::String(_)) => 1,
        _ => 0,
    };
    let message_count = parsed
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let tool_count = parsed
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let has_instructions = parsed
        .get("instructions")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());

    // Cursor sends stream:true and expects an SSE response. We force non-streaming
    // upstream (cleaner with Kimi thinking mode) but must reply in the format the
    // client asked for, or Cursor hangs/fails parsing.
    let client_wants_stream = parsed
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let sanitizer_config = SanitizerConfig {
        real_model: settings.real_model.clone(),
        force_non_streaming: settings.force_non_streaming,
        thinking_disabled: settings.thinking_disabled,
        sanitize_tools: settings.sanitize_tools,
        max_tokens_default: settings.max_tokens_default,
        max_tokens_cap: MAX_CONTEXT_TOKENS,
        inject_reasoning_placeholder: settings.inject_reasoning_placeholder,
    };
    let mut sanitized = sanitize_request(parsed, &sanitizer_config);

    let final_message_count = sanitized
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let adapt_line = format!(
        "[{request_id}] shape input_items={input_count} messages_in={message_count} messages_out={final_message_count} tools={tool_count} instructions={has_instructions}"
    );
    append_log(&ctx.logs_dir, "adapt.log", &adapt_line);

    // Stream control: when Cursor asks for streaming, stream from Kimi directly so
    // reasoning + content tokens render live. Otherwise buffer a single completion.
    if let Some(obj) = sanitized.as_object_mut() {
        if client_wants_stream {
            obj.insert("stream".into(), Value::Bool(true));
            obj.insert("stream_options".into(), json!({ "include_usage": true }));
        } else {
            obj.insert("stream".into(), Value::Bool(false));
            obj.remove("stream_options");
        }
    }

    let mut request = ctx
        .http_client
        .post(MOONSHOT_API_URL)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .header("Content-Type", "application/json")
        .json(&sanitized);

    // Buffered (non-streaming) reasoning responses can take a while; cap the wait.
    // Streaming responses must NOT have a total timeout or long generations get cut.
    if !client_wants_stream {
        request = request.timeout(Duration::from_secs(240));
    }

    let response = request.send().await;

    let latency = started.elapsed().as_millis() as u64;

    match response {
        Ok(resp) => {
            let status = resp.status();
            let status_code = status.as_u16();

            // True streaming passthrough: pipe Kimi's SSE straight to Cursor so the
            // thinking + content tokens stream live instead of blocking until done.
            if status.is_success() && client_wants_stream {
                let log_line = format!(
                    "[{request_id}] status={status_code} latency_ms={latency} messages_in={message_count} messages_out={final_message_count} tools={tool_count} model={} stream=passthrough",
                    settings.alias_model
                );
                append_log(&ctx.logs_dir, "requests.log", &log_line);
                ctx.metrics.record_success(status_code, latency);

                // Buffered scanner: SSE lines are split across TCP chunks, so
                // per-chunk parsing misses the final usage payload. The scanner
                // reassembles lines and records token usage exactly once.
                let mut usage_scanner = SseUsageScanner::new(
                    ctx.usage.clone(),
                    request_id.clone(),
                    settings.alias_model.clone(),
                    started,
                );
                let upstream = resp.bytes_stream().map(move |chunk| {
                    if let Ok(ref bytes) = chunk {
                        usage_scanner.feed(bytes);
                    }
                    chunk.map_err(std::io::Error::other)
                });
                return (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/event-stream"),
                        (header::CACHE_CONTROL, "no-cache"),
                        (header::CONNECTION, "keep-alive"),
                    ],
                    Body::from_stream(upstream),
                )
                    .into_response();
            }

            let text = resp.text().await.unwrap_or_default();

            let log_line = format!(
                "[{request_id}] status={status_code} latency_ms={latency} messages_in={message_count} messages_out={final_message_count} tools={tool_count} model={}",
                settings.alias_model
            );
            append_log(&ctx.logs_dir, "requests.log", &log_line);

            if status.is_success() {
                ctx.metrics.record_success(status_code, latency);
                let body: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
                if let Some(usage) = body.get("usage") {
                    ctx.usage.record_from_value(
                        usage,
                        &request_id,
                        &settings.alias_model,
                        latency,
                    );
                    let usage_line = format!(
                        "[{request_id}] tokens prompt={} completion={} total={}",
                        usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        usage
                            .get("completion_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        usage.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    );
                    append_log(&ctx.logs_dir, "tokens.log", &usage_line);
                }
                if client_wants_stream {
                    let sse = completion_to_sse(&body);
                    (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, "text/event-stream"),
                            (header::CACHE_CONTROL, "no-cache"),
                            (header::CONNECTION, "keep-alive"),
                        ],
                        sse,
                    )
                        .into_response()
                } else {
                    (
                        StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK),
                        Json(body),
                    )
                        .into_response()
                }
            } else {
                // Automatic retry for tool_call_id mismatch: Moonshot 400 with
                // "tool_call_id" / "not found" often means Cursor sent mismatched
                // IDs between function_call and function_call_output items.
                // We rebuild the messages array forcing every tool_call_id to
                // pair with its nearest preceding assistant tool_calls entry.
                let is_tool_id_error = status_code == 400
                    && text.contains("tool_call_id")
                    && (text.contains("not found") || text.contains("not match"));

                if is_tool_id_error {
                    if let Value::Object(ref mut obj) = sanitized.clone() {
                        if repair_tool_call_ids(obj) {
                            warn!("[{request_id}] Retrying after repairing tool_call_id mismatch");
                            append_log(&ctx.logs_dir, "adapt.log", &format!("[{request_id}] RETRY tool_call_id repair applied"));
                            let retry_request = ctx
                                .http_client
                                .post(MOONSHOT_API_URL)
                                .header("Authorization", format!("Bearer {moonshot_key}"))
                                .header("Content-Type", "application/json")
                                .json(&Value::Object(obj.clone()));
                            let retry_response = retry_request.send().await;
                            if let Ok(retry_resp) = retry_response {
                                let retry_status = retry_resp.status();
                                let retry_code = retry_status.as_u16();
                                if retry_status.is_success() && client_wants_stream {
                                    let log_line = format!(
                                        "[{request_id}] RETRY status={retry_code} messages_in={message_count} messages_out={final_message_count} tools={tool_count} model={} stream=passthrough",
                                        settings.alias_model
                                    );
                                    append_log(&ctx.logs_dir, "requests.log", &log_line);
                                    ctx.metrics.record_success(retry_code, started.elapsed().as_millis() as u64);
                                    let mut usage_scanner = SseUsageScanner::new(
                                        ctx.usage.clone(),
                                        request_id.clone(),
                                        settings.alias_model.clone(),
                                        started,
                                    );
                                    let upstream = retry_resp.bytes_stream().map(move |chunk| {
                                        if let Ok(ref bytes) = chunk {
                                            usage_scanner.feed(bytes);
                                        }
                                        chunk.map_err(std::io::Error::other)
                                    });
                                    return (
                                        StatusCode::OK,
                                        [
                                            (header::CONTENT_TYPE, "text/event-stream"),
                                            (header::CACHE_CONTROL, "no-cache"),
                                            (header::CONNECTION, "keep-alive"),
                                        ],
                                        Body::from_stream(upstream),
                                    )
                                        .into_response();
                                }
                                let retry_text = retry_resp.text().await.unwrap_or_default();
                                let log_line = format!(
                                    "[{request_id}] RETRY status={retry_code} latency_ms={} messages_in={message_count} messages_out={final_message_count} tools={tool_count} model={}",
                                    started.elapsed().as_millis(),
                                    settings.alias_model
                                );
                                append_log(&ctx.logs_dir, "requests.log", &log_line);
                                if retry_status.is_success() {
                                    ctx.metrics.record_success(retry_code, started.elapsed().as_millis() as u64);
                                    let body: Value = serde_json::from_str(&retry_text).unwrap_or(json!({ "raw": retry_text }));
                                    if let Some(usage) = body.get("usage") {
                                        ctx.usage.record_from_value(usage, &request_id, &settings.alias_model, started.elapsed().as_millis() as u64);
                                    }
                                    if client_wants_stream {
                                        let sse = completion_to_sse(&body);
                                        return (
                                            StatusCode::OK,
                                            [
                                                (header::CONTENT_TYPE, "text/event-stream"),
                                                (header::CACHE_CONTROL, "no-cache"),
                                                (header::CONNECTION, "keep-alive"),
                                            ],
                                            sse,
                                        )
                                            .into_response();
                                    }
                                    return (
                                        StatusCode::from_u16(retry_code).unwrap_or(StatusCode::OK),
                                        Json(body),
                                    )
                                        .into_response();
                                }
                                // Retry also failed — fall through to normal error path with retry text
                                let friendly = friendly_moonshot_error(retry_code, &retry_text);
                                ctx.metrics.record_error(Some(retry_code), started.elapsed().as_millis() as u64, friendly.clone());
                                let redacted = redact_secrets(&retry_text, &[&moonshot_key]);
                                append_log(&ctx.logs_dir, "errors.log", &format!("[{request_id}] RETRY-FAIL {friendly} raw={redacted}"));
                                warn!("Moonshot retry failed: {friendly}");
                                return (
                                    StatusCode::from_u16(retry_code).unwrap_or(StatusCode::BAD_GATEWAY),
                                    Json(json!({ "error": { "message": friendly, "type": "upstream_error" } })),
                                )
                                    .into_response();
                            }
                        }
                    }
                }

                let friendly = friendly_moonshot_error(status_code, &text);
                ctx.metrics.record_error(Some(status_code), latency, friendly.clone());
                let redacted = redact_secrets(&text, &[&moonshot_key]);
                append_log(
                    &ctx.logs_dir,
                    "errors.log",
                    &format!("[{request_id}] {friendly} raw={redacted}"),
                );
                warn!("Moonshot error: {friendly}");
                (
                    StatusCode::from_u16(status_code).unwrap_or(StatusCode::BAD_GATEWAY),
                    Json(json!({ "error": { "message": friendly, "type": "upstream_error" } })),
                )
                    .into_response()
            }
        }
        Err(e) => {
            let friendly = format!(
                "Could not reach Moonshot API. Check your internet connection. ({e})"
            );
            ctx.metrics.record_error(None, latency, friendly.clone());
            append_log(&ctx.logs_dir, "errors.log", &format!("[{request_id}] {friendly}"));
            error!("Upstream request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": friendly } })),
            )
                .into_response()
        }
    }
}

/// Convert a non-streamed OpenAI/Moonshot chat completion into a Server-Sent Events
/// stream so streaming clients (Cursor) receive the format they requested.
pub fn completion_to_sse(completion: &Value) -> String {
    let id = completion
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("chatcmpl-kimi")
        .to_string();
    let created = completion
        .get("created")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| Utc::now().timestamp());
    let model = completion
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("kimi-k2.6")
        .to_string();

    let choice = completion
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first());
    let message = choice.and_then(|c| c.get("message"));
    let finish_reason = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|f| f.as_str())
        .unwrap_or("stop")
        .to_string();

    let content = message
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let tool_calls = message.and_then(|m| m.get("tool_calls")).cloned();

    let mut out = String::new();

    // First chunk: role + content (+ tool_calls if present).
    let mut delta = json!({ "role": "assistant", "content": content });
    if let Some(calls) = tool_calls {
        if let Some(arr) = calls.as_array() {
            let indexed: Vec<Value> = arr
                .iter()
                .enumerate()
                .map(|(i, call)| {
                    let mut c = call.clone();
                    if let Some(obj) = c.as_object_mut() {
                        obj.insert("index".into(), json!(i));
                    }
                    c
                })
                .collect();
            delta["tool_calls"] = Value::Array(indexed);
        }
    }

    let first = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{ "index": 0, "delta": delta, "finish_reason": Value::Null }]
    });
    out.push_str(&format!("data: {first}\n\n"));

    // Final chunk: finish reason, then usage passthrough, then [DONE].
    let mut final_chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{ "index": 0, "delta": {}, "finish_reason": finish_reason }]
    });
    if let Some(usage) = completion.get("usage") {
        final_chunk["usage"] = usage.clone();
    }
    out.push_str(&format!("data: {final_chunk}\n\n"));
    out.push_str("data: [DONE]\n\n");

    out
}

fn friendly_moonshot_error(status: u16, body: &str) -> String {
    match status {
        401 | 403 => {
            "Moonshot rejected your API key. Open Settings and paste a valid Kimi Open Platform key."
                .to_string()
        }
        429 => "Moonshot rate limit reached. Wait a moment and try again.".to_string(),
        400 if body.contains("tool_call_id") || (body.contains("tool call") && body.contains("not found")) => {
            "Kimi rejected a mismatched tool call in the conversation history. The gateway now repairs tool-call pairing automatically — retry the request. If this repeats, restart the gateway and export diagnostics.".to_string()
        }
        400 if body.contains("image") || body.contains("vision") || body.contains("file") => {
            format!("Kimi rejected the image or file upload. Response: {}", body)
        }
        400 if body.contains("json schema") || body.contains("$defs") || body.contains("$ref") => {
            "Cursor sent a tool schema Kimi does not accept. The gateway tried to repair it. If this repeats, export diagnostics.".to_string()
        }
        400 if body.contains("reasoning_content") => {
            "Cursor dropped Kimi reasoning history during tool calls. Enable 'Inject reasoning placeholder' in Advanced settings (on by default).".to_string()
        }
        400 if body.contains("temperature") || body.contains("presence_penalty") || body.contains("frequency_penalty") => {
            "Cursor sent OpenAI sampling parameters Kimi rejects. Restart the gateway to pick up the latest sanitizer.".to_string()
        }
        400 if body.contains("developer") || body.contains("tokenization failed") => {
            "Cursor sent an unsupported message format. Restart the gateway — developer roles are auto-converted.".to_string()
        }
        400 if body.contains("function name is invalid") => {
            "Cursor sent tool names Kimi rejects (dots/slashes in MCP tool names). The gateway now sanitizes these — restart the gateway to apply.".to_string()
        }
        _ => format!("Moonshot returned an error (HTTP {status}). Try again or export diagnostics."),
    }
}

async fn list_files_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let resp = ctx
        .http_client
        .get(MOONSHOT_FILES_URL)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<Value>(&body) {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(json)).into_response()
            } else {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response()
            }
        }
        Err(e) => {
            error!("Files list request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": format!("Upstream error: {e}") } })),
            )
                .into_response()
        }
    }
}

async fn upload_file_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let mut file_bytes: Option<Bytes> = None;
    let mut file_name: Option<String> = None;
    let mut purpose = "assistants".to_string();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            file_name = field.file_name().map(|s| s.to_string());
            if let Ok(data) = field.bytes().await {
                file_bytes = Some(data);
            }
        } else if name == "purpose" {
            if let Ok(text) = field.text().await {
                purpose = text;
            }
        }
    }

    let Some(bytes) = file_bytes else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": { "message": "No file provided in multipart request." } })),
        )
            .into_response();
    };

    let part_name = file_name.unwrap_or_else(|| "upload".to_string());

    // Detect MIME type from filename extension for images, audio, etc.
    let mime_type = mime_type_from_name(&part_name);
    info!(
        "Upload file: name={part_name}, size={} bytes, mime={mime_type}, purpose={purpose}",
        bytes.len()
    );

    let part = reqwest::multipart::Part::bytes(bytes.to_vec())
        .file_name(part_name)
        .mime_str(&mime_type)
        .unwrap_or_else(|_| {
            reqwest::multipart::Part::bytes(bytes.to_vec())
                .file_name("upload")
        });

    let form = reqwest::multipart::Form::new()
        .text("purpose", purpose)
        .part("file", part);

    let resp = ctx
        .http_client
        .post(MOONSHOT_FILES_URL)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .multipart(form)
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<Value>(&body) {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(json)).into_response()
            } else {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response()
            }
        }
        Err(e) => {
            error!("File upload request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": format!("Upstream error: {e}") } })),
            )
                .into_response()
        }
    }
}

async fn retrieve_file_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let url = format!("{MOONSHOT_FILES_URL}/{file_id}");
    let resp = ctx
        .http_client
        .get(&url)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<Value>(&body) {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(json)).into_response()
            } else {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response()
            }
        }
        Err(e) => {
            error!("File retrieve request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": format!("Upstream error: {e}") } })),
            )
                .into_response()
        }
    }
}

async fn delete_file_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let url = format!("{MOONSHOT_FILES_URL}/{file_id}");
    let resp = ctx
        .http_client
        .delete(&url)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<Value>(&body) {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(json)).into_response()
            } else {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response()
            }
        }
        Err(e) => {
            error!("File delete request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": format!("Upstream error: {e}") } })),
            )
                .into_response()
        }
    }
}

async fn retrieve_file_content_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let url = format!("{MOONSHOT_FILES_URL}/{file_id}/content");
    let resp = ctx
        .http_client
        .get(&url)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status();
            let headers = r.headers().clone();
            let bytes = r.bytes().await.unwrap_or_default();

            let mut response_builder = axum::http::Response::builder().status(status);
            for (key, value) in headers.iter() {
                if key.as_str().eq_ignore_ascii_case("content-type")
                    || key.as_str().eq_ignore_ascii_case("content-disposition")
                {
                    response_builder = response_builder.header(key.as_str(), value.as_bytes());
                }
            }
            response_builder
                .body(Body::from(bytes))
                .unwrap_or_else(|_| (StatusCode::OK, Body::empty()).into_response())
                .into_response()
        }
        Err(e) => {
            error!("File content request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": format!("Upstream error: {e}") } })),
            )
                .into_response()
        }
    }
}

async fn completions_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Legacy /v1/completions maps to chat completions for Moonshot.
    // Parse body, wrap the `prompt` into a user message, then forward.
    let _request_id = Uuid::new_v4().to_string();
    let started = Instant::now();
    ctx.metrics.record_request();

    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("Request body is not valid JSON: {e}");
            ctx.metrics.record_error(Some(400), started.elapsed().as_millis() as u64, msg.clone());
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": { "message": msg } })))
                .into_response();
        }
    };

    // Convert legacy completions payload to chat completions.
    let mut chat_body = parsed.clone();
    if let Some(obj) = chat_body.as_object_mut() {
        if let Some(prompt) = obj.remove("prompt") {
            let messages = match prompt {
                Value::String(s) => vec![json!({ "role": "user", "content": s })],
                Value::Array(arr) => {
                    arr.into_iter()
                        .filter_map(|v| v.as_str().map(|s| json!({ "role": "user", "content": s })))
                        .collect()
                }
                _ => vec![json!({ "role": "user", "content": " " })],
            };
            obj.insert("messages".into(), Value::Array(messages));
        }
        // suffix is not supported; remove it.
        obj.remove("suffix");
        if !obj.contains_key("messages") {
            obj.insert("messages".into(), json!([{ "role": "user", "content": " " }]));
        }
    }

    // Apply standard sanitization.
    let sanitizer_config = SanitizerConfig {
        real_model: settings.real_model.clone(),
        force_non_streaming: settings.force_non_streaming,
        thinking_disabled: settings.thinking_disabled,
        sanitize_tools: settings.sanitize_tools,
        max_tokens_default: settings.max_tokens_default,
        max_tokens_cap: MAX_CONTEXT_TOKENS,
        inject_reasoning_placeholder: settings.inject_reasoning_placeholder,
    };
    let mut sanitized = sanitize_request(chat_body, &sanitizer_config);

    let client_wants_stream = parsed
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if let Some(obj) = sanitized.as_object_mut() {
        if client_wants_stream {
            obj.insert("stream".into(), Value::Bool(true));
            obj.insert("stream_options".into(), json!({ "include_usage": true }));
        } else {
            obj.insert("stream".into(), Value::Bool(false));
            obj.remove("stream_options");
        }
    }

    let mut request = ctx
        .http_client
        .post(MOONSHOT_API_URL)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .header("Content-Type", "application/json")
        .json(&sanitized);

    if !client_wants_stream {
        request = request.timeout(Duration::from_secs(240));
    }

    let response = request.send().await;
    let latency = started.elapsed().as_millis() as u64;

    match response {
        Ok(resp) => {
            let status = resp.status();
            let status_code = status.as_u16();
            let text = resp.text().await.unwrap_or_default();
            ctx.metrics.record_success(status_code, latency);

            if status.is_success() {
                let body: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
                (StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK), Json(body))
                    .into_response()
            } else {
                let friendly = friendly_moonshot_error(status_code, &text);
                ctx.metrics.record_error(Some(status_code), latency, friendly.clone());
                (
                    StatusCode::from_u16(status_code).unwrap_or(StatusCode::BAD_GATEWAY),
                    Json(json!({ "error": { "message": friendly, "type": "upstream_error" } })),
                )
                    .into_response()
            }
        }
        Err(e) => {
            let friendly = format!("Could not reach Moonshot API. ({e})");
            ctx.metrics.record_error(None, latency, friendly.clone());
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": friendly } })),
            )
                .into_response()
        }
    }
}

async fn embeddings_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    proxy_json_body(ctx, headers, body, MOONSHOT_EMBEDDINGS_URL).await
}

async fn audio_transcriptions_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    multipart: Multipart,
) -> impl IntoResponse {
    proxy_multipart(ctx, headers, multipart, "https://api.moonshot.ai/v1/audio/transcriptions").await
}

async fn uploads_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    proxy_json_body(ctx, headers, body, "https://api.moonshot.ai/v1/uploads").await
}

async fn list_batches_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
) -> impl IntoResponse {
    proxy_method(ctx, headers, Method::GET, "https://api.moonshot.ai/v1/batches").await
}

async fn create_batch_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    proxy_json_body(ctx, headers, body, "https://api.moonshot.ai/v1/batches").await
}

async fn retrieve_batch_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    Path(batch_id): Path<String>,
) -> impl IntoResponse {
    let url = format!("https://api.moonshot.ai/v1/batches/{batch_id}");
    proxy_method(ctx, headers, Method::GET, &url).await
}

async fn cancel_batch_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    Path(batch_id): Path<String>,
) -> impl IntoResponse {
    let url = format!("https://api.moonshot.ai/v1/batches/{batch_id}/cancel");
    proxy_json_body(ctx, headers, Bytes::new(), &url).await
}

/// Generic catch-all for any /v1/* endpoint not explicitly handled above.
async fn generic_proxy_handler(
    State(ctx): State<GatewayContext>,
    headers: HeaderMap,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    let method = req.method().clone();
    let body_bytes = match axum::body::to_bytes(req.into_body(), MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": { "message": format!("Body read error: {e}") } })),
            )
                .into_response();
        }
    };

    let url = format!("{MOONSHOT_BASE_URL}/{path}");

    if method == Method::GET || body_bytes.is_empty() {
        proxy_method(ctx, headers, method, &url).await.into_response()
    } else {
        proxy_json_body_with_method(ctx, headers, body_bytes, &url, method).await.into_response()
    }
}

/// Proxies a JSON body to an upstream URL with the same HTTP method.
async fn proxy_json_body_with_method(
    ctx: GatewayContext,
    headers: HeaderMap,
    body: Bytes,
    url: &str,
    method: Method,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let mut request_builder = ctx
        .http_client
        .request(method, url)
        .header("Authorization", format!("Bearer {moonshot_key}"));

    if !body.is_empty() {
        request_builder = request_builder.header("Content-Type", "application/json").body(body);
    }

    let resp = request_builder.send().await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<Value>(&body) {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(json)).into_response()
            } else {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response()
            }
        }
        Err(e) => {
            error!("Generic proxy request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": format!("Upstream error: {e}") } })),
            )
                .into_response()
        }
    }
}

/// Proxies a JSON body to an upstream URL via POST.
async fn proxy_json_body(
    ctx: GatewayContext,
    headers: HeaderMap,
    body: Bytes,
    url: &str,
) -> impl IntoResponse {
    proxy_json_body_with_method(ctx, headers, body, url, Method::POST).await
}

/// Proxies a request with a given HTTP method (no body, or forwarded body).
async fn proxy_method(
    ctx: GatewayContext,
    headers: HeaderMap,
    method: Method,
    url: &str,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let resp = ctx
        .http_client
        .request(method, url)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<Value>(&body) {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(json)).into_response()
            } else {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response()
            }
        }
        Err(e) => {
            error!("Method proxy request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": format!("Upstream error: {e}") } })),
            )
                .into_response()
        }
    }
}

/// Proxies multipart/form-data to an upstream URL.
/// Preserves filenames, MIME types, and handles text fields properly.
async fn proxy_multipart(
    ctx: GatewayContext,
    headers: HeaderMap,
    mut multipart: Multipart,
    url: &str,
) -> impl IntoResponse {
    let settings = ctx.settings.read().await.clone();
    let provided_key = extract_api_key(&headers).unwrap_or_default();
    if !gateway_key_matches(&provided_key, &settings.gateway_key) {
        return openai_invalid_key_response().into_response();
    }

    let moonshot_key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "Moonshot API key not configured." } })),
            )
                .into_response();
        }
    };

    let mut form = reqwest::multipart::Form::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        let file_name = field.file_name().map(|s| s.to_string());
        let content_type = field.content_type().map(|s| s.to_string());

        if let Ok(data) = field.bytes().await {
            let data_vec = data.to_vec();
            let mut part = reqwest::multipart::Part::bytes(data_vec.clone());

            // Preserve original filename so Moonshot can infer MIME type
            let fname_clone = file_name.clone();
            if let Some(fname) = fname_clone {
                part = part.file_name(fname);
            }

            // Determine MIME type: explicit Content-Type takes priority,
            // otherwise infer from filename.
            let mime_to_set: Option<String> = content_type
                .or_else(|| file_name.map(|n| mime_type_from_name(&n).to_string()));

            if let Some(mime) = mime_to_set {
                part = match part.mime_str(&mime) {
                    Ok(p) => p,
                    Err(_) => reqwest::multipart::Part::bytes(data_vec),
                };
            }

            form = form.part(name, part);
        }
    }

    let resp = ctx
        .http_client
        .post(url)
        .header("Authorization", format!("Bearer {moonshot_key}"))
        .multipart(form)
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<Value>(&body) {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(json)).into_response()
            } else {
                (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response()
            }
        }
        Err(e) => {
            error!("Multipart proxy request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": { "message": format!("Upstream error: {e}") } })),
            )
                .into_response()
        }
    }
}

/// Infer MIME type from filename extension for image, audio, text uploads.
fn mime_type_from_name(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".wav") {
        "audio/wav"
    } else if lower.ends_with(".ogg") {
        "audio/ogg"
    } else if lower.ends_with(".m4a") {
        "audio/mp4"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else if lower.ends_with(".txt") {
        "text/plain"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else {
        "application/octet-stream"
    }
}

async fn dashboard_handler(State(ctx): State<GatewayContext>) -> Html<String> {
    let settings = ctx.settings.read().await;
    let public = ctx
        .public_url
        .read()
        .await
        .clone()
        .unwrap_or_else(|| "(tunnel not ready)".to_string());
    let public_base = if public.starts_with("http") {
        format!("{public}/v1")
    } else {
        public.clone()
    };

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"/><title>Kimi Cursor Gateway</title>
<style>body{{font-family:Segoe UI,system-ui;background:#0a0b0f;color:#e8eaf0;padding:2rem}}
.card{{background:#1a1d26;border:1px solid #2a2f3d;border-radius:12px;padding:1.25rem;margin:1rem 0}}
.label{{color:#9aa3b8;font-size:.85rem}} .value{{font-family:Consolas,monospace;margin-top:.35rem}}</style></head>
<body>
<h1>Kimi Cursor Gateway</h1>
<p>Local debug dashboard. Use the desktop app for the full experience.</p>
<div class="card"><div class="label">OpenAI API Key (gateway)</div><div class="value">{}</div></div>
<div class="card"><div class="label">Base URL</div><div class="value">{}</div></div>
<div class="card"><div class="label">Model</div><div class="value">{}</div></div>
<div class="card"><div class="label">Health</div><div class="value"><a href="/health" style="color:#7c5cff">/health</a></div></div>
</body></html>"#,
        settings.gateway_key, public_base, settings.alias_model
    ))
}
