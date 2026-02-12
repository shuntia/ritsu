//! Configuration management

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tracing::{info, warn};

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
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub triggers: TriggersConfig,
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
    /// Direct API key (less secure, but convenient)
    pub api_key: Option<String>,
    /// Environment variable name containing the API key (more secure)
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
    /// Whether to include AI-generated prompt enhancements when building effective prompts
    #[serde(default = "default_include_ai_generated")]
    pub include_ai_generated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggersConfig {
    #[serde(default = "default_enable_builtin_triggers")]
    pub enable_builtin_triggers: bool,
    #[serde(default = "default_enable_compactions")]
    pub enable_compactions: bool,
}

impl Default for TriggersConfig {
    fn default() -> Self {
        Self {
            enable_builtin_triggers: default_enable_builtin_triggers(),
            enable_compactions: default_enable_compactions(),
        }
    }
}

const fn default_enable_builtin_triggers() -> bool {
    true
}

const fn default_enable_compactions() -> bool {
    true
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
                api_key: None,
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
            include_ai_generated: default_include_ai_generated(),
        }
    }
}

const fn default_include_ai_generated() -> bool {
    true
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
            info!(
                "{} Loaded configuration from: {}",
                nerd_font::categories::Fa::Check,
                path.display()
            );
            Ok(config)
        } else {
            warn!(
                "{} Configuration file not found: {}",
                nerd_font::categories::Fa::ExclamationCircle,
                path.display()
            );
            warn!("Using default configuration.");
            info!("To customize settings, create a config file:");
            info!("  mkdir -p ~/.config/ritsu");
            info!("  ritsu-server --write-example-config");
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
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let example = format!(
            r#"[llm]
default_backend = "ollama"
disable_streaming = false
disable_tools = false

# Ollama (local)
[[llm.backends]]
name = "ollama"
endpoint = "http://localhost:11434"
model = "llama3.2"

# Groq (fast cloud inference)
# Option 1: API key from environment variable (more secure)
# [[llm.backends]]
# name = "groq"
# endpoint = "https://api.groq.com/openai/v1"
# model = "llama-3.3-70b-versatile"
# api_key_env = "GROQ_API_KEY"

# Option 2: API key directly in config (less secure, but convenient)
# [[llm.backends]]
# name = "groq"
# endpoint = "https://api.groq.com/openai/v1"
# model = "llama-3.3-70b-versatile"
# api_key = "gsk_your_groq_api_key_here"

# OpenAI
# [[llm.backends]]
# name = "openai"
# endpoint = "https://api.openai.com/v1"
# model = "gpt-4"
# api_key_env = "OPENAI_API_KEY"  # or use: api_key = "sk-..."

# Anthropic
# [[llm.backends]]
# name = "anthropic"
# endpoint = "https://api.anthropic.com"
# model = "claude-3-5-sonnet-20241022"
# api_key_env = "ANTHROPIC_API_KEY"  # or use: api_key = "sk-ant-..."

[server]
socket_path = "{socket_path}"
database_path = "{database_path}"
client_binary_path = "{client_binary_path}"

[memory]
daily_rotation_days = {rotation_days}
include_ai_generated = true

[triggers]
# Whether to auto-create built-in triggers (daily compaction, weekly pattern, monthly reflection)
enable_builtin_triggers = true
# Whether to enable automatic compaction-related built-in triggers
enable_compactions = true

[network]
# Allowed hosts for the 'get' tool. Only requests to these hosts will be permitted.
allowed_http_hosts = ["api.ipify.org", "httpbin.org", "jsonplaceholder.typicode.com", "example.com", "api.github.com", "api.openweathermap.org", "api.weatherapi.com", "api.weather.gov", "ipinfo.io", "ip-api.com"]

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

        info!(
            "{} Created example config: {}",
            nerd_font::categories::Fa::Check,
            path.display()
        );
        Ok(())
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// List of allowed hosts for the 'get' tool (e.g., ["api.ipify.org", "example.com"]).
    pub allowed_http_hosts: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            allowed_http_hosts: vec![
                "api.ipify.org".to_string(),
                "httpbin.org".to_string(),
                "jsonplaceholder.typicode.com".to_string(),
                "example.com".to_string(),
                "api.github.com".to_string(),
                "api.openweathermap.org".to_string(),
                "api.weatherapi.com".to_string(),
                "api.weather.gov".to_string(),
                "ipinfo.io".to_string(),
                "ip-api.com".to_string(),
            ],
        }
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
