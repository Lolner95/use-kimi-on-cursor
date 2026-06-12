//! Live tunnel test — run with:
//!   cargo test --test live_tunnel -- --ignored --nocapture

use kimi_cursor_gateway_lib::config::AppSettings;
use kimi_cursor_gateway_lib::gateway::server::{GatewayContext, GatewayServer};
use kimi_cursor_gateway_lib::gateway::{MetricsStore, TokenUsageStore};
use kimi_cursor_gateway_lib::tunnel::manager::download_cloudflared;
use regex::Regex;
use reqwest::Client;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::RwLock;

#[tokio::test]
#[ignore = "requires network and cloudflared download"]
async fn live_quick_tunnel_exposes_gateway_health() {
    let port = 17430u16;
    let data_dir = std::env::temp_dir().join("kimi-tunnel-e2e");
    let cloudflared = data_dir.join("cloudflared.exe");
    if !cloudflared.exists() {
        download_cloudflared(&cloudflared)
            .await
            .expect("download cloudflared");
    }

    let mut settings = AppSettings::default();
    let moonshot = std::env::var("MOONSHOT_API_KEY")
        .ok()
        .filter(|k| k.starts_with("sk-"));
    if let Some(ref k) = moonshot {
        settings.set_moonshot_key(k).expect("encrypt key");
    }
    let gateway_key = settings.gateway_key.clone();
    let ctx = GatewayContext {
        settings: Arc::new(RwLock::new(settings)),
        metrics: MetricsStore::new(),
        usage: TokenUsageStore::new(data_dir.join("usage")),
        public_url: Arc::new(RwLock::new(None)),
        logs_dir: data_dir.join("logs"),
        http_client: Client::new(),
        bound_port: port,
    };
    let _gateway = GatewayServer::start(ctx, port)
        .await
        .expect("gateway start");
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let mut child = Command::new(&cloudflared)
        .args([
            "tunnel",
            "--protocol",
            "http2",
            "--url",
            &format!("http://127.0.0.1:{port}"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn cloudflared");

    let url_regex = Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap();
    let mut tunnel_url: Option<String> = None;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Some(out) = stdout {
        let regex = url_regex.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(out);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(url) = regex.find(&line).map(|m| m.as_str().to_string()) {
                    println!("tunnel stdout url: {url}");
                }
            }
        });
    }

    if let Some(err) = stderr {
        let regex = url_regex.clone();
        let reader = BufReader::new(err);
        let mut lines = reader.lines();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(45);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Some(line)) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                lines.next_line(),
            )
            .await
            .unwrap_or(Ok(None))
            {
                println!("tunnel: {line}");
                if let Some(url) = regex.find(&line).map(|m| m.as_str().to_string()) {
                    tunnel_url = Some(url);
                    break;
                }
            }
        }
    }

    let public = tunnel_url.expect("tunnel URL should appear in cloudflared logs");
    println!("Public tunnel: {public}");

    let client = Client::new();
    let health_url = format!("{public}/health");
    let mut ok = false;
    for _ in 0..10 {
        if let Ok(resp) = client.get(&health_url).send().await {
            if resp.status().is_success() {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                assert_eq!(body["ok"], true);
                ok = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    assert!(ok, "public /health should respond through tunnel");

    // Full Cursor simulation through the PUBLIC tunnel URL (only if key provided).
    if let Some(_key) = moonshot {
        let base = format!("{public}/v1/chat/completions");

        // 1) Cursor model-validation probe: empty messages + dotted MCP tool names.
        let probe = client
            .post(&base)
            .header("Authorization", format!("Bearer {gateway_key}"))
            .json(&serde_json::json!({
                "model": "gpt-4-turbo",
                "temperature": 0,
                "messages": [],
                "tools": [
                    { "type": "function", "name": "mcp.fs.read_file",
                      "parameters": { "type": "object", "properties": {} } },
                    { "type": "custom", "name": "apply_patch", "description": "patch" }
                ]
            }))
            .send()
            .await
            .expect("probe through tunnel");
        let probe_status = probe.status();
        let probe_body = probe.text().await.unwrap_or_default();
        assert!(
            probe_status.is_success(),
            "public probe failed {probe_status}: {probe_body}"
        );
        println!("Public probe (empty msgs + MCP tools): OK");

        // 2) Streaming chat exactly like Cursor agent.
        let stream = client
            .post(&base)
            .header("Authorization", format!("Bearer {gateway_key}"))
            .json(&serde_json::json!({
                "model": "gpt-4-turbo",
                "stream": true,
                "messages": [{ "role": "user", "content": "Reply with exactly: TUNNEL_OK" }],
                "max_tokens": 50
            }))
            .send()
            .await
            .expect("stream through tunnel");
        let stream_status = stream.status();
        let ct = stream
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let stream_body = stream.text().await.unwrap_or_default();
        assert!(
            stream_status.is_success(),
            "public stream failed {stream_status}: {stream_body}"
        );
        assert!(ct.contains("text/event-stream"), "expected SSE, got {ct}");
        assert!(
            stream_body.contains("[DONE]") && stream_body.to_uppercase().contains("TUNNEL"),
            "stream body unexpected: {stream_body}"
        );
        println!("Public streaming chat through tunnel: OK");
    }

    let _ = child.kill().await;
}
