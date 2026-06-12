//! Live E2E tests — run with:
//!   $env:MOONSHOT_API_KEY="sk-..."; cargo test --test live_e2e -- --ignored --nocapture

use kimi_cursor_gateway_lib::config::AppSettings;
use kimi_cursor_gateway_lib::gateway::server::{GatewayContext, GatewayServer};
use kimi_cursor_gateway_lib::gateway::{MetricsStore, TokenUsageStore};
use reqwest::Client;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

fn test_logs_dir() -> PathBuf {
    std::env::temp_dir().join("kimi-live-e2e-logs")
}

async fn start_gateway_with_key(port: u16, moonshot_key: &str) -> (GatewayServer, String) {
    let mut settings = AppSettings::default();
    settings
        .set_moonshot_key(moonshot_key)
        .expect("encrypt key");
    let gateway_key = settings.gateway_key.clone();

    let ctx = GatewayContext {
        settings: Arc::new(RwLock::new(settings)),
        metrics: MetricsStore::new(),
        usage: TokenUsageStore::new(test_logs_dir().join("usage")),
        public_url: Arc::new(RwLock::new(None)),
        logs_dir: test_logs_dir(),
        http_client: Client::new(),
        bound_port: port,
    };

    let server = GatewayServer::start(ctx, port)
        .await
        .expect("start gateway");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    (server, gateway_key)
}

fn moonshot_key() -> Option<String> {
    std::env::var("MOONSHOT_API_KEY")
        .ok()
        .filter(|k| k.starts_with("sk-"))
}

#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_moonshot_api_accepts_key() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let client = Client::new();
    let resp = client
        .get("https://api.moonshot.ai/v1/models")
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .expect("moonshot request");
    assert!(
        resp.status().is_success(),
        "moonshot models failed: {}",
        resp.status()
    );
    println!("Moonshot API key: OK");
}

#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_gateway_chat_completion_returns_assistant_message() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let port = 17420;
    let (_server, gateway_key) = start_gateway_with_key(port, &key).await;

    let client = Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "Reply with exactly: KIMI_OK" }],
            "max_tokens": 50
        }))
        .send()
        .await
        .expect("gateway chat");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("json");
    assert!(
        status.is_success(),
        "gateway chat failed {status}: {body}"
    );

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    assert!(!content.is_empty(), "empty assistant content: {body}");
    println!("Assistant reply: {content}");
    assert!(
        content.to_uppercase().contains("KIMI"),
        "unexpected reply: {content}"
    );
}

#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_gateway_sanitizes_tool_schemas_for_moonshot() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let port = 17421;
    let (_server, gateway_key) = start_gateway_with_key(port, &key).await;

    let client = Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "What is 2+2? Reply with just the number." }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "calc",
                    "strict": true,
                    "parameters": {
                        "$schema": "http://json-schema.org/draft-07/schema#",
                        "definitions": { "Num": { "type": "number" } },
                        "type": "object",
                        "properties": { "n": { "$ref": "#/definitions/Num" } }
                    }
                }
            }],
            "max_tokens": 80
        }))
        .send()
        .await
        .expect("tool schema request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("json");
    let err = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        !err.contains("#/definitions"),
        "schema sanitizer failed: {err}"
    );
    assert!(
        status.is_success() || !err.contains("json schema"),
        "moonshot rejected schema: {status} {body}"
    );
    println!("Tool schema test: status={status}");
}

/// Reproduces the MFJS validation failures Cursor subagents trigger:
/// enum-only properties (no type), anyOf with parent type, and deep nesting.
#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_gateway_accepts_mfjs_edge_case_schemas() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let port = 17426;
    let (_server, gateway_key) = start_gateway_with_key(port, &key).await;

    // Deep schema (12 levels) that must be flattened under the depth-10 cap.
    let mut deep = serde_json::json!({ "type": "string" });
    for _ in 0..12 {
        deep = serde_json::json!({ "type": "object", "properties": { "child": deep } });
    }

    let client = Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "Say hi." }],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "set_mode",
                        "parameters": {
                            "type": "object",
                            "properties": { "mode": { "enum": ["start", "end"] } }
                        }
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "pick_value",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "val": {
                                    "type": "string",
                                    "anyOf": [{ "type": "string" }, { "type": "number" }]
                                }
                            }
                        }
                    }
                },
                {
                    "type": "function",
                    "function": { "name": "deep_tool", "parameters": deep }
                }
            ],
            "max_tokens": 80
        }))
        .send()
        .await
        .expect("mfjs request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("json");
    let err = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        !err.contains("moonshot flavored json schema"),
        "MFJS normalization failed: {err}"
    );
    assert!(status.is_success(), "moonshot rejected MFJS schemas: {status} {body}");
    println!("MFJS edge-case schemas: status={status}");
}

/// Simulates what Cursor Agent sends: sampling params, developer role, multi-turn tools.
#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_cursor_agent_multiturn_with_cursor_payload() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let port = 17422;
    let (_server, gateway_key) = start_gateway_with_key(port, &key).await;
    let client = Client::new();

    // Turn 1 — Cursor-like first request
    let turn1 = serde_json::json!({
        "model": "gpt-4-turbo",
        "temperature": 0,
        "presence_penalty": 0,
        "frequency_penalty": 0,
        "max_completion_tokens": 4096,
        "stream": false,
        "messages": [
            { "role": "developer", "content": "You are a coding agent." },
            { "role": "user", "content": "Use the read_file tool to read package.json" }
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": "read_file",
                "strict": true,
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        }],
        "tool_choice": "auto"
    });

    let resp1 = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&turn1)
        .send()
        .await
        .expect("turn1");
    let status1 = resp1.status();
    let body1: serde_json::Value = resp1.json().await.expect("turn1 json");
    assert!(
        status1.is_success(),
        "cursor turn1 failed {status1}: {body1}"
    );

    let tool_calls = body1["choices"][0]["message"]["tool_calls"]
        .as_array()
        .expect("tool_calls");
    assert!(!tool_calls.is_empty(), "expected tool call: {body1}");

    // Turn 2 — Cursor replays history WITHOUT reasoning_content (the bug we fix)
    let turn2 = serde_json::json!({
        "model": "gpt-4-turbo",
        "temperature": 0.2,
        "messages": [
            { "role": "developer", "content": "You are a coding agent." },
            { "role": "user", "content": "Use the read_file tool to read package.json" },
            {
                "role": "assistant",
                "content": null,
                "tool_calls": tool_calls
            },
            {
                "role": "tool",
                "tool_call_id": tool_calls[0]["id"],
                "content": "{ \"content\": \"{}\" }"
            }
        ],
        "tools": turn1["tools"],
        "max_completion_tokens": 2048
    });

    let resp2 = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&turn2)
        .send()
        .await
        .expect("turn2");
    let status2 = resp2.status();
    let body2: serde_json::Value = resp2.json().await.expect("turn2 json");
    let err = body2["error"]["message"].as_str().unwrap_or("");
    assert!(
        status2.is_success(),
        "cursor turn2 failed {status2}: {err} full={body2}"
    );
    println!("Cursor agent simulation: turn1+turn2 OK");
}

/// Reproduces the exact failure from production logs: 19 Cursor tools with dotted MCP names.
#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_cursor_mcp_tool_names_are_sanitized() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let port = 17423;
    let (_server, gateway_key) = start_gateway_with_key(port, &key).await;
    let client = Client::new();

    let mut tools: Vec<serde_json::Value> = Vec::new();
    for (i, name) in [
        "mcp.filesystem.read_file",
        "mcp.terminal.run",
        "server/search",
        "apply_patch",
        "2invalid",
    ]
    .iter()
    .enumerate()
    {
        tools.push(serde_json::json!({
            "type": if i == 3 { "custom" } else { "function" },
            "name": name,
            "description": format!("tool {i}"),
            "parameters": { "type": "object", "properties": {} }
        }));
    }

    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "temperature": 0,
            "messages": [],
            "tools": tools,
            "max_completion_tokens": 256
        }))
        .send()
        .await
        .expect("mcp tools request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("json");
    assert!(
        status.is_success(),
        "mcp tool names failed {status}: {body}"
    );
    println!("Cursor MCP tool names: OK");
}

/// Cursor sends stream:true and expects an SSE response. Verify the gateway returns
/// a valid text/event-stream that ends with [DONE].
#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_streaming_request_returns_sse() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let port = 17424;
    let (_server, gateway_key) = start_gateway_with_key(port, &key).await;
    let client = Client::new();

    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "stream": true,
            "messages": [{ "role": "user", "content": "Reply with exactly: STREAM_OK" }],
            "max_tokens": 50
        }))
        .send()
        .await
        .expect("stream request");

    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = resp.text().await.expect("body");

    assert!(status.is_success(), "stream failed {status}: {text}");
    assert!(
        content_type.contains("text/event-stream"),
        "expected SSE content-type, got: {content_type}"
    );
    assert!(text.contains("data: "), "no SSE data frames: {text}");
    assert!(text.contains("chat.completion.chunk"), "no chunk object: {text}");
    assert!(text.trim_end().ends_with("[DONE]"), "missing [DONE]: {text}");
    assert!(
        text.to_uppercase().contains("STREAM"),
        "content missing from stream: {text}"
    );
    println!("Streaming SSE response: OK");
}

/// Reproduces the exact Cursor Agent bug: `messages` empty, full history in `input`.
/// Without the Responses adapter the gateway would seed "Hi" and Kimi greets fresh.
#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_cursor_responses_format_preserves_context() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let port = 17427;
    let (_server, gateway_key) = start_gateway_with_key(port, &key).await;
    let client = Client::new();

    let cursor_agent_payload = serde_json::json!({
        "model": "gpt-4-turbo",
        "stream": false,
        "temperature": 0,
        "presence_penalty": 0,
        "store": false,
        "instructions": "You are a senior Rust engineer building a todo CLI.",
        "input": [
            { "role": "developer", "content": "Be direct. Follow instructions exactly." },
            { "role": "user", "content": "We are building a todo CLI. Phase 1: use clap + serde. Reply with exactly: PHASE1_OK" },
            { "role": "assistant", "content": "PHASE1_OK — scaffolding with clap and serde." },
            { "role": "user", "content": "Continue the build. Reply with exactly: PHASE2_CONTINUE" }
        ],
        "tools": [{
            "type": "function",
            "name": "read_file",
            "description": "Read a file",
            "parameters": { "type": "object", "properties": { "path": { "type": "string" } } }
        }],
        "tool_choice": "auto",
        "max_tokens": 200
    });

    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&cursor_agent_payload)
        .send()
        .await
        .expect("responses-format request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("json");
    assert!(status.is_success(), "responses format failed {status}: {body}");

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_uppercase();

    assert!(
        content.contains("PHASE2_CONTINUE"),
        "model lost context — got generic reply instead: {content}"
    );
    assert!(
        !content.contains("HELLO! HOW CAN I HELP"),
        "model greeted fresh (context was dropped): {content}"
    );
    println!("Cursor Responses format context: OK -> {content}");
}

/// Cursor may hit /v1/responses — same adapter must work there too.
#[tokio::test]
#[ignore = "requires MOONSHOT_API_KEY env var"]
async fn live_v1_responses_endpoint_accepts_cursor_payload() {
    let key = moonshot_key().expect("set MOONSHOT_API_KEY");
    let port = 17428;
    let (_server, gateway_key) = start_gateway_with_key(port, &key).await;
    let client = Client::new();

    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/responses"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "stream": false,
            "input": [{ "role": "user", "content": "Reply with exactly: RESPONSES_ENDPOINT_OK" }],
            "max_tokens": 50
        }))
        .send()
        .await
        .expect("responses endpoint");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("json");
    assert!(status.is_success(), "/v1/responses failed {status}: {body}");
    let content = body["choices"][0]["message"]["content"].as_str().unwrap_or("");
    assert!(content.to_uppercase().contains("RESPONSES_ENDPOINT_OK"));
    println!("/v1/responses endpoint: OK");
}
