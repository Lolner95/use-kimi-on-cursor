use regex::Regex;
use std::env::consts::{ARCH, OS};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

/// Kill orphaned cloudflared processes from prior gateway runs (same binary or port).
pub fn kill_stale_cloudflared_processes(cloudflared_path: &Path, local_port: u16) -> u32 {
    #[cfg(windows)]
    {
        kill_stale_cloudflared_windows(cloudflared_path, local_port)
    }
    #[cfg(not(windows))]
    {
        kill_stale_cloudflared_unix(cloudflared_path, local_port)
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

#[cfg(not(windows))]
fn kill_stale_cloudflared_unix(cloudflared_path: &Path, local_port: u16) -> u32 {
    let port_pattern = format!("127.0.0.1:{local_port}");
    let binary_pattern = cloudflared_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("cloudflared");
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ps -axo pid=,command= | awk 'index($0, \"{binary_pattern}\") && index($0, \"{port_pattern}\") {{print $1}}'"
        ))
        .output();

    let Ok(out) = output else {
        return 0;
    };

    let pids = String::from_utf8_lossy(&out.stdout);
    let mut killed = 0u32;
    for pid in pids.lines().map(str::trim).filter(|p| !p.is_empty()) {
        if std::process::Command::new("kill")
            .args(["-9", pid])
            .status()
            .is_ok_and(|s| s.success())
        {
            killed += 1;
        }
    }
    if killed > 0 {
        info!("Killed {killed} stale cloudflared process(es) for port {local_port}");
    }
    killed
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
                "cloudflared is not installed yet. Use the in-app downloader or place the cloudflared binary in app data."
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
    let url = cloudflared_download_url()?;
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

    if target
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        || !matches!(OS, "macos")
    {
        std::fs::write(target, bytes).map_err(|e| format!("Could not save cloudflared: {e}"))?;
    } else {
        // macOS release artifacts are tarballs. Extract the `cloudflared` binary.
        let parent = target
            .parent()
            .ok_or_else(|| "Could not resolve cloudflared target directory.".to_string())?;
        let tarball = parent.join("cloudflared-download.tgz");
        std::fs::write(&tarball, bytes).map_err(|e| format!("Could not save cloudflared archive: {e}"))?;
        let status = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(parent)
            .status()
            .map_err(|e| format!("Could not extract cloudflared archive: {e}"))?;
        let _ = std::fs::remove_file(&tarball);
        if !status.success() {
            return Err("Failed to extract cloudflared archive.".to_string());
        }
        let extracted = parent.join("cloudflared");
        if !extracted.exists() {
            return Err("cloudflared archive extracted but binary was not found.".to_string());
        }
        if extracted != target {
            std::fs::rename(&extracted, target)
                .map_err(|e| format!("Could not move cloudflared binary: {e}"))?;
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(target)
            .map_err(|e| format!("Could not read cloudflared permissions: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(target, perms)
            .map_err(|e| format!("Could not set cloudflared as executable: {e}"))?;
    }

    info!("Downloaded cloudflared to {}", target.display());
    Ok(())
}

pub fn resolve_cloudflared_path(app_data_path: &Path, resource_dir: Option<PathBuf>) -> PathBuf {
    if let Some(res) = resource_dir {
        let sidecar = res.join(cloudflared_sidecar_filename());
        if sidecar.exists() {
            return sidecar;
        }
        let sidecar2 = res.join("binaries").join(cloudflared_sidecar_filename());
        if sidecar2.exists() {
            return sidecar2;
        }
    }
    app_data_path.join(cloudflared_binary_name())
}

fn cloudflared_binary_name() -> &'static str {
    if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    }
}

fn cloudflared_sidecar_filename() -> String {
    let target = if cfg!(windows) {
        "x86_64-pc-windows-msvc".to_string()
    } else if cfg!(target_os = "macos") {
        let arch = if ARCH == "aarch64" {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        };
        arch.to_string()
    } else if cfg!(target_os = "linux") {
        let arch = if ARCH == "aarch64" {
            "aarch64-unknown-linux-gnu"
        } else {
            "x86_64-unknown-linux-gnu"
        };
        arch.to_string()
    } else {
        format!("{ARCH}-{OS}")
    };

    if cfg!(windows) {
        format!("cloudflared-{target}.exe")
    } else {
        format!("cloudflared-{target}")
    }
}

fn cloudflared_download_url() -> Result<String, String> {
    let artifact = match (OS, ARCH) {
        ("windows", "x86_64") => "cloudflared-windows-amd64.exe",
        ("windows", "aarch64") => "cloudflared-windows-arm64.exe",
        ("linux", "x86_64") => "cloudflared-linux-amd64",
        ("linux", "aarch64") => "cloudflared-linux-arm64",
        ("macos", "x86_64") => "cloudflared-darwin-amd64.tgz",
        ("macos", "aarch64") => "cloudflared-darwin-arm64.tgz",
        _ => {
            return Err(format!(
                "Unsupported platform for automatic cloudflared download: os={OS}, arch={ARCH}"
            ))
        }
    };
    Ok(format!(
        "https://github.com/cloudflare/cloudflared/releases/latest/download/{artifact}"
    ))
}
