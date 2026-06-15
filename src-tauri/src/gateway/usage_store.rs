use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageEvent {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub date: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub request_id: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DailyTokenUsage {
    pub date: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub request_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatsSnapshot {
    pub today: DailyTokenUsage,
    pub last_7_days: Vec<DailyTokenUsage>,
    pub last_30_days: Vec<DailyTokenUsage>,
    pub lifetime: DailyTokenUsage,
    pub recent_events: Vec<TokenUsageEvent>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct UsageIndex {
    #[serde(default)]
    daily: BTreeMap<String, DailyTokenUsage>,
    #[serde(default)]
    lifetime_prompt_tokens: u64,
    #[serde(default)]
    lifetime_completion_tokens: u64,
    #[serde(default)]
    lifetime_total_tokens: u64,
    #[serde(default)]
    lifetime_request_count: u64,
}

#[derive(Clone)]
pub struct TokenUsageStore {
    inner: Arc<RwLock<UsageIndex>>,
    usage_dir: PathBuf,
    events_file: PathBuf,
}

impl TokenUsageStore {
    pub fn new(usage_dir: PathBuf) -> Self {
        if let Err(e) = fs::create_dir_all(&usage_dir) {
            warn!("Could not create usage dir {}: {e}", usage_dir.display());
        }
        let events_file = usage_dir.join("events.jsonl");
        let mut index = UsageIndex::default();
        if let Ok(raw) = fs::read_to_string(usage_dir.join("daily_index.json")) {
            if let Ok(parsed) = serde_json::from_str::<UsageIndex>(&raw) {
                index = parsed;
            }
        }
        Self {
            inner: Arc::new(RwLock::new(index)),
            usage_dir,
            events_file,
        }
    }

    pub fn record_from_value(
        &self,
        usage: &Value,
        request_id: &str,
        model: &str,
        latency_ms: u64,
    ) {
        let (prompt, completion, total) = parse_usage_triplet(usage);
        if total == 0 && prompt == 0 && completion == 0 {
            return;
        }
        self.record_counts(request_id, model, prompt, completion, total, latency_ms);
    }

    pub fn try_record_from_sse(&self, chunk: &[u8], request_id: &str, model: &str) {
        let Ok(text) = std::str::from_utf8(chunk) else {
            return;
        };
        for line in text.lines() {
            let payload = line.strip_prefix("data: ").unwrap_or(line).trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            let Ok(json) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            if let Some(usage) = json.get("usage") {
                self.record_from_value(usage, request_id, model, 0);
            }
        }
    }

    fn record_usage_if_present(
        &self,
        usage: &Value,
        request_id: &str,
        model: &str,
        latency_ms: u64,
    ) -> bool {
        let (prompt, completion, total) = parse_usage_triplet(usage);
        if prompt == 0 && completion == 0 && total == 0 {
            return false;
        }
        self.record_counts(request_id, model, prompt, completion, total, latency_ms);
        true
    }

    fn record_counts(
        &self,
        request_id: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        latency_ms: u64,
    ) {
        let now = Utc::now();
        let date = now.format("%Y-%m-%d").to_string();
        let event = TokenUsageEvent {
            id: Uuid::new_v4().to_string(),
            timestamp: now,
            date: date.clone(),
            model: model.to_string(),
            prompt_tokens,
            completion_tokens,
            total_tokens,
            request_id: request_id.to_string(),
            latency_ms,
        };

        {
            let mut index = self.inner.write();
            let day = index.daily.entry(date.clone()).or_insert_with(|| DailyTokenUsage {
                date: date.clone(),
                ..Default::default()
            });
            day.prompt_tokens += prompt_tokens;
            day.completion_tokens += completion_tokens;
            day.total_tokens += total_tokens;
            day.request_count += 1;

            index.lifetime_prompt_tokens += prompt_tokens;
            index.lifetime_completion_tokens += completion_tokens;
            index.lifetime_total_tokens += total_tokens;
            index.lifetime_request_count += 1;
        }

        self.append_event(&event);
        self.persist_index();
    }

    fn append_event(&self, event: &TokenUsageEvent) {
        if let Ok(line) = serde_json::to_string(event) {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.events_file)
            {
                let _ = writeln!(file, "{line}");
            }
        }
    }

    fn persist_index(&self) {
        let index = self.inner.read().clone();
        let path = self.usage_dir.join("daily_index.json");
        if let Ok(json) = serde_json::to_string_pretty(&index) {
            if let Err(e) = fs::write(&path, json) {
                warn!("Could not persist usage index: {e}");
            }
        }
    }

    pub fn snapshot(&self) -> UsageStatsSnapshot {
        let index = self.inner.read();
        let today_key = Utc::now().format("%Y-%m-%d").to_string();
        let today = index
            .daily
            .get(&today_key)
            .cloned()
            .unwrap_or_else(|| DailyTokenUsage {
                date: today_key,
                ..Default::default()
            });

        let last_7_days = rolling_days(&index.daily, 7);
        let last_30_days = rolling_days(&index.daily, 30);

        let lifetime = DailyTokenUsage {
            date: "lifetime".to_string(),
            prompt_tokens: index.lifetime_prompt_tokens,
            completion_tokens: index.lifetime_completion_tokens,
            total_tokens: index.lifetime_total_tokens,
            request_count: index.lifetime_request_count,
        };

        let recent_events = self.read_recent_events(40);

        UsageStatsSnapshot {
            today,
            last_7_days,
            last_30_days,
            lifetime,
            recent_events,
        }
    }

    fn read_recent_events(&self, limit: usize) -> Vec<TokenUsageEvent> {
        if !self.events_file.exists() {
            return Vec::new();
        }
        let Ok(raw) = fs::read_to_string(&self.events_file) else {
            return Vec::new();
        };
        let mut events: Vec<TokenUsageEvent> = raw
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        if events.len() > limit {
            events = events.split_off(events.len() - limit);
        }
        events
    }

    pub fn events_for_date(&self, date: &str) -> Vec<TokenUsageEvent> {
        if !self.events_file.exists() {
            return Vec::new();
        }
        let Ok(raw) = fs::read_to_string(&self.events_file) else {
            return Vec::new();
        };
        raw.lines()
            .filter_map(|line| serde_json::from_str::<TokenUsageEvent>(line).ok())
            .filter(|e| e.date == date)
            .collect()
    }

    pub fn usage_dir(&self) -> &Path {
        &self.usage_dir
    }
}

/// Buffers SSE bytes across network chunk boundaries so the final `usage`
/// payload is parsed even when TCP framing splits it mid-line. The previous
/// per-chunk parsing silently lost usage on virtually every streamed request,
/// which is why the token consumption report stayed empty.
pub struct SseUsageScanner {
    store: TokenUsageStore,
    request_id: String,
    model: String,
    started: std::time::Instant,
    buffer: String,
    recorded: bool,
}

const MAX_SSE_BUFFER_BYTES: usize = 4 * 1024 * 1024;

impl SseUsageScanner {
    pub fn new(
        store: TokenUsageStore,
        request_id: String,
        model: String,
        started: std::time::Instant,
    ) -> Self {
        Self {
            store,
            request_id,
            model,
            started,
            buffer: String::new(),
            recorded: false,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        if self.recorded {
            return;
        }
        self.buffer.push_str(&String::from_utf8_lossy(chunk));

        // Process complete lines; keep any trailing partial line buffered.
        while let Some(pos) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=pos).collect();
            self.process_line(line.trim_end());
            if self.recorded {
                self.buffer.clear();
                self.buffer.shrink_to_fit();
                return;
            }
        }

        // Safety valve: a pathological line without newline can't grow forever.
        if self.buffer.len() > MAX_SSE_BUFFER_BYTES {
            self.buffer.clear();
        }
    }

    fn process_line(&mut self, line: &str) {
        let payload = line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
            .unwrap_or(line)
            .trim();
        if payload.is_empty() || payload == "[DONE]" {
            return;
        }
        let Ok(json) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        if let Some(usage) = json.get("usage") {
            let latency = self.started.elapsed().as_millis() as u64;
            if self
                .store
                .record_usage_if_present(usage, &self.request_id, &self.model, latency)
            {
                self.recorded = true;
            }
        }
    }
}

fn parse_usage_triplet(usage: &Value) -> (u64, u64, u64) {
    let prompt = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(prompt + completion);
    (prompt, completion, total)
}

fn rolling_days(daily: &BTreeMap<String, DailyTokenUsage>, days: u32) -> Vec<DailyTokenUsage> {
    let today = Utc::now().date_naive();
    (0..days)
        .map(|offset| today - chrono::Duration::days(i64::from(offset)))
        .map(|date| date.format("%Y-%m-%d").to_string())
        .map(|key| {
            daily.get(&key).cloned().unwrap_or_else(|| DailyTokenUsage {
                date: key,
                ..Default::default()
            })
        })
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_store() -> TokenUsageStore {
        let dir = std::env::temp_dir().join(format!("kimi-usage-test-{}", Uuid::new_v4()));
        TokenUsageStore::new(dir)
    }

    #[test]
    fn records_usage_from_json_value() {
        let store = temp_store();
        store.record_from_value(
            &json!({
                "prompt_tokens": 120,
                "completion_tokens": 45,
                "total_tokens": 165
            }),
            "req-1",
            "gpt-5-high-max",
            900,
        );
        let snap = store.snapshot();
        assert_eq!(snap.today.prompt_tokens, 120);
        assert_eq!(snap.today.completion_tokens, 45);
        assert_eq!(snap.today.total_tokens, 165);
        assert_eq!(snap.today.request_count, 1);
        assert_eq!(snap.lifetime.total_tokens, 165);
    }

    #[test]
    fn extracts_usage_from_sse_chunk() {
        let store = temp_store();
        let chunk = b"data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n";
        store.try_record_from_sse(chunk, "req-2", "kimi-k2.7");
        let snap = store.snapshot();
        assert_eq!(snap.today.total_tokens, 15);
    }

    #[test]
    fn scanner_records_usage_split_across_chunks() {
        let store = temp_store();
        let mut scanner = SseUsageScanner::new(
            store.clone(),
            "req-3".into(),
            "kimi-k2.7".into(),
            std::time::Instant::now(),
        );
        // The usage event is split mid-JSON across two network chunks.
        scanner.feed(b"data: {\"choices\":[]}\n\ndata: {\"usage\":{\"prompt_tok");
        scanner.feed(b"ens\":100,\"completion_tokens\":20,\"total_tokens\":120}}\n\ndata: [DONE]\n\n");
        let snap = store.snapshot();
        assert_eq!(snap.today.total_tokens, 120);
        assert_eq!(snap.today.request_count, 1);
    }

    #[test]
    fn scanner_records_only_once_and_skips_null_usage() {
        let store = temp_store();
        let mut scanner = SseUsageScanner::new(
            store.clone(),
            "req-4".into(),
            "kimi-k2.7".into(),
            std::time::Instant::now(),
        );
        scanner.feed(b"data: {\"usage\":null}\n\n");
        scanner.feed(b"data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n");
        scanner.feed(b"data: {\"usage\":{\"prompt_tokens\":99,\"completion_tokens\":99,\"total_tokens\":198}}\n\n");
        let snap = store.snapshot();
        assert_eq!(snap.today.total_tokens, 15, "only the first usage event counts");
        assert_eq!(snap.today.request_count, 1);
    }

    #[test]
    fn rolling_days_includes_empty_slots() {
        let mut daily = BTreeMap::new();
        let today = Utc::now().format("%Y-%m-%d").to_string();
        daily.insert(
            today.clone(),
            DailyTokenUsage {
                date: today,
                total_tokens: 50,
                request_count: 1,
                ..Default::default()
            },
        );
        let week = rolling_days(&daily, 7);
        assert_eq!(week.len(), 7);
        assert!(week.iter().any(|d| d.total_tokens == 50));
    }
}
