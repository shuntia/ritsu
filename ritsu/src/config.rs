//! Client configuration management

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(default)]
    pub sockets: SocketConfig,
    
    #[serde(default)]
    pub timeouts: TimeoutConfig,
    
    #[serde(default)]
    pub gui: GuiConfig,
    
    #[serde(default)]
    pub paths: PathConfig,
    
    #[serde(default)]
    pub retry: RetryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketConfig {
    #[serde(default = "default_client_socket")]
    pub client_socket: String,
    
    #[serde(default = "default_server_socket")]
    pub server_socket: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutConfig {
    /// Connection timeout in seconds
    #[serde(default = "default_connect_timeout")]
    pub connect_seconds: u64,
    
    /// Request timeout in seconds (for non-streaming requests)
    #[serde(default = "default_request_timeout")]
    pub request_seconds: u64,
    
    /// Streaming timeout in seconds (for initial response)
    #[serde(default = "default_streaming_timeout")]
    pub streaming_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiConfig {
    /// Window width in pixels
    #[serde(default = "default_window_width")]
    pub window_width: u32,
    
    /// Window height in pixels
    #[serde(default = "default_window_height")]
    pub window_height: u32,
    
    /// Font size
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    
    /// Enable desktop notifications
    #[serde(default = "default_notifications_enabled")]
    pub notifications_enabled: bool,
    
    /// Max messages to display in chat
    #[serde(default = "default_max_messages")]
    pub max_messages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConfig {
    /// Session file path (stores last session ID)
    #[serde(default = "default_session_file")]
    pub session_file: PathBuf,
    
    /// Chat history cache directory
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Number of connection retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    
    /// Retry delay in milliseconds
    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: u64,
    
    /// Enable exponential backoff for retries
    #[serde(default = "default_exponential_backoff")]
    pub exponential_backoff: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            sockets: SocketConfig::default(),
            timeouts: TimeoutConfig::default(),
            gui: GuiConfig::default(),
            paths: PathConfig::default(),
            retry: RetryConfig::default(),
        }
    }
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            client_socket: default_client_socket(),
            server_socket: default_server_socket(),
        }
    }
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            connect_seconds: default_connect_timeout(),
            request_seconds: default_request_timeout(),
            streaming_seconds: default_streaming_timeout(),
        }
    }
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            window_width: default_window_width(),
            window_height: default_window_height(),
            font_size: default_font_size(),
            notifications_enabled: default_notifications_enabled(),
            max_messages: default_max_messages(),
        }
    }
}

impl Default for PathConfig {
    fn default() -> Self {
        Self {
            session_file: default_session_file(),
            cache_dir: default_cache_dir(),
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            retry_delay_ms: default_retry_delay_ms(),
            exponential_backoff: default_exponential_backoff(),
        }
    }
}

// Default value functions
fn default_client_socket() -> String {
    std::env::var("RITSU_CLIENT_SOCKET")
        .unwrap_or_else(|_| "/tmp/ritsu-client.sock".to_string())
}

fn default_server_socket() -> String {
    std::env::var("RITSU_SERVER_SOCKET")
        .unwrap_or_else(|_| "/tmp/ritsu.sock".to_string())
}

fn default_connect_timeout() -> u64 {
    std::env::var("RITSU_CONNECT_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
}

fn default_request_timeout() -> u64 {
    std::env::var("RITSU_REQUEST_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
}

fn default_streaming_timeout() -> u64 {
    std::env::var("RITSU_STREAMING_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120)
}

fn default_window_width() -> u32 {
    800
}

fn default_window_height() -> u32 {
    600
}

fn default_font_size() -> f32 {
    14.0
}

fn default_notifications_enabled() -> bool {
    true
}

fn default_max_messages() -> usize {
    500
}

fn default_session_file() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ritsu")
        .join("session.txt")
}

fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ritsu")
}

fn default_max_retries() -> u32 {
    3
}

fn default_retry_delay_ms() -> u64 {
    1000
}

fn default_exponential_backoff() -> bool {
    true
}

impl ClientConfig {
    /// Load configuration from file or use defaults
    pub fn load() -> Result<Self> {
        let config_path = Self::config_file_path();
        
        if config_path.exists() {
            Self::load_from_path(&config_path)
        } else {
            Ok(Self::default())
        }
    }
    
    /// Load configuration from specific path
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
            .join("client.toml")
    }
    
    /// Get client socket path (respects environment variable)
    #[must_use]
    pub fn client_socket_path(&self) -> String {
        std::env::var("RITSU_CLIENT_SOCKET")
            .unwrap_or_else(|_| self.sockets.client_socket.clone())
    }
    
    /// Get server socket path (respects environment variable)
    #[must_use]
    pub fn server_socket_path(&self) -> String {
        std::env::var("RITSU_SERVER_SOCKET")
            .unwrap_or_else(|_| self.sockets.server_socket.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = ClientConfig::default();
        assert_eq!(config.sockets.client_socket, "/tmp/ritsu-client.sock");
        assert_eq!(config.sockets.server_socket, "/tmp/ritsu.sock");
        assert_eq!(config.timeouts.connect_seconds, 5);
        assert_eq!(config.timeouts.request_seconds, 30);
        assert_eq!(config.gui.window_width, 800);
        assert_eq!(config.retry.max_retries, 3);
    }
    
    #[test]
    fn test_env_override() {
        std::env::set_var("RITSU_CLIENT_SOCKET", "/custom/client.sock");
        std::env::set_var("RITSU_SERVER_SOCKET", "/custom/server.sock");
        std::env::set_var("RITSU_CONNECT_TIMEOUT", "10");
        
        let config = ClientConfig::default();
        assert_eq!(config.client_socket_path(), "/custom/client.sock");
        assert_eq!(config.server_socket_path(), "/custom/server.sock");
        assert_eq!(config.timeouts.connect_seconds, 10);
        
        std::env::remove_var("RITSU_CLIENT_SOCKET");
        std::env::remove_var("RITSU_SERVER_SOCKET");
        std::env::remove_var("RITSU_CONNECT_TIMEOUT");
    }
}
