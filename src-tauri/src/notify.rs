use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
    {
        tracing::warn!("Could not show notification: {e}");
    }
}

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .map_err(|e| format!("Clipboard unavailable: {e}"))?
        .set_text(text)
        .map_err(|e| format!("Could not copy to clipboard: {e}"))
}
