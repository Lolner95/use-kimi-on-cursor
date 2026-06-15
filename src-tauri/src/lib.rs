mod commands;
pub mod config;
mod crypto;
mod cursor_settings;
mod doctor;
pub mod gateway;
mod logging;
mod notify;
pub mod state;
pub mod tunnel;

use config::ConfigStore;
use notify::{copy_to_clipboard, notify};
use state::AppState;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};
use tracing::info;
use tunnel::manager::resolve_cloudflared_path;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = ConfigStore::load().expect("failed to load config");
    let logs_dir = config.paths.logs_dir.clone();
    logging::init_logging(&logs_dir).expect("failed to init logging");

    let app_state = Arc::new(AppState::new(config));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_moonshot_key,
            commands::test_moonshot_key,
            commands::start_gateway,
            commands::stop_gateway,
            commands::restart_gateway,
            commands::get_gateway_status,
            commands::update_settings,
            commands::rotate_gateway_key,
            commands::complete_wizard,
            commands::get_logs,
            commands::clear_logs,
            commands::run_doctor_checks,
            commands::set_autostart,
            commands::is_autostart_enabled,
            commands::export_diagnostics,
            commands::download_cloudflared_cmd,
            commands::get_app_info,
            commands::inspect_cursor_install,
            commands::apply_cursor_settings,
            commands::get_cursor_alignment,
            commands::get_token_usage,
            commands::get_token_usage_for_date,
        ])
        .setup(move |app| {
            if let Ok(resource_dir) = app.path().resource_dir() {
                let data_dir = {
                    let config = app_state.config.lock();
                    config.paths.data_dir.clone()
                };
                *app_state.cloudflared_path.lock() =
                    resolve_cloudflared_path(&data_dir, Some(resource_dir));
            }
            // ── Window ───────────────────────────────────────────────────
            let show     = MenuItem::with_id(app, "show",     "Open Dashboard",    true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings",          true, None::<&str>)?;
            let hide     = MenuItem::with_id(app, "hide",     "Hide to Tray",      true, None::<&str>)?;
            let sep1     = PredefinedMenuItem::separator(app)?;
            // ── Gateway ──────────────────────────────────────────────────
            let start    = MenuItem::with_id(app, "start",    "Start Gateway",     true, None::<&str>)?;
            let stop     = MenuItem::with_id(app, "stop",     "Stop Gateway",      true, None::<&str>)?;
            let sep2     = PredefinedMenuItem::separator(app)?;
            // ── Clipboard ────────────────────────────────────────────────
            let copy_url = MenuItem::with_id(app, "copy_url", "Copy Cursor Base URL", true, None::<&str>)?;
            let copy_key = MenuItem::with_id(app, "copy_key", "Copy Cursor API Key",  true, None::<&str>)?;
            let sep3     = PredefinedMenuItem::separator(app)?;
            // ── App ──────────────────────────────────────────────────────
            let quit     = MenuItem::with_id(app, "quit",     "Quit",              true, None::<&str>)?;

            let menu = Menu::with_items(app, &[
                &show, &settings, &hide,
                &sep1,
                &start, &stop,
                &sep2,
                &copy_url, &copy_key,
                &sep3,
                &quit,
            ])?;

            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("Kimi Cursor Gateway")
                .on_menu_event({
                    let handle = app.handle().clone();
                    let state = app_state.clone();
                    move |app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "settings" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                                let _ = app.emit("navigate", "settings");
                            }
                        }
                        "hide" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.hide();
                            }
                        }
                        "start" => {
                            let h = handle.clone();
                            let s = state.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = s.start_gateway(&h).await;
                            });
                        }
                        "stop" => {
                            let h = handle.clone();
                            let s = state.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = s.stop_gateway(&h).await;
                            });
                        }
                        "copy_url" => {
                            let h = handle.clone();
                            let s = state.clone();
                            tauri::async_runtime::spawn(async move {
                                let status = s.status_snapshot().await;
                                if let Some(url) = status.public_base_url {
                                    let _ = copy_to_clipboard(&url);
                                    notify(&h, "Copied", "Cursor Base URL copied.");
                                } else {
                                    notify(
                                        &h,
                                        "Not ready",
                                        "Start the gateway first to get a Base URL.",
                                    );
                                }
                            });
                        }
                        "copy_key" => {
                            let h = handle.clone();
                            let s = state.clone();
                            tauri::async_runtime::spawn(async move {
                                let status = s.status_snapshot().await;
                                let _ = copy_to_clipboard(&status.gateway_key);
                                notify(&h, "Copied", "Cursor API key copied.");
                            });
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            let handle = app.handle().clone();
            let state = app_state.clone();
            let args: Vec<String> = std::env::args().collect();
            let minimized = args.iter().any(|a| a == "--minimized");

            // Mark the state so the first tunnel URL triggers a delayed Cursor sync
            // (gives Cursor time to finish its own boot before we write to its DB).
            if minimized {
                state.is_boot_start.store(true, std::sync::atomic::Ordering::Relaxed);
            }

            tauri::async_runtime::spawn(async move {
                let (auto_start, has_key) = {
                    let config = state.config.lock();
                    (
                        config.settings.auto_start_gateway,
                        config.settings.moonshot_key_encrypted.is_some(),
                    )
                };

                // When launched at system boot (--minimized flag), delay gateway startup
                // by 10 s so the OS finishes its own startup churn (network stack, DNS,
                // etc.) before cloudflared tries to reach Cloudflare's edge.
                if minimized && auto_start && has_key {
                    info!("Boot autostart: waiting 10 s for OS to stabilise before starting gateway…");
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }

                // Start as soon as a key exists, even if the wizard was never formally
                // finished - avoids a stale "incomplete wizard" state leaving Cursor with
                // no gateway/tunnel to talk to.
                if auto_start && has_key {
                    match state.start_gateway(&handle).await {
                        Ok(status) => {
                            notify(
                                &handle,
                                "Kimi Cursor Gateway",
                                if status.cursor_ready {
                                    "Gateway running - Cursor is configured and ready."
                                } else {
                                    "Gateway started. Establishing secure tunnel…"
                                },
                            );
                        }
                        Err(e) => {
                            tracing::error!("Auto-start gateway failed: {e}");
                            notify(&handle, "Gateway failed to start", &e);
                        }
                    }
                }

                if minimized {
                    if let Some(window) = handle.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
            });

            info!("Kimi Cursor Gateway initialized");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
