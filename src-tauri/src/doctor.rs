use crate::config::{AppSettings, MOONSHOT_MODELS_URL};
use crate::state::AppState;
use reqwest::Client;
use serde::Serialize;
use std::net::TcpListener;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheck {
    pub id: String,
    pub label: String,
    pub status: DoctorStatus,
    pub detail: String,
    pub repairable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

pub async fn run_doctor(state: &AppState) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();

    let (settings, logs_dir, cloudflared_path) = {
        let config = state.config.lock();
        (
            config.settings.clone(),
            config.paths.logs_dir.clone(),
            state.cloudflared_path.lock().clone(),
        )
    };

    checks.push(check_moonshot_key_exists(&settings));
    checks.push(check_moonshot_key_works(&settings).await);
    checks.push(check_port_available(settings.local_port));
    checks.push(check_local_gateway(state, settings.local_port).await);
    checks.push(check_tunnel(state).await);
    checks.push(check_public_url(state).await);
    checks.push(check_cursor_settings(state).await);
    checks.push(check_last_request(state).await);
    checks.push(check_cloudflared(&cloudflared_path));
    checks.push(check_autostart(&settings));
    checks.push(check_logs_dir(&logs_dir));

    checks
}

fn check_moonshot_key_exists(settings: &AppSettings) -> DoctorCheck {
    let has = settings.moonshot_key_encrypted.is_some();
    DoctorCheck {
        id: "moonshot_key".into(),
        label: "Moonshot API key saved".into(),
        status: if has { DoctorStatus::Pass } else { DoctorStatus::Fail },
        detail: if has {
            "Your Kimi key is stored securely.".into()
        } else {
            "Paste your Moonshot/Kimi API key in Settings.".into()
        },
        repairable: !has,
    }
}

async fn check_moonshot_key_works(settings: &AppSettings) -> DoctorCheck {
    let key = match settings.get_moonshot_key() {
        Ok(Some(k)) => k,
        Ok(None) => {
            return DoctorCheck {
                id: "moonshot_auth".into(),
                label: "Moonshot API key works".into(),
                status: DoctorStatus::Fail,
                detail: "No API key configured.".into(),
                repairable: true,
            };
        }
        Err(_) => {
            return DoctorCheck {
                id: "moonshot_auth".into(),
                label: "Moonshot API key works".into(),
                status: DoctorStatus::Fail,
                detail: "Could not decrypt saved key. Re-enter it.".into(),
                repairable: true,
            };
        }
    };

    let client = Client::new();
    let resp = client
        .get(MOONSHOT_MODELS_URL)
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => DoctorCheck {
            id: "moonshot_auth".into(),
            label: "Moonshot API key works".into(),
            status: DoctorStatus::Pass,
            detail: "Moonshot accepted your API key.".into(),
            repairable: false,
        },
        Ok(r) => DoctorCheck {
            id: "moonshot_auth".into(),
            label: "Moonshot API key works".into(),
            status: DoctorStatus::Fail,
            detail: format!(
                "Moonshot rejected your key (HTTP {}). Check your Kimi Open Platform key.",
                r.status()
            ),
            repairable: true,
        },
        Err(e) => DoctorCheck {
            id: "moonshot_auth".into(),
            label: "Moonshot API key works".into(),
            status: DoctorStatus::Warn,
            detail: format!("Could not reach Moonshot to verify: {e}"),
            repairable: false,
        },
    }
}

fn check_port_available(port: u16) -> DoctorCheck {
    let ok = TcpListener::bind(("127.0.0.1", port)).is_ok()
        || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
    DoctorCheck {
        id: "port".into(),
        label: "Local gateway port".into(),
        status: if ok { DoctorStatus::Pass } else { DoctorStatus::Fail },
        detail: if ok {
            format!("Port {port} is available or already in use by the gateway.")
        } else {
            format!("Port {port} is busy. Change the port in Advanced settings.")
        },
        repairable: !ok,
    }
}

async fn check_local_gateway(_state: &AppState, port: u16) -> DoctorCheck {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = Client::new();
    match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => DoctorCheck {
            id: "local_gateway".into(),
            label: "Local gateway responds".into(),
            status: DoctorStatus::Pass,
            detail: "The local proxy is running.".into(),
            repairable: false,
        },
        Ok(r) => DoctorCheck {
            id: "local_gateway".into(),
            label: "Local gateway responds".into(),
            status: DoctorStatus::Warn,
            detail: format!("Gateway returned HTTP {}.", r.status()),
            repairable: true,
        },
        Err(_) => DoctorCheck {
            id: "local_gateway".into(),
            label: "Local gateway responds".into(),
            status: DoctorStatus::Fail,
            detail: "Local gateway is not running. Click Start Gateway.".into(),
            repairable: true,
        },
    }
}

async fn check_tunnel(state: &AppState) -> DoctorCheck {
    let url = state.public_url.read().await.clone();
    let has_tunnel = url.is_some();
    DoctorCheck {
        id: "tunnel".into(),
        label: "Public tunnel active".into(),
        status: if has_tunnel {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Fail
        },
        detail: url
            .map(|u| format!("Tunnel URL: {u}"))
            .unwrap_or_else(|| "No public tunnel yet. Start the gateway.".into()),
        repairable: !has_tunnel,
    }
}

async fn check_public_url(state: &AppState) -> DoctorCheck {
    let public = state.public_url.read().await.clone();
    let Some(root) = public else {
        return DoctorCheck {
            id: "public_health".into(),
            label: "Public URL responds".into(),
            status: DoctorStatus::Fail,
            detail: "Tunnel URL not available.".into(),
            repairable: true,
        };
    };

    let client = Client::new();
    let url = format!("{root}/health");
    match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => DoctorCheck {
            id: "public_health".into(),
            label: "Public URL responds".into(),
            status: DoctorStatus::Pass,
            detail: "Cursor can reach your gateway over HTTPS.".into(),
            repairable: false,
        },
        Ok(r) => DoctorCheck {
            id: "public_health".into(),
            label: "Public URL responds".into(),
            status: DoctorStatus::Warn,
            detail: format!("Public URL returned HTTP {}.", r.status()),
            repairable: true,
        },
        Err(e) => DoctorCheck {
            id: "public_health".into(),
            label: "Public URL responds".into(),
            status: DoctorStatus::Warn,
            detail: format!("Could not reach public URL yet: {e}"),
            repairable: true,
        },
    }
}

async fn check_cursor_settings(state: &AppState) -> DoctorCheck {
    let status = state.status_snapshot().await;
    if status.public_base_url.is_none() {
        return DoctorCheck {
            id: "cursor_settings".into(),
            label: "Cursor OpenAI override".into(),
            status: DoctorStatus::Fail,
            detail: "Start the gateway and wait for the tunnel URL.".into(),
            repairable: true,
        };
    }

    let Some(alignment) = status.cursor_alignment else {
        return DoctorCheck {
            id: "cursor_settings".into(),
            label: "Cursor OpenAI override".into(),
            status: DoctorStatus::Fail,
            detail: "Cursor database not found.".into(),
            repairable: false,
        };
    };

    if alignment.aligned {
        DoctorCheck {
            id: "cursor_settings".into(),
            label: "Cursor OpenAI override".into(),
            status: DoctorStatus::Pass,
            detail: format!(
                "useOpenAIKey on, base URL set to {}",
                alignment.expected_base_url
            ),
            repairable: false,
        }
    } else {
        DoctorCheck {
            id: "cursor_settings".into(),
            label: "Cursor OpenAI override".into(),
            status: DoctorStatus::Fail,
            detail: alignment.issues.join(" "),
            repairable: true,
        }
    }
}

async fn check_last_request(state: &AppState) -> DoctorCheck {
    let m = state.metrics.snapshot();
    if m.requests == 0 {
        return DoctorCheck {
            id: "last_request".into(),
            label: "Last Cursor request".into(),
            status: DoctorStatus::Warn,
            detail: "No requests yet. Send a message in Cursor to test.".into(),
            repairable: false,
        };
    }

    let status = if m.upstream_errors > 0 && m.last_error.is_some() {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };

    DoctorCheck {
        id: "last_request".into(),
        label: "Last Cursor request".into(),
        status,
        detail: m
            .last_error
            .clone()
            .unwrap_or_else(|| format!("Last status: {:?}, latency: {:?}ms", m.last_status, m.last_latency_ms)),
        repairable: false,
    }
}

fn check_cloudflared(path: &std::path::Path) -> DoctorCheck {
    let exists = path.exists();
    DoctorCheck {
        id: "cloudflared".into(),
        label: "cloudflared available".into(),
        status: if exists {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
        detail: if exists {
            format!("Found at {}", path.display())
        } else {
            "cloudflared will be downloaded on first gateway start.".into()
        },
        repairable: !exists,
    }
}

fn check_autostart(settings: &AppSettings) -> DoctorCheck {
    DoctorCheck {
        id: "autostart".into(),
        label: "Autostart configured".into(),
        status: if settings.autostart_enabled {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
        detail: if settings.autostart_enabled {
            "App will start automatically after login.".into()
        } else {
            "Autostart is off. Enable it in Controls if desired.".into()
        },
        repairable: false,
    }
}

fn check_logs_dir(path: &std::path::Path) -> DoctorCheck {
    let ok = path.exists();
    DoctorCheck {
        id: "logs".into(),
        label: "Logs directory".into(),
        status: if ok { DoctorStatus::Pass } else { DoctorStatus::Fail },
        detail: path.display().to_string(),
        repairable: !ok,
    }
}
