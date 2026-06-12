use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayMetrics {
    pub started_at: Option<DateTime<Utc>>,
    pub requests: u64,
    pub upstream_ok: u64,
    pub upstream_errors: u64,
    pub last_request_at: Option<DateTime<Utc>>,
    pub last_status: Option<u16>,
    pub last_latency_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl Default for GatewayMetrics {
    fn default() -> Self {
        Self {
            started_at: None,
            requests: 0,
            upstream_ok: 0,
            upstream_errors: 0,
            last_request_at: None,
            last_status: None,
            last_latency_ms: None,
            last_error: None,
        }
    }
}

#[derive(Clone)]
pub struct MetricsStore {
    inner: Arc<RwLock<GatewayMetrics>>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(GatewayMetrics::default())),
        }
    }

    pub fn mark_started(&self) {
        let mut m = self.inner.write();
        m.started_at = Some(Utc::now());
    }

    pub fn record_request(&self) {
        self.inner.write().requests += 1;
    }

    pub fn record_success(&self, status: u16, latency_ms: u64) {
        let mut m = self.inner.write();
        m.upstream_ok += 1;
        m.last_request_at = Some(Utc::now());
        m.last_status = Some(status);
        m.last_latency_ms = Some(latency_ms);
        m.last_error = None;
    }

    pub fn record_error(&self, status: Option<u16>, latency_ms: u64, error: String) {
        let mut m = self.inner.write();
        m.upstream_errors += 1;
        m.last_request_at = Some(Utc::now());
        m.last_status = status;
        m.last_latency_ms = Some(latency_ms);
        m.last_error = Some(error);
    }

    pub fn snapshot(&self) -> GatewayMetrics {
        self.inner.read().clone()
    }
}

impl Default for MetricsStore {
    fn default() -> Self {
        Self::new()
    }
}
