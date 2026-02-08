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
    /// Disable streaming responses (use non-streaming mode for all requests)
    /// Useful for models that struggle with tool calls in streaming mode
    #[serde(default)]
    pub disable_streaming: bool,
    /// Disable tool calls entirely (faster for simple queries)
    /// When true, the LLM won't be told about available tools
    #[serde(default)]
    pub disable_tools: bool,
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
            disable_streaming: false,
            disable_tools: false,
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
            eprintln!("✓ Loaded configuration from: {}", path.display());
            Ok(config)
        } else {
            eprintln!("⚠ Configuration file not found: {}", path.display());
            eprintln!("  Using default configuration.");
            eprintln!();
            eprintln!("  To customize settings, create a config file:");
            eprintln!("    mkdir -p ~/.config/ritsu");
            eprintln!("    ritsu-server --write-example-config");
            eprintln!();
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

    /// Write an example configuration file
    pub fn write_example(path: &PathBuf) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
        }

        let example = format!(r#"[llm]
default_backend = "ollama"
disable_streaming = false
disable_tools = false

# Ollama (local)
[[llm.backends]]
name = "ollama"
endpoint = "http://localhost:11434"
model = "llama3.2"

# Groq (fast cloud inference) - uncomment and set GROQ_API_KEY env var
# [[llm.backends]]
# name = "groq"
# endpoint = "https://api.groq.com/openai/v1"
# model = "llama-3.3-70b-versatile"
# api_key_env = "GROQ_API_KEY"

# OpenAI - uncomment and set OPENAI_API_KEY env var
# [[llm.backends]]
# name = "openai"
# endpoint = "https://api.openai.com/v1"
# model = "gpt-4"
# api_key_env = "OPENAI_API_KEY"

# Anthropic - uncomment and set ANTHROPIC_API_KEY env var
# [[llm.backends]]
# name = "anthropic"
# endpoint = "https://api.anthropic.com"
# model = "claude-3-5-sonnet-20241022"
# api_key_env = "ANTHROPIC_API_KEY"

[server]
socket_path = "{socket_path}"
database_path = "{database_path}"
client_binary_path = "{client_binary_path}"

[memory]
daily_rotation_days = {rotation_days}

[timeouts]
user_response_seconds = {user_response}
http_request_seconds = {http_request}
llm_request_seconds = {llm_request}
"#,
            socket_path = default_socket_path(),
            database_path = default_database_path(),
            client_binary_path = default_client_binary(),
            rotation_days = default_rotation_days(),
            user_response = default_user_response(),
            http_request = default_http_request(),
            llm_request = default_llm_request(),
        );
        
        std::fs::write(path, example)
            .with_context(|| format!("Failed to write example config to: {}", path.display()))?;
        
        eprintln!("✓ Created example config: {}", path.display());
        Ok(())
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
