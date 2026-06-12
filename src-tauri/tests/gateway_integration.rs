use kimi_cursor_gateway_lib::config::AppSettings;
use kimi_cursor_gateway_lib::gateway::server::{GatewayContext, GatewayServer};
use kimi_cursor_gateway_lib::gateway::MetricsStore;
use reqwest::Client;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

fn test_logs_dir() -> PathBuf {
    std::env::temp_dir().join("kimi-gateway-test-logs")
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

async fn start_test_gateway(settings: AppSettings) -> (GatewayServer, u16) {
    let port = free_port();
    let ctx = GatewayContext {
        settings: Arc::new(RwLock::new(settings)),
        metrics: MetricsStore::new(),
        usage: kimi_cursor_gateway_lib::gateway::TokenUsageStore::new(test_logs_dir().join("usage")),
        public_url: Arc::new(RwLock::new(Some(
            "https://test-tunnel.trycloudflare.com".to_string(),
        ))),
        logs_dir: test_logs_dir(),
        http_client: Client::new(),
        bound_port: port,
    };
    let server = GatewayServer::start(ctx, port)
        .await
        .expect("gateway should start");
    (server, port)
}

#[tokio::test]
async fn health_endpoint_returns_expected_shape() {
    let settings = AppSettings::default();
    let gateway_key = settings.gateway_key.clone();
    let (_server, port) = start_test_gateway(settings).await;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .expect("health request");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["ok"], true);
    assert_eq!(body["app"], "Kimi Cursor Gateway");
    assert_eq!(body["model"], "gpt-5-high-max");
    assert_eq!(body["realModel"], "kimi-k2.6");
    assert!(body["publicBaseUrl"]
        .as_str()
        .unwrap()
        .ends_with("/v1"));
    assert_eq!(
        body["localBaseUrl"],
        format!("http://127.0.0.1:{port}/v1")
    );
    assert!(!body["metrics"].is_null());
    let _ = gateway_key;
}

#[tokio::test]
async fn models_endpoint_lists_openai_compatible_models() {
    let (_server, port) = start_test_gateway(AppSettings::default()).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/v1/models"))
        .send()
        .await
        .expect("models request");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.expect("json");
    let ids: Vec<String> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&"gpt-5-high-max".to_string()));
    assert!(ids.contains(&"gpt-4-turbo".to_string()));
    assert!(ids.contains(&"gpt-4o".to_string()));
    assert!(ids.contains(&"kimi-k2.6".to_string()));
}

#[tokio::test]
async fn chat_completions_rejects_missing_gateway_key() {
    let (_server, port) = start_test_gateway(AppSettings::default()).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "Say hi." }]
        }))
        .send()
        .await
        .expect("chat request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn chat_completions_rejects_wrong_gateway_key() {
    let (_server, port) = start_test_gateway(AppSettings::default()).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", "Bearer wrong-key")
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "Say hi." }]
        }))
        .send()
        .await
        .expect("chat request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn chat_completions_requires_moonshot_key_when_gateway_key_valid() {
    let settings = AppSettings::default();
    let gateway_key = settings.gateway_key.clone();
    let (_server, port) = start_test_gateway(settings).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {gateway_key}"))
        .json(&serde_json::json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "Say hi." }]
        }))
        .send()
        .await
        .expect("chat request");

    assert_eq!(resp.status(), 500);
    let body: serde_json::Value = resp.json().await.expect("json");
    let msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("Moonshot API key"));
}

#[tokio::test]
async fn dashboard_endpoint_returns_html() {
    let (_server, port) = start_test_gateway(AppSettings::default()).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/dashboard"))
        .send()
        .await
        .expect("dashboard request");

    assert!(resp.status().is_success());
    let html = resp.text().await.expect("html");
    assert!(html.contains("Kimi Cursor Gateway"));
    assert!(html.contains("gpt-5-high-max"));
}
