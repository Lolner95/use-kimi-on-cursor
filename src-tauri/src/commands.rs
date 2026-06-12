use crate::config::AppSettings;
use crate::crypto::mask_secret;
use crate::doctor::run_doctor;
use crate::state::{AppState, GatewayStatus};
use serde::Serialize;
use std::fs;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub moonshot_key_masked: Option<String>,
    pub gateway_key: String,
    pub local_port: u16,
    pub real_model: String,
    pub alias_model: String,
    pub force_non_streaming: bool,
    pub thinking_disabled: bool,
    pub sanitize_tools: bool,
    pub max_tokens_default: u32,
    pub inject_reasoning_placeholder: bool,
    pub autostart_enabled: bool,
    pub auto_start_gateway: bool,
    pub wizard_completed: bool,
    pub logs_dir: String,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<SettingsView, String> {
    let config = state.config.lock();
    let moonshot_masked = match config.settings.get_moonshot_key() {
        Ok(Some(k)) => Some(mask_secret(&k)),
        _ => None,
    };

    Ok(SettingsView {
        moonshot_key_masked: moonshot_masked,
        gateway_key: config.settings.gateway_key.clone(),
        local_port: config.settings.local_port,
        real_model: config.settings.real_model.clone(),
        alias_model: config.settings.alias_model.clone(),
        force_non_streaming: config.settings.force_non_streaming,
        thinking_disabled: config.settings.thinking_disabled,
        sanitize_tools: config.settings.sanitize_tools,
        max_tokens_default: config.settings.max_tokens_default,
        inject_reasoning_placeholder: config.settings.inject_reasoning_placeholder,
        autostart_enabled: config.settings.autostart_enabled,
        auto_start_gateway: config.settings.auto_start_gateway,
        wizard_completed: config.settings.wizard_completed,
        logs_dir: config.paths.logs_dir.display().to_string(),
    })
}

#[tauri::command]
pub async fn save_moonshot_key(
    state: State<'_, Arc<AppState>>,
    key: String,
) -> Result<(), String> {
    {
        let mut config = state.config.lock();
        config
            .settings
            .set_moonshot_key(key.trim())
            .map_err(|e| e.to_string())?;
        config.save().map_err(|e| e.to_string())?;
    }
    state.push_log("Moonshot API key saved.".to_string());
    Ok(())
}

#[tauri::command]
pub async fn test_moonshot_key(
    state: State<'_, Arc<AppState>>,
    key: Option<String>,
) -> Result<String, String> {
    use crate::config::MOONSHOT_MODELS_URL;
    use reqwest::Client;

    let api_key = if let Some(k) = key.filter(|s| !s.trim().is_empty()) {
        k
    } else {
        let config = state.config.lock();
        config
            .settings
            .get_moonshot_key()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "No Moonshot API key configured.".to_string())?
    };

    let client = Client::new();
    let resp = client
        .get(MOONSHOT_MODELS_URL)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .send()
        .await
        .map_err(|e| format!("Could not reach Moonshot: {e}"))?;

    if resp.status().is_success() {
        Ok("Moonshot accepted your API key.".to_string())
    } else {
        Err(format!(
            "Moonshot rejected your API key (HTTP {}). Use a valid Kimi Open Platform key.",
            resp.status()
        ))
    }
}

#[tauri::command]
pub async fn start_gateway(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<GatewayStatus, String> {
    state.start_gateway(&app).await
}

#[tauri::command]
pub async fn stop_gateway(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<GatewayStatus, String> {
    state.stop_gateway(&app).await
}

#[tauri::command]
pub async fn restart_gateway(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<GatewayStatus, String> {
    state.restart_gateway(&app).await
}

#[tauri::command]
pub async fn get_gateway_status(
    state: State<'_, Arc<AppState>>,
) -> Result<GatewayStatus, String> {
    Ok(state.status_snapshot().await)
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<(), String> {
    let mut config = state.config.lock();
    let existing_key = config.settings.moonshot_key_encrypted.clone();
    let existing_gateway = config.settings.gateway_key.clone();
    config.settings = settings;
    config.settings.moonshot_key_encrypted = existing_key;
    if config.settings.gateway_key.is_empty() {
        config.settings.gateway_key = existing_gateway;
    }
    config.save().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn rotate_gateway_key(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let mut config = state.config.lock();
    config.settings.rotate_gateway_key();
    let key = config.settings.gateway_key.clone();
    config.save().map_err(|e| e.to_string())?;
    state.push_log("Gateway API key rotated. Update Cursor with the new key.".to_string());
    Ok(key)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub portable_mode: bool,
    pub data_dir: String,
    pub exe_dir: Option<String>,
}

#[tauri::command]
pub fn get_app_info(state: State<'_, Arc<AppState>>) -> Result<AppInfo, String> {
    let config = state.config.lock();
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        portable_mode: config.paths.portable_mode(),
        data_dir: config.paths.root.display().to_string(),
        exe_dir: crate::config::exe_directory().map(|p| p.display().to_string()),
    })
}

#[tauri::command]
pub fn inspect_cursor_install() -> Result<crate::cursor_settings::CursorInstallInfo, String> {
    crate::cursor_settings::inspect_cursor_install().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn apply_cursor_settings(
    state: State<'_, Arc<AppState>>,
) -> Result<crate::cursor_settings::ApplyCursorSettingsResult, String> {
    let alignment = state.sync_cursor_settings("manual apply").await?;
    let status = state.status_snapshot().await;
    Ok(crate::cursor_settings::ApplyCursorSettingsResult {
        applied: true,
        db_path: alignment.db_path.clone(),
        exe_path: crate::cursor_settings::cursor_exe_path().map(|p| p.display().to_string()),
        base_url: status.public_base_url.unwrap_or_default(),
        model: status.alias_model,
        message: if alignment.aligned {
            "Cursor settings synced. Restart Cursor (or reload window) if it was already open."
                .to_string()
        } else {
            format!("Cursor updated but still misaligned: {}", alignment.issues.join(" "))
        },
        alignment,
    })
}

#[tauri::command]
pub async fn get_cursor_alignment(
    state: State<'_, Arc<AppState>>,
) -> Result<crate::cursor_settings::CursorAlignmentStatus, String> {
    let status = state.status_snapshot().await;
    let base = status
        .public_base_url
        .ok_or_else(|| "Start the gateway and wait for the tunnel URL.".to_string())?;
    let (gateway_key, alias_model) = {
        let config = state.config.lock();
        (
            config.settings.gateway_key.clone(),
            config.settings.alias_model.clone(),
        )
    };
    crate::cursor_settings::verify_cursor_alignment(&gateway_key, &base, &alias_model)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_token_usage(
    state: State<'_, Arc<AppState>>,
) -> Result<crate::gateway::UsageStatsSnapshot, String> {
    Ok(state.usage.snapshot())
}

#[tauri::command]
pub async fn get_token_usage_for_date(
    state: State<'_, Arc<AppState>>,
    date: String,
) -> Result<Vec<crate::gateway::usage_store::TokenUsageEvent>, String> {
    Ok(state.usage.events_for_date(&date))
}

#[tauri::command]
pub async fn complete_wizard(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    enable_autostart: bool,
) -> Result<(), String> {
    {
        let mut config = state.config.lock();
        config.settings.wizard_completed = true;
        config.settings.autostart_enabled = enable_autostart;
        config.settings.auto_start_gateway = true;
        config.settings.start_minimized = true;
        config.save().map_err(|e| e.to_string())?;
    }

    // Always register/unregister with Windows startup based on the user's choice.
    // The wizard checkbox defaults to checked, so autostart is on unless explicitly
    // unchecked. We always call the API (not just on enable) so the registry stays
    // in sync with the saved setting.
    if enable_autostart {
        app.autolaunch()
            .enable()
            .map_err(|e| format!("Could not register Windows startup entry: {e}"))?;
        state.push_log("Windows autostart enabled — gateway will start automatically on boot.".to_string());
    } else {
        // Best-effort removal; ignore if it was never registered.
        let _ = app.autolaunch().disable();
        state.push_log("Windows autostart disabled.".to_string());
    }

    state.push_log("Setup wizard completed.".to_string());
    Ok(())
}

#[tauri::command]
pub async fn get_logs(state: State<'_, Arc<AppState>>) -> Result<Vec<String>, String> {
    Ok(state.get_logs())
}

#[tauri::command]
pub async fn clear_logs(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.clear_logs();
    Ok(())
}

#[tauri::command]
pub async fn run_doctor_checks(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<crate::doctor::DoctorCheck>, String> {
    Ok(run_doctor(&state).await)
}

#[tauri::command]
pub async fn set_autostart(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<bool, String> {
    let autostart = app.autolaunch();
    if enabled {
        autostart.enable().map_err(|e| e.to_string())?;
    } else {
        autostart.disable().map_err(|e| e.to_string())?;
    }

    let mut config = state.config.lock();
    config.settings.autostart_enabled = enabled;
    config.save().map_err(|e| e.to_string())?;
    Ok(enabled)
}

#[tauri::command]
pub async fn is_autostart_enabled(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<bool, String> {
    let enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    let mut config = state.config.lock();
    config.settings.autostart_enabled = enabled;
    Ok(enabled)
}

#[tauri::command]
pub async fn export_diagnostics(
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let (logs_dir, redacted_config) = {
        let config = state.config.lock();
        (
            config.paths.logs_dir.clone(),
            config.settings.redacted_for_export(),
        )
    };
    let status = state.status_snapshot().await;

    let export_dir = logs_dir.parent().unwrap_or(&logs_dir).join("diagnostics");
    fs::create_dir_all(&export_dir).map_err(|e| e.to_string())?;

    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let zip_path = export_dir.join(format!("kimi-diagnostics-{timestamp}.zip"));

    let file = fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();

    let meta = serde_json::json!({
        "app": "Kimi Cursor Gateway",
        "version": "1.0.0",
        "windowsVersion": std::env::consts::OS,
        "config": redacted_config,
        "gatewayStatus": status,
    });
    zip.start_file("metadata.json", options)
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(
        &mut zip,
        serde_json::to_string_pretty(&meta)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;

    for name in ["app.log", "proxy.log", "tunnel.log", "requests.log", "errors.log"] {
        let path = logs_dir.join(name);
        if path.exists() {
            let content = fs::read(&path).map_err(|e| e.to_string())?;
            zip.start_file(format!("logs/{name}"), options)
                .map_err(|e| e.to_string())?;
            std::io::Write::write_all(&mut zip, &content).map_err(|e| e.to_string())?;
        }
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(zip_path.display().to_string())
}

#[tauri::command]
pub async fn download_cloudflared_cmd(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let path = state.cloudflared_path.lock().clone();
    crate::tunnel::manager::download_cloudflared(&path).await?;
    Ok(path.display().to_string())
}
