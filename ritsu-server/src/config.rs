//! Configuration management

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub timeouts: TimeoutConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_backend")]
    pub default_backend: String,
    #[serde(default)]
    pub backends: Vec<LlmBackend>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmBackend {
    pub name: String,
    pub endpoint: String,
    pub model: String,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)]
pub struct ServerConfig {
    #[serde(default = "default_socket_path")]
    pub socket_path: String,
    #[serde(default = "default_database_path")]
    pub database_path: String,
    #[serde(default = "default_client_binary")]
    pub client_binary_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_rotation_days")]
    pub daily_rotation_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)]
pub struct TimeoutConfig {
    #[serde(default = "default_user_response")]
    pub user_response_seconds: u64,
    #[serde(default = "default_http_request")]
    pub http_request_seconds: u64,
    #[serde(default = "default_llm_request")]
    pub llm_request_seconds: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            default_backend: "ollama".to_string(),
            backends: vec![LlmBackend {
                name: "ollama".to_string(),
                endpoint: "http://localhost:11434".to_string(),
                model: "llama3.2".to_string(),
                api_key_env: None,
            }],
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            socket_path: default_socket_path(),
            database_path: default_database_path(),
            client_binary_path: default_client_binary(),
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            daily_rotation_days: default_rotation_days(),
        }
    }
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            user_response_seconds: default_user_response(),
            http_request_seconds: default_http_request(),
            llm_request_seconds: default_llm_request(),
        }
    }
}

impl Config {
    /// Load configuration from file or use defaults
    pub fn load() -> Result<Self> {
        let config_path = Self::config_file_path();
        Self::load_from_path(&config_path)
    }

    /// Load configuration from a specific path
    pub fn load_from_path(path: &PathBuf) -> Result<Self> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file: {}", path.display()))?;
            let config: Self = toml::from_str(&contents)
                .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Get default configuration file path
    #[must_use]
    pub fn config_file_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ritsu")
            .join("config.toml")
    }
}

impl TimeoutConfig {
    #[must_use]
    #[allow(dead_code)]
    pub const fn user_response(&self) -> Duration {
        Duration::from_secs(self.user_response_seconds)
    }

    #[must_use]
    #[allow(dead_code)]
    pub const fn http_request(&self) -> Duration {
        Duration::from_secs(self.http_request_seconds)
    }

    #[must_use]
    #[allow(dead_code)]
    pub const fn llm_request(&self) -> Duration {
        Duration::from_secs(self.llm_request_seconds)
    }
}

fn default_backend() -> String {
    "ollama".to_string()
}

fn default_socket_path() -> String {
    "/tmp/ritsu.sock".to_string()
}

fn default_database_path() -> String {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ritsu")
        .join("ritsu.db")
        .to_string_lossy()
        .to_string()
}

fn default_client_binary() -> String {
    "/usr/bin/ritsu".to_string()
}

const fn default_rotation_days() -> u32 {
    40
}

const fn default_user_response() -> u64 {
    300 // 5 minutes
}

const fn default_http_request() -> u64 {
    30
}

const fn default_llm_request() -> u64 {
    60
}
