use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

/// Kill orphaned `cloudflared.exe` processes from prior gateway runs (same binary or port).
pub fn kill_stale_cloudflared_processes(cloudflared_path: &Path, local_port: u16) -> u32 {
    #[cfg(windows)]
    {
        kill_stale_cloudflared_windows(cloudflared_path, local_port)
    }
    #[cfg(not(windows))]
    {
        let _ = (cloudflared_path, local_port);
        0
    }
}

#[cfg(windows)]
fn kill_stale_cloudflared_windows(cloudflared_path: &Path, local_port: u16) -> u32 {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let path_lower = cloudflared_path.to_string_lossy().to_lowercase();
    let path_escaped = path_lower.replace('\'', "''");
    let port_target = format!("127.0.0.1:{local_port}");

    let script = format!(
        "$killed = 0; \
         Get-CimInstance Win32_Process -Filter \"name='cloudflared.exe'\" -ErrorAction SilentlyContinue | \
         ForEach-Object {{ \
           $exe = ('' + $_.ExecutablePath).ToLower(); \
           $cmd = ('' + $_.CommandLine).ToLower(); \
           if ($exe -eq '{path_escaped}' -or $cmd -like '*{port_target}*') {{ \
             Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue; \
             $killed++ \
           }} \
         }}; \
         Write-Output $killed"
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let killed = stdout.trim().parse::<u32>().unwrap_or(0);
            if killed > 0 {
                info!(
                    "Killed {killed} stale cloudflared process(es) for port {local_port}"
                );
            }
            killed
        }
        Err(e) => {
            warn!("Could not run cloudflared cleanup: {e}");
            0
        }
    }
}

#[derive(Debug, Clone)]
pub enum TunnelEvent {
    UrlChanged(String),
    Crashed,
    Ready,
    Log(String),
}

pub struct TunnelManager {
    child: Option<Child>,
    public_url: Arc<RwLock<Option<String>>>,
    cloudflared_path: PathBuf,
    local_port: u16,
}

impl TunnelManager {
    pub fn new(cloudflared_path: PathBuf, local_port: u16) -> Self {
        Self {
            child: None,
            public_url: Arc::new(RwLock::new(None)),
            cloudflared_path,
            local_port,
        }
    }

    pub fn public_url_handle(&self) -> Arc<RwLock<Option<String>>> {
        self.public_url.clone()
    }

    pub async fn current_url(&self) -> Option<String> {
        self.public_url.read().await.clone()
    }

    pub async fn start(&mut self, event_tx: mpsc::UnboundedSender<TunnelEvent>) -> Result<(), String> {
        if !self.cloudflared_path.exists() {
            return Err(
                "cloudflared is not installed yet. Use the in-app downloader or place cloudflared.exe in app data."
                    .to_string(),
            );
        }

        self.stop().await;
        kill_stale_cloudflared_processes(&self.cloudflared_path, self.local_port);

        let mut child = Command::new(&self.cloudflared_path)
            .args([
                "tunnel",
                "--protocol",
                "http2",
                "--url",
                &format!("http://127.0.0.1:{}", self.local_port),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Could not start secure tunnel: {e}"))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let url_store = self.public_url.clone();
        let port = self.local_port;

        if let Some(out) = stdout {
            let tx = event_tx.clone();
            tokio::spawn(async move {
                read_stream_for_url(out, url_store, tx, false).await;
            });
        }

        if let Some(err) = stderr {
            let tx = event_tx.clone();
            tokio::spawn(async move {
                read_stream_for_url(err, Arc::new(RwLock::new(None)), tx, true).await;
            });
        }

        let tx_monitor = event_tx.clone();
        let child_for_monitor = child.id();
        tokio::spawn(async move {
            // monitor placeholder - actual wait handled below
            let _ = (tx_monitor, child_for_monitor, port);
        });

        self.child = Some(child);
        info!("Tunnel process started for port {}", self.local_port);
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        kill_stale_cloudflared_processes(&self.cloudflared_path, self.local_port);
        *self.public_url.write().await = None;
    }

}

async fn read_stream_for_url<R>(
    stream: R,
    url_store: Arc<RwLock<Option<String>>>,
    event_tx: mpsc::UnboundedSender<TunnelEvent>,
    is_stderr: bool,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let regex = Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap();
    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let _ = event_tx.send(TunnelEvent::Log(line.clone()));
        if let Some(url) = regex.find(&line).map(|m| m.as_str().to_string()) {
            let previous = url_store.read().await.clone();
            *url_store.write().await = Some(url.clone());
            if previous.as_deref() != Some(&url) {
                let _ = event_tx.send(TunnelEvent::UrlChanged(url.clone()));
            }
            let _ = event_tx.send(TunnelEvent::Ready);
        } else if is_stderr && line.to_lowercase().contains("error") {
            let _ = event_tx.send(TunnelEvent::Log(line));
        }
    }
    let _ = event_tx.send(TunnelEvent::Crashed);
}

pub async fn download_cloudflared(target: &Path) -> Result<(), String> {
    let url = "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe";
    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    if !response.status().is_success() {
        return Err("Could not download cloudflared from GitHub releases.".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Download read failed: {e}"))?;

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Could not create directory: {e}"))?;
    }
    std::fs::write(target, bytes).map_err(|e| format!("Could not save cloudflared: {e}"))?;
    info!("Downloaded cloudflared to {}", target.display());
    Ok(())
}

pub fn resolve_cloudflared_path(app_data_path: &Path, resource_dir: Option<PathBuf>) -> PathBuf {
    if let Some(res) = resource_dir {
        let sidecar = res.join("cloudflared-x86_64-pc-windows-msvc.exe");
        if sidecar.exists() {
            return sidecar;
        }
        let sidecar2 = res.join("binaries").join("cloudflared-x86_64-pc-windows-msvc.exe");
        if sidecar2.exists() {
            return sidecar2;
        }
    }
    app_data_path.join("cloudflared.exe")
}
