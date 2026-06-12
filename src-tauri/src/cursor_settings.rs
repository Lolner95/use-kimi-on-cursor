use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use thiserror::Error;

const APPLICATION_USER_KEY: &str =
    "src.vs.platform.reactivestorage.browser.reactiveStorageServiceImpl.persistentStorage.applicationUser";
const OPENAI_KEY_DB_KEY: &str = "cursorAuth/openAIKey";

#[derive(Debug, Error)]
pub enum CursorSettingsError {
    #[error("Cursor is not installed or its database was not found")]
    NotFound,
    #[error("database error: {0}")]
    Database(String),
    #[error("could not parse Cursor settings: {0}")]
    Parse(String),
    #[error("base URL must end with /v1")]
    InvalidBaseUrl,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorInstallInfo {
    pub db_path: String,
    pub exe_path: Option<String>,
    pub use_openai_key_before: Option<bool>,
    pub openai_base_url_before: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorAlignmentStatus {
    pub installed: bool,
    pub db_path: String,
    pub key_matches: bool,
    pub use_openai_key: bool,
    pub base_url_matches: bool,
    pub composer_model_matches: bool,
    pub aligned: bool,
    pub stored_key_prefix: Option<String>,
    pub expected_key_prefix: String,
    pub stored_base_url: Option<String>,
    pub expected_base_url: String,
    pub stored_composer_model: Option<String>,
    pub expected_model: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyCursorSettingsResult {
    pub applied: bool,
    pub db_path: String,
    pub exe_path: Option<String>,
    pub base_url: String,
    pub model: String,
    pub message: String,
    pub alignment: CursorAlignmentStatus,
}

pub fn cursor_db_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Cursor").join("User").join("globalStorage").join("state.vscdb"))
}

pub fn cursor_exe_path() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let candidates = [
        PathBuf::from(&local).join("Programs").join("cursor").join("Cursor.exe"),
        PathBuf::from(&local).join("Programs").join("Cursor").join("Cursor.exe"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

pub fn inspect_cursor_install() -> Result<CursorInstallInfo, CursorSettingsError> {
    let db_path = cursor_db_path().filter(|p| p.exists()).ok_or(CursorSettingsError::NotFound)?;
    let (use_openai_key_before, openai_base_url_before) = read_openai_toggle(&db_path)?;
    Ok(CursorInstallInfo {
        db_path: db_path.display().to_string(),
        exe_path: cursor_exe_path().map(|p| p.display().to_string()),
        use_openai_key_before,
        openai_base_url_before,
    })
}

pub fn verify_cursor_alignment(
    gateway_key: &str,
    base_url: &str,
    model: &str,
) -> Result<CursorAlignmentStatus, CursorSettingsError> {
    let normalized = normalize_base_url(base_url)?;
    let db_path = cursor_db_path().filter(|p| p.exists()).ok_or(CursorSettingsError::NotFound)?;
    let state = read_cursor_state(&db_path)?;

    let key_matches = state
        .stored_openai_key
        .as_deref()
        .map(|k| keys_equal(k, gateway_key))
        .unwrap_or(false);
    let use_openai_key = state.use_openai_key;
    let base_url_matches = state
        .open_ai_base_url
        .as_deref()
        .map(|u| urls_equal(u, &normalized))
        .unwrap_or(false);
    let composer_model_matches = state
        .composer_model
        .as_deref()
        .map(|m| m == model)
        .unwrap_or(false);

    let mut issues = Vec::new();
    if !key_matches {
        issues.push(
            "Cursor's stored OpenAI API key does not match the gateway key. Click Apply to Cursor."
                .to_string(),
        );
    }
    if !use_openai_key {
        issues.push(
            "Cursor is not using your custom OpenAI key (useOpenAIKey is off). Requests never reach the gateway."
                .to_string(),
        );
    }
    if !base_url_matches {
        issues.push(
            "Cursor's Override OpenAI Base URL is missing or outdated. Tunnel URLs change on restart."
                .to_string(),
        );
    }
    if !composer_model_matches {
        issues.push(format!(
            "Cursor composer model is {:?}, expected \"{model}\". Select the gateway model in Cursor.",
            state.composer_model
        ));
    }

    let aligned = key_matches && use_openai_key && base_url_matches;

    Ok(CursorAlignmentStatus {
        installed: true,
        db_path: db_path.display().to_string(),
        key_matches,
        use_openai_key,
        base_url_matches,
        composer_model_matches,
        aligned,
        stored_key_prefix: state
            .stored_openai_key
            .as_ref()
            .map(|k| mask_key(k)),
        expected_key_prefix: mask_key(gateway_key),
        stored_base_url: state.open_ai_base_url.clone(),
        expected_base_url: normalized,
        stored_composer_model: state.composer_model.clone(),
        expected_model: model.to_string(),
        issues,
    })
}

pub fn apply_cursor_settings(
    gateway_key: &str,
    base_url: &str,
    model: &str,
) -> Result<ApplyCursorSettingsResult, CursorSettingsError> {
    let normalized = normalize_base_url(base_url)?;
    let db_path = cursor_db_path().filter(|p| p.exists()).ok_or(CursorSettingsError::NotFound)?;

    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| CursorSettingsError::Database(e.to_string()))?;

    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (OPENAI_KEY_DB_KEY, gateway_key),
    )
    .map_err(|e| CursorSettingsError::Database(e.to_string()))?;

    update_application_user(&conn, &normalized, model)?;

    let alignment = verify_cursor_alignment(gateway_key, &normalized, model)?;

    Ok(ApplyCursorSettingsResult {
        applied: true,
        db_path: db_path.display().to_string(),
        exe_path: cursor_exe_path().map(|p| p.display().to_string()),
        base_url: normalized,
        model: model.to_string(),
        message: if alignment.aligned {
            "Cursor settings synced. Restart Cursor (or reload window) if it was already open.".to_string()
        } else {
            format!(
                "Cursor updated but still misaligned: {}",
                alignment.issues.join(" ")
            )
        },
        alignment,
    })
}

fn normalize_base_url(base_url: &str) -> Result<String, CursorSettingsError> {
    let mut url = base_url.trim().trim_end_matches('/').to_string();
    if !url.ends_with("/v1") {
        if url.ends_with("/v1/") {
            url.pop();
        } else {
            url.push_str("/v1");
        }
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(CursorSettingsError::InvalidBaseUrl);
    }
    Ok(url)
}

fn keys_equal(a: &str, b: &str) -> bool {
    a.trim() == b.trim()
}

fn urls_equal(a: &str, b: &str) -> bool {
    a.trim().trim_end_matches('/') == b.trim().trim_end_matches('/')
}

fn mask_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() <= 12 {
        return "***".to_string();
    }
    format!("{}...{}", &trimmed[..8], &trimmed[trimmed.len() - 4..])
}

#[derive(Debug)]
struct CursorState {
    use_openai_key: bool,
    open_ai_base_url: Option<String>,
    stored_openai_key: Option<String>,
    composer_model: Option<String>,
}

fn read_cursor_state(db_path: &Path) -> Result<CursorState, CursorSettingsError> {
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| CursorSettingsError::Database(e.to_string()))?;

    let stored_openai_key: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [OPENAI_KEY_DB_KEY],
            |row| row.get(0),
        )
        .ok();

    let row: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [APPLICATION_USER_KEY],
            |row| row.get(0),
        )
        .ok();

    let Some(raw) = row else {
        return Ok(CursorState {
            use_openai_key: false,
            open_ai_base_url: None,
            stored_openai_key,
            composer_model: None,
        });
    };

    let app: ApplicationUser = serde_json::from_str(&raw)
        .map_err(|e| CursorSettingsError::Parse(e.to_string()))?;

    Ok(CursorState {
        use_openai_key: app.use_openai_key,
        open_ai_base_url: app.open_ai_base_url,
        stored_openai_key,
        composer_model: app
            .ai_settings
            .as_ref()
            .and_then(|ai| ai.composer_model.clone()),
    })
}

fn read_openai_toggle(db_path: &Path) -> Result<(Option<bool>, Option<String>), CursorSettingsError> {
    let state = read_cursor_state(db_path)?;
    Ok((Some(state.use_openai_key), state.open_ai_base_url))
}

fn update_application_user(
    conn: &rusqlite::Connection,
    base_url: &str,
    model: &str,
) -> Result<(), CursorSettingsError> {
    let raw: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [APPLICATION_USER_KEY],
            |row| row.get(0),
        )
        .map_err(|_| CursorSettingsError::Parse("applicationUser row missing".into()))?;

    let mut app: ApplicationUser = serde_json::from_str(&raw)
        .map_err(|e| CursorSettingsError::Parse(e.to_string()))?;

    app.use_openai_key = true;
    app.open_ai_base_url = Some(base_url.to_string());

    if let Some(ai) = app.ai_settings.as_mut() {
        apply_model_to_ai_settings(ai, model);
    } else {
        let mut ai = AiSettings::default();
        apply_model_to_ai_settings(&mut ai, model);
        app.ai_settings = Some(ai);
    }

    let updated = serde_json::to_string(&app).map_err(|e| CursorSettingsError::Parse(e.to_string()))?;
    conn.execute(
        "UPDATE ItemTable SET value = ?1 WHERE key = ?2",
        rusqlite::params![updated, APPLICATION_USER_KEY],
    )
    .map_err(|e| CursorSettingsError::Database(e.to_string()))?;

    Ok(())
}

fn apply_model_to_ai_settings(ai: &mut AiSettings, model: &str) {
    if !ai.user_added_models.iter().any(|m| m == model) {
        ai.user_added_models.push(model.to_string());
    }
    if !ai.model_override_enabled.iter().any(|m| m == model) {
        ai.model_override_enabled.push(model.to_string());
    }
    ai.composer_model = Some(model.to_string());
    ai.cmd_k_model = Some(model.to_string());

    let mut config = ai.model_config.take().unwrap_or_default();
    for mode in ["composer", "cmd-k", "background-composer", "plan-execution"] {
        let entry = config.entry(mode.to_string()).or_insert_with(|| {
            json!({
                "modelName": model,
                "maxMode": false,
                "selectedModels": [{"modelId": model, "parameters": []}]
            })
        });
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("modelName".into(), json!(model));
            obj.insert(
                "selectedModels".into(),
                json!([{"modelId": model, "parameters": []}]),
            );
        }
    }
    ai.model_config = Some(config);
}

#[derive(Debug, Deserialize, Serialize)]
struct ApplicationUser {
    #[serde(rename = "useOpenAIKey", default)]
    use_openai_key: bool,
    #[serde(rename = "openAIBaseUrl", default)]
    open_ai_base_url: Option<String>,
    #[serde(rename = "aiSettings", default)]
    ai_settings: Option<AiSettings>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct AiSettings {
    #[serde(rename = "userAddedModels", default)]
    user_added_models: Vec<String>,
    #[serde(rename = "modelOverrideEnabled", default)]
    model_override_enabled: Vec<String>,
    #[serde(rename = "composerModel", default)]
    composer_model: Option<String>,
    #[serde(rename = "cmdKModel", default)]
    cmd_k_model: Option<String>,
    #[serde(rename = "modelConfig", default)]
    model_config: Option<serde_json::Map<String, Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_url_appends_v1() {
        let url = normalize_base_url("https://example.trycloudflare.com").unwrap();
        assert_eq!(url, "https://example.trycloudflare.com/v1");
    }

    #[test]
    fn urls_equal_ignore_trailing_slash() {
        assert!(urls_equal(
            "https://example.trycloudflare.com/v1/",
            "https://example.trycloudflare.com/v1"
        ));
    }
}
