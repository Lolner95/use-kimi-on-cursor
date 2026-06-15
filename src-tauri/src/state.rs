use crate::config::ConfigStore;
use crate::notify::notify;
use crate::cursor_settings::{apply_cursor_settings, verify_cursor_alignment, CursorAlignmentStatus};
use crate::gateway::server::{GatewayContext, GatewayServer};
use crate::gateway::{MetricsStore, TokenUsageStore};
use crate::logging::append_log;
use crate::tunnel::manager::{
    download_cloudflared, kill_stale_cloudflared_processes, resolve_cloudflared_path,
};
use parking_lot::Mutex;
use reqwest::Client;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::RwLock;
use tracing::info;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub running: bool,
    pub local_server: bool,
    pub tunnel: bool,
    pub moonshot_reachable: bool,
    pub cursor_ready: bool,
    pub public_root_url: Option<String>,
    pub public_base_url: Option<String>,
    pub local_base_url: String,
    pub gateway_key: String,
    pub alias_model: String,
    pub real_model: String,
    pub last_error: Option<String>,
    pub cursor_alignment: Option<CursorAlignmentStatus>,
}

pub struct AppState {
    pub config: Mutex<ConfigStore>,
    pub gateway_server: Mutex<Option<GatewayServer>>,
    pub public_url: Arc<RwLock<Option<String>>>,
    pub metrics: MetricsStore,
    pub usage: TokenUsageStore,
    pub running: Arc<RwLock<bool>>,
    pub tunnel_shutdown: Arc<RwLock<Option<tokio::sync::watch::Sender<bool>>>>,
    pub app_handle: Mutex<Option<AppHandle>>,
    pub cloudflared_path: Mutex<PathBuf>,
    pub ui_logs: Mutex<Vec<String>>,
    /// Set to true when the app is launched at boot (--minimized flag).
    /// The first tunnel-ready sync will sleep 10 s to let Cursor finish booting
    /// before we write to its SQLite settings database.
    pub is_boot_start: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(config: ConfigStore) -> Self {
        let cloudflared = resolve_cloudflared_path(&config.paths.data_dir, None);
        let usage = TokenUsageStore::new(config.paths.usage_dir.clone());
        Self {
            config: Mutex::new(config),
            gateway_server: Mutex::new(None),
            public_url: Arc::new(RwLock::new(None)),
            metrics: MetricsStore::new(),
            usage,
            running: Arc::new(RwLock::new(false)),
            tunnel_shutdown: Arc::new(RwLock::new(None)),
            app_handle: Mutex::new(None),
            cloudflared_path: Mutex::new(cloudflared),
            ui_logs: Mutex::new(Vec::new()),
            is_boot_start: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn push_log(&self, line: String) {
        let mut logs = self.ui_logs.lock();
        logs.push(line);
        if logs.len() > 500 {
            let drain = logs.len() - 500;
            logs.drain(0..drain);
        }
    }

    pub fn get_logs(&self) -> Vec<String> {
        self.ui_logs.lock().clone()
    }

    pub fn clear_logs(&self) {
        self.ui_logs.lock().clear();
    }

    pub async fn status_snapshot(&self) -> GatewayStatus {
        let (local_port, gateway_key, alias_model, real_model, local_server) = {
            let config = self.config.lock();
            (
                config.settings.local_port,
                config.settings.gateway_key.clone(),
                config.settings.alias_model.clone(),
                config.settings.real_model.clone(),
                self.gateway_server.lock().is_some(),
            )
        };

        let public = self.public_url.read().await.clone();
        let running = *self.running.read().await;
        let metrics = self.metrics.snapshot();

        let public_base = public.as_ref().map(|u| format!("{u}/v1"));
        let local_base = format!("http://127.0.0.1:{local_port}/v1");

        let cursor_alignment = public_base.as_ref().map(|base| {
            verify_cursor_alignment(&gateway_key, base, &alias_model).unwrap_or_else(|_| {
                CursorAlignmentStatus {
                    installed: false,
                    db_path: String::new(),
                    key_matches: false,
                    use_openai_key: false,
                    base_url_matches: false,
                    composer_model_matches: false,
                    aligned: false,
                    stored_key_prefix: None,
                    expected_key_prefix: gateway_key.chars().take(12).collect(),
                    stored_base_url: None,
                    expected_base_url: base.clone(),
                    stored_composer_model: None,
                    expected_model: alias_model.clone(),
                    issues: vec!["Cursor database not found.".to_string()],
                }
            })
        });

        GatewayStatus {
            running,
            local_server,
            tunnel: public.is_some(),
            moonshot_reachable: metrics.last_error.as_deref()
                != Some("Moonshot rejected your API key. Open Settings and paste a valid Kimi Open Platform key."),
            cursor_ready: running && public.is_some(),
            public_root_url: public.clone(),
            public_base_url: public_base,
            local_base_url: local_base,
            gateway_key,
            alias_model,
            real_model,
            last_error: metrics.last_error.clone(),
            cursor_alignment,
        }
    }

    pub async fn sync_cursor_settings(&self, reason: &str) -> Result<CursorAlignmentStatus, String> {
        let status = self.status_snapshot().await;
        let base = status
            .public_base_url
            .ok_or_else(|| "Tunnel URL is not ready yet.".to_string())?;
        let (gateway_key, alias_model) = {
            let config = self.config.lock();
            (
                config.settings.gateway_key.clone(),
                config.settings.alias_model.clone(),
            )
        };

        let result = apply_cursor_settings(&gateway_key, &base, &alias_model)
            .map_err(|e| e.to_string())?;

        self.push_log(format!(
            "Synced Cursor settings ({reason}): base={}, aligned={}",
            result.base_url, result.alignment.aligned
        ));
        Ok(result.alignment)
    }

    pub async fn start_gateway(&self, app: &AppHandle) -> Result<GatewayStatus, String> {
        if *self.running.read().await {
            return Ok(self.status_snapshot().await);
        }

        let (port, logs_dir, settings_snapshot, cloudflared_path) = {
            let config = self.config.lock();
            (
                config.settings.local_port,
                config.paths.logs_dir.clone(),
                config.settings.clone(),
                self.cloudflared_path.lock().clone(),
            )
        };

        if !cloudflared_path.exists() {
            download_cloudflared(&cloudflared_path).await?;
            self.push_log("Downloaded cloudflared.".to_string());
        }

        let killed = kill_stale_cloudflared_processes(&cloudflared_path, port);
        if killed > 0 {
            self.push_log(format!("Stopped {killed} stale cloudflared tunnel(s)."));
        }

        let ctx = GatewayContext {
            settings: Arc::new(RwLock::new(settings_snapshot)),
            metrics: self.metrics.clone(),
            usage: self.usage.clone(),
            public_url: self.public_url.clone(),
            logs_dir: logs_dir.clone(),
            http_client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .pool_max_idle_per_host(10)
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| Client::new()),
            bound_port: port,
        };

        let server = GatewayServer::start(ctx, port).await?;
        *self.gateway_server.lock() = Some(server);
        *self.running.write().await = true;

        self.start_tunnel_supervisor(port, cloudflared_path, logs_dir, app)
            .await?;

        let status = self.status_snapshot().await;
        let _ = app.emit("gateway-status", &status);
        let _ = app.emit("gateway-ready", &status);
        notify(
            app,
            "Kimi Cursor Gateway",
            "Local gateway started. Waiting for secure tunnel...",
        );
        info!("Gateway started");
        Ok(status)
    }

    async fn start_tunnel_supervisor(
        &self,
        port: u16,
        cloudflared_path: PathBuf,
        logs_dir: PathBuf,
        app: &AppHandle,
    ) -> Result<(), String> {
        if let Some(tx) = self.tunnel_shutdown.read().await.as_ref() {
            let _ = tx.send(true);
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        *self.tunnel_shutdown.write().await = Some(shutdown_tx);

        let public_url = self.public_url.clone();
        let app_handle = app.clone();

        tokio::spawn(async move {
            supervise_tunnel(
                cloudflared_path,
                port,
                public_url,
                logs_dir,
                app_handle,
                shutdown_rx,
            )
            .await;
        });

        Ok(())
    }

    pub async fn stop_gateway(&self, app: &AppHandle) -> Result<GatewayStatus, String> {
        if let Some(tx) = self.tunnel_shutdown.read().await.as_ref() {
            let _ = tx.send(true);
        }

        let (cloudflared_path, port) = {
            let config = self.config.lock();
            (
                self.cloudflared_path.lock().clone(),
                config.settings.local_port,
            )
        };
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let killed = kill_stale_cloudflared_processes(&cloudflared_path, port);
        if killed > 0 {
            self.push_log(format!("Stopped {killed} cloudflared tunnel(s)."));
        }

        let server = self.gateway_server.lock().take();
        if let Some(server) = server {
            server.stop().await;
        }

        *self.public_url.write().await = None;
        *self.running.write().await = false;

        let status = self.status_snapshot().await;
        let _ = app.emit("gateway-status", &status);
        info!("Gateway stopped");
        Ok(status)
    }

    pub async fn restart_gateway(&self, app: &AppHandle) -> Result<GatewayStatus, String> {
        let _ = self.stop_gateway(app).await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        self.start_gateway(app).await
    }
}

/// Seconds to wait before the next restart attempt. Grows exponentially,
/// capped at MAX_BACKOFF_SECS, and resets when a tunnel stays healthy for
/// at least BACKOFF_RESET_SECS.
const INITIAL_BACKOFF_SECS: u64 = 3;
const MAX_BACKOFF_SECS: u64 = 30;
const BACKOFF_RESET_SECS: u64 = 120;

/// How long to wait for cloudflared to report a URL before giving up and
/// restarting (handles silent hangs where the process is alive but stuck).
const URL_TIMEOUT_SECS: u64 = 45;

/// How often to HTTP-probe the tunnel URL to catch silent disconnections.
const HEALTH_CHECK_INTERVAL_SECS: u64 = 60;

/// Consecutive health-check failures before we kill and restart.
const HEALTH_CHECK_MAX_FAILURES: u32 = 2;

async fn supervise_tunnel(
    cloudflared_path: PathBuf,
    local_port: u16,
    public_url: Arc<RwLock<Option<String>>>,
    logs_dir: PathBuf,
    app: AppHandle,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    use regex::Regex;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;
    use tokio::sync::mpsc;

    let url_regex = Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap();

    let mut backoff_secs = INITIAL_BACKOFF_SECS;
    let mut last_healthy_at = std::time::Instant::now();

    loop {
        if *shutdown.borrow() {
            break;
        }

        kill_stale_cloudflared_processes(&cloudflared_path, local_port);

        if !cloudflared_path.exists() {
            append_log(&logs_dir, "tunnel.log", "cloudflared missing, retrying in 5s");
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                _ = shutdown.changed() => { break; }
            }
            continue;
        }

        // Try QUIC first (more resilient); cloudflared falls back internally if unavailable.
        let mut child = match Command::new(&cloudflared_path)
            .args([
                "tunnel",
                "--no-autoupdate",
                "--protocol",
                "quic",
                "--url",
                &format!("http://127.0.0.1:{local_port}"),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                append_log(&logs_dir, "tunnel.log", &format!("spawn error: {e}, retrying in {backoff_secs}s"));
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)) => {}
                    _ = shutdown.changed() => { break; }
                }
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                continue;
            }
        };

        append_log(&logs_dir, "tunnel.log", "tunnel process started (protocol=quic)");

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Channel that delivers the first tunnel URL found in stdout or stderr.
        let (url_tx, mut url_rx) = mpsc::unbounded_channel::<String>();

        // Spawn stdout reader.
        if let Some(out) = stdout {
            let regex = url_regex.clone();
            let tx = url_tx.clone();
            let logs = logs_dir.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(out);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(url) = regex.find(&line).map(|m| m.as_str().to_string()) {
                        let _ = tx.send(url);
                    }
                }
                append_log(&logs, "tunnel.log", "stdout stream closed");
            });
        }

        // Spawn stderr reader (cloudflared writes the URL to stderr).
        if let Some(err) = stderr {
            let regex = url_regex.clone();
            let tx = url_tx.clone();
            let logs = logs_dir.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(err);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    append_log(&logs, "tunnel.log", &line);
                    if let Some(url) = regex.find(&line).map(|m| m.as_str().to_string()) {
                        let _ = tx.send(url);
                    }
                }
                append_log(&logs, "tunnel.log", "stderr stream closed");
            });
        }

        drop(url_tx); // all senders are in the spawned tasks above

        // Wait for the tunnel URL (with a timeout to catch silent hangs).
        let new_url = tokio::select! {
            url = url_rx.recv() => url,
            _ = tokio::time::sleep(std::time::Duration::from_secs(URL_TIMEOUT_SECS)) => {
                append_log(&logs_dir, "tunnel.log",
                    &format!("No URL after {URL_TIMEOUT_SECS}s - killing hung cloudflared and restarting"));
                let _ = child.kill().await;
                None
            }
            _ = shutdown.changed() => {
                let _ = child.kill().await;
                break;
            }
        };

        if let Some(url) = new_url {
            let previous = public_url.read().await.clone();
            *public_url.write().await = Some(url.clone());
            last_healthy_at = std::time::Instant::now();
            backoff_secs = INITIAL_BACKOFF_SECS; // reset on successful connection

            emit_url_and_sync(&app, &url, previous.as_deref()).await;

            // Background health prober - periodically checks the tunnel is alive.
            let health_url = url.clone();
            let health_pub = public_url.clone();
            let health_logs = logs_dir.clone();
            let child_id = child.id();
            tokio::spawn(async move {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()
                    .unwrap_or_default();
                let mut failures = 0u32;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS)).await;

                    // Stop probing if the URL has already changed (another restart happened).
                    if health_pub.read().await.as_deref() != Some(&health_url) {
                        break;
                    }

                    let probe = client
                        .head(&health_url)
                        .send()
                        .await;

                    match probe {
                        Ok(r) if r.status().as_u16() < 530 => {
                            // Any response < 530 means the edge reached our tunnel.
                            // (530 is Cloudflare's "tunnel unreachable" sentinel.)
                            failures = 0;
                        }
                        Ok(r) => {
                            failures += 1;
                            append_log(
                                &health_logs,
                                "tunnel.log",
                                &format!("health probe got {} (failure {failures}/{HEALTH_CHECK_MAX_FAILURES})",
                                    r.status()),
                            );
                        }
                        Err(e) => {
                            failures += 1;
                            append_log(
                                &health_logs,
                                "tunnel.log",
                                &format!("health probe error: {e} (failure {failures}/{HEALTH_CHECK_MAX_FAILURES})"),
                            );
                        }
                    }

                    if failures >= HEALTH_CHECK_MAX_FAILURES {
                        append_log(
                            &health_logs,
                            "tunnel.log",
                            "health probe failed - marking tunnel as dead",
                        );
                        *health_pub.write().await = None;
                        // Kill cloudflared by pid so the supervise loop's wait() unblocks.
                        if let Some(pid) = child_id {
                            #[cfg(windows)]
                            {
                                use std::os::windows::process::CommandExt;
                                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                                let _ = std::process::Command::new("taskkill")
                                    .args(["/PID", &pid.to_string(), "/F"])
                                    .creation_flags(CREATE_NO_WINDOW)
                                    .output();
                            }
                            #[cfg(not(windows))]
                            {
                                let _ = std::process::Command::new("kill")
                                    .args(["-9", &pid.to_string()])
                                    .output();
                            }
                        }
                        break;
                    }
                }
            });

            // Wait for process exit OR shutdown signal.
            let exit_status = tokio::select! {
                s = child.wait() => s,
                _ = shutdown.changed() => {
                    let _ = child.kill().await;
                    kill_stale_cloudflared_processes(&cloudflared_path, local_port);
                    break;
                }
            };

            *public_url.write().await = None;
            let msg = match exit_status {
                Ok(s) => format!("Tunnel exited ({s}), restarting..."),
                Err(e) => format!("Tunnel error ({e}), restarting..."),
            };
            append_log(&logs_dir, "tunnel.log", &msg);
            let _ = app.emit("tunnel-crashed", &msg);
            notify(
                &app,
                "Tunnel reconnecting",
                "The secure tunnel disconnected. Reconnecting automatically...",
            );
        }

        // Reset backoff if the tunnel was healthy for a sustained period.
        if last_healthy_at.elapsed().as_secs() >= BACKOFF_RESET_SECS {
            backoff_secs = INITIAL_BACKOFF_SECS;
        }

        let wait = backoff_secs;
        append_log(&logs_dir, "tunnel.log", &format!("Waiting {wait}s before restart..."));
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(wait)) => {}
            _ = shutdown.changed() => { break; }
        }
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
    }

    kill_stale_cloudflared_processes(&cloudflared_path, local_port);
}

/// Emit the new tunnel URL event, sync Cursor settings, and notify the user.
async fn emit_url_and_sync(app: &AppHandle, url: &str, previous: Option<&str>) {
    let is_new = previous != Some(url);
    let _ = app.emit("tunnel-url-changed", url);

    if is_new {
        if let Some(managed) = app.try_state::<Arc<AppState>>() {
            let reason = if previous.is_some() { "tunnel URL changed" } else { "tunnel ready" };
            let app_state = managed.inner().clone();
            let app_for_sync = app.clone();
            let url_owned = url.to_string();

            // On the very first tunnel URL after a boot-time start, give Cursor
            // 10 s to finish its own startup before we write to its SQLite DB.
            // swap() returns the old value and atomically clears the flag so
            // subsequent URL changes (re-connects) run immediately.
            let is_boot = app_state.is_boot_start.swap(false, Ordering::Relaxed);

            tokio::spawn(async move {
                if is_boot {
                    info!("Boot mode: waiting 10 s before syncing Cursor settings so Cursor can finish starting up…");
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
                match app_state.sync_cursor_settings(reason).await {
                    Ok(alignment) if alignment.aligned => {
                        notify(
                            &app_for_sync,
                            "Cursor settings synced",
                            &format!("Tunnel ready: {url_owned}/v1 - restart Cursor if it was already open."),
                        );
                    }
                    Ok(alignment) => {
                        notify(&app_for_sync, "Cursor needs attention", &alignment.issues.join(" "));
                    }
                    Err(e) => {
                        notify(&app_for_sync, "Cursor sync failed", &e);
                    }
                }
            });
        } else {
            notify(app, "Tunnel URL updated", &format!("Cursor Base URL: {url}/v1"));
        }
    } else {
        notify(app, "Gateway ready", &format!("Cursor Base URL: {url}/v1"));
    }
}
