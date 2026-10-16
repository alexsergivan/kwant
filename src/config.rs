use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 8799;
pub const DEFAULT_LAYA_URL: &str = "http://127.0.0.1:8000";
pub const DEFAULT_FALLBACK_MODEL: &str = "grok-beta";

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("I/O error accessing configuration: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Failed to serialize configuration to TOML: {0}")]
    Serialize(#[from] toml::ser::Error),

    #[error("Invalid configuration value: {0}")]
    Invalid(String),
}

fn default_host() -> String {
    DEFAULT_HOST.to_string()
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn default_laya_url() -> String {
    DEFAULT_LAYA_URL.to_string()
}

fn default_true() -> bool {
    true
}

fn default_laya_models() -> String {
    "english".to_string()
}

fn default_fallback_model() -> String {
    DEFAULT_FALLBACK_MODEL.to_string()
}

pub fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".kwnt"))
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".kwnt"))
                .unwrap_or_else(|_| PathBuf::from(".kwnt"))
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    #[serde(default = "default_laya_url")]
    pub laya_url: String,

    /// When true, `kwnt serve` installs, starts and supervises a local `laya-serve`.
    #[serde(default = "default_true")]
    pub laya_managed: bool,

    /// Laya checkpoints the managed sidecar preloads (comma list).
    #[serde(default = "default_laya_models")]
    pub laya_models: String,

    #[serde(default = "default_fallback_model")]
    pub default_fallback_model: String,

    /// Optional private browser service URL (server-side only).
    #[serde(default)]
    pub browser_cdp_url: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            data_dir: default_data_dir(),
            laya_url: default_laya_url(),
            laya_managed: true,
            laya_models: default_laya_models(),
            default_fallback_model: default_fallback_model(),
            browser_cdp_url: None,
        }
    }
}

impl Config {
    /// Loads configuration from default file location (~/.kwnt/config.toml) if present,
    /// falling back to defaults and applying environment variable overrides.
    pub fn load() -> Result<Self, ConfigError> {
        let data_dir = if let Ok(dir) = std::env::var("KWNT_DATA_DIR") {
            PathBuf::from(dir)
        } else {
            default_data_dir()
        };

        let config_file = data_dir.join("config.toml");
        let mut cfg = if config_file.exists() {
            Self::from_file(&config_file)?
        } else {
            Self::default()
        };

        cfg.apply_env();
        Ok(cfg)
    }

    /// Loads default configuration overridden by environment variables.
    pub fn from_env_or_default() -> Self {
        let mut cfg = Self::default();
        cfg.apply_env();
        cfg
    }

    /// Loads configuration from a specific TOML file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&content)?;
        Ok(cfg)
    }

    /// Saves configuration to a specific TOML file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Applies environment variable overrides onto this configuration.
    pub fn apply_env(&mut self) {
        if let Ok(url) = std::env::var("KWNT_BROWSER_CDP_URL") {
            self.browser_cdp_url = Some(url);
        }
        if let Ok(host) = std::env::var("KWNT_HOST").or_else(|_| std::env::var("HOST")) {
            if !host.is_empty() {
                self.host = host;
            }
        }
        if let Ok(port_str) = std::env::var("KWNT_PORT").or_else(|_| std::env::var("PORT")) {
            if let Ok(port) = port_str.parse::<u16>() {
                self.port = port;
            }
        }
        if let Ok(dir) = std::env::var("KWNT_DATA_DIR") {
            if !dir.is_empty() {
                self.data_dir = PathBuf::from(dir);
            }
        }
        if let Ok(url) = std::env::var("LAYA_URL").or_else(|_| std::env::var("KWNT_LAYA_URL")) {
            if !url.is_empty() {
                self.laya_url = url;
            }
        }
        if let Ok(v) = std::env::var("KWNT_LAYA_MANAGED") {
            self.laya_managed = !matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "no" | "off");
        }
        if let Ok(models) = std::env::var("KWNT_LAYA_MODELS") {
            if !models.trim().is_empty() {
                self.laya_models = models.trim().to_string();
            }
        }
        if let Ok(model) = std::env::var("KWNT_DEFAULT_FALLBACK_MODEL")
            .or_else(|_| std::env::var("DEFAULT_FALLBACK_MODEL"))
        {
            if !model.is_empty() {
                self.default_fallback_model = model;
            }
        }
    }
}
