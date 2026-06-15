use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::crypto::{
    decrypt_secret, encrypt_secret, generate_gateway_key, is_openai_style_gateway_key, CryptoError,
};

pub const APP_NAME: &str = "KimiCursorGateway";
pub const DEFAULT_PORT: u16 = 4001;
pub const DEFAULT_REAL_MODEL: &str = "kimi-k2.7";
pub const DEFAULT_ALIAS_MODEL: &str = "gpt-5-high-max";
pub const DEFAULT_MAX_TOKENS: u32 = 32_768;
pub const MAX_CONTEXT_TOKENS: u32 = 256 * 1024;

fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

fn default_inject_reasoning() -> bool {
    true
}
pub const MOONSHOT_API_URL: &str = "https://api.moonshot.ai/v1/chat/completions";
pub const MOONSHOT_MODELS_URL: &str = "https://api.moonshot.ai/v1/models";
pub const MOONSHOT_FILES_URL: &str = "https://api.moonshot.ai/v1/files";
pub const MOONSHOT_EMBEDDINGS_URL: &str = "https://api.moonshot.ai/v1/embeddings";
pub const MOONSHOT_BASE_URL: &str = "https://api.moonshot.ai/v1";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
}

/// Portable mode stores all data next to the executable (when `portable` marker exists).
pub fn is_portable_mode() -> bool {
    if std::env::args().any(|a| a == "--portable") {
        return true;
    }
    portable_marker_path()
        .map(|p| p.exists())
        .unwrap_or(false)
}

pub fn portable_marker_path() -> Option<PathBuf> {
    exe_directory().map(|dir| dir.join("portable"))
}

pub fn exe_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

fn resolve_data_root() -> Result<PathBuf, ConfigError> {
    if is_portable_mode() {
        let dir = exe_directory().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "executable path not found")
        })?;
        let root = dir.join("KimiCursorGatewayData");
        fs::create_dir_all(&root)?;
        return Ok(root);
    }

    let base = dirs::data_local_dir()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "local app data not found")
        })?
        .join(APP_NAME);
    fs::create_dir_all(&base)?;
    Ok(base)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub moonshot_key_encrypted: Option<String>,
    pub gateway_key: String,
    pub local_port: u16,
    pub real_model: String,
    pub alias_model: String,
    pub force_non_streaming: bool,
    pub thinking_disabled: bool,
    pub sanitize_tools: bool,
    #[serde(default = "default_max_tokens")]
    pub max_tokens_default: u32,
    #[serde(default = "default_inject_reasoning")]
    pub inject_reasoning_placeholder: bool,
    pub autostart_enabled: bool,
    pub auto_start_gateway: bool,
    pub wizard_completed: bool,
    pub start_minimized: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            moonshot_key_encrypted: None,
            gateway_key: generate_gateway_key(),
            local_port: DEFAULT_PORT,
            real_model: DEFAULT_REAL_MODEL.to_string(),
            alias_model: DEFAULT_ALIAS_MODEL.to_string(),
            force_non_streaming: false,
            thinking_disabled: false,
            sanitize_tools: true,
            max_tokens_default: DEFAULT_MAX_TOKENS,
            inject_reasoning_placeholder: true,
            autostart_enabled: false,
            auto_start_gateway: true,
            wizard_completed: false,
            start_minimized: true,
        }
    }
}

impl AppSettings {
    pub fn set_moonshot_key(&mut self, key: &str) -> Result<(), ConfigError> {
        self.moonshot_key_encrypted = Some(encrypt_secret(key)?);
        Ok(())
    }

    pub fn get_moonshot_key(&self) -> Result<Option<String>, ConfigError> {
        match &self.moonshot_key_encrypted {
            Some(enc) => Ok(Some(decrypt_secret(enc)?)),
            None => Ok(None),
        }
    }

    pub fn rotate_gateway_key(&mut self) {
        self.gateway_key = generate_gateway_key();
    }

    pub fn redacted_for_export(&self) -> serde_json::Value {
        serde_json::json!({
            "localPort": self.local_port,
            "realModel": self.real_model,
            "aliasModel": self.alias_model,
            "forceNonStreaming": self.force_non_streaming,
            "thinkingDisabled": self.thinking_disabled,
            "sanitizeTools": self.sanitize_tools,
            "maxTokensDefault": self.max_tokens_default,
            "injectReasoningPlaceholder": self.inject_reasoning_placeholder,
            "autostartEnabled": self.autostart_enabled,
            "autoStartGateway": self.auto_start_gateway,
            "wizardCompleted": self.wizard_completed,
            "hasMoonshotKey": self.moonshot_key_encrypted.is_some(),
            "gatewayKeyPrefix": self.gateway_key.chars().take(4).collect::<String>(),
            "portableMode": is_portable_mode(),
        })
    }
}

pub struct AppPaths {
    pub root: PathBuf,
    pub config_file: PathBuf,
    pub logs_dir: PathBuf,
    pub data_dir: PathBuf,
    pub usage_dir: PathBuf,
}

impl AppPaths {
    pub fn new() -> Result<Self, ConfigError> {
        let root = resolve_data_root()?;
        let logs_dir = root.join("logs");
        let data_dir = root.join("data");
        let usage_dir = root.join("usage");
        fs::create_dir_all(&logs_dir)?;
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&usage_dir)?;

        Ok(Self {
            config_file: root.join("config.json"),
            root,
            logs_dir,
            data_dir,
            usage_dir,
        })
    }

    pub fn cloudflared_path(&self) -> PathBuf {
        #[cfg(windows)]
        {
            self.data_dir.join("cloudflared.exe")
        }
        #[cfg(not(windows))]
        {
            self.data_dir.join("cloudflared")
        }
    }

    pub fn portable_mode(&self) -> bool {
        is_portable_mode()
    }
}

pub struct ConfigStore {
    pub paths: AppPaths,
    pub settings: AppSettings,
}

impl ConfigStore {
    pub fn load() -> Result<Self, ConfigError> {
        let paths = AppPaths::new()?;
        let mut settings = if paths.config_file.exists() {
            let raw = fs::read_to_string(&paths.config_file)?;
            serde_json::from_str(&raw)?
        } else {
            AppSettings::default()
        };

        let mut migrated = false;
        if !is_openai_style_gateway_key(&settings.gateway_key) {
            settings.gateway_key = generate_gateway_key();
            migrated = true;
        }

        let config_exists = paths.config_file.exists();
        let store = Self { paths, settings };
        if migrated || !config_exists {
            store.save()?;
        }
        Ok(store)
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let json = serde_json::to_string_pretty(&self.settings)?;
        fs::write(&self.paths.config_file, json)?;
        Ok(())
    }
}

pub fn ensure_portable_marker(dir: &Path) -> Result<(), ConfigError> {
    let marker = dir.join("portable");
    if !marker.exists() {
        fs::write(&marker, "Kimi Cursor Gateway portable mode\n")?;
    }
    Ok(())
}
