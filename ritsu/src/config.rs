//! Client configuration management

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
#[allow(clippy::struct_field_names)]
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
    std::env::var("RITSU_CLIENT_SOCKET").unwrap_or_else(|_| "/tmp/ritsu-client.sock".to_string())
}

fn default_server_socket() -> String {
    std::env::var("RITSU_SERVER_SOCKET").unwrap_or_else(|_| "/tmp/ritsu.sock".to_string())
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
        Self::load_from_path(&config_path)
    }

    /// Load configuration from specific path
    pub fn load_from_path(path: &PathBuf) -> Result<Self> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file: {}", path.display()))?;
            let config: Self = toml::from_str(&contents)
                .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
            info!("Loaded client configuration from: {}", path.display());
            Ok(config)
        } else {
            warn!("Client configuration file not found: {}", path.display());
            info!("Using default client configuration. To customize, create '{}' or run 'ritsu init --config --prompts'", path.display());
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

    /// Write an example configuration file (client only)
    pub fn write_example_config(path: &PathBuf) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let example_config = Self::default();
        let toml_string = toml::to_string_pretty(&example_config)
            .context("Failed to serialize example config")?;

        std::fs::write(path, toml_string)
            .with_context(|| format!("Failed to write example config to: {}", path.display()))?;

        info!("Created example client config: {}", path.display());
        Ok(())
    }

    /// Write example prompts to ~/.config/ritsu/prompts and per-trigger prompts
    pub fn write_example_prompts() -> Result<()> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        let prompts_dir = home.join(".config/ritsu/prompts");

        // Create prompts directory
        std::fs::create_dir_all(&prompts_dir)?;

        // Base system prompt
        let base_path = prompts_dir.join("system_base.md");
        let base_content = r#"# Ritsu System Prompt

You are Ritsu, an autonomous AI assistant with the following capabilities:

## Core Identity
- Self-triggering agent that can schedule and execute tasks independently
- Maintain long-term memory through conversation summaries
- Learn user preferences over time
- Proactive in suggesting improvements and automations

## Interaction Style
- Concise and direct in responses
- Ask clarifying questions when needed
- Provide context for your actions
- Be transparent about limitations

## Key Responsibilities
1. Manage scheduled tasks and triggers
2. Maintain organized memory (notes, summaries)
3. Execute tools to help the user
4. Learn from conversations to improve service
5. Suggest optimizations and automations

## Available Tools
You have access to various tools for:
- Creating notes and managing memory
- Scheduling triggers for future actions
- Sending notifications to the user
- Managing tasks with priorities and due dates
- Analyzing patterns in user behavior

Use these tools proactively to assist the user effectively.
"#;
        std::fs::write(&base_path, base_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, base_path.display());

        // Chat context prompt
        let chat_path = prompts_dir.join("chat.md");
        let chat_content = r#"# Chat Context

You are in an interactive chat session with the user.

## Behavior
- Respond naturally and conversationally
- Keep responses focused and relevant
- Use tools when appropriate (notifications, notes, tasks)
- Reference relevant memory when helpful

## Response Style
- Be helpful and attentive
- Clarify ambiguous requests
- Provide actionable suggestions
"#;
        std::fs::write(&chat_path, chat_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, chat_path.display());

        // Background prompt
        let background_path = prompts_dir.join("background.md");
        let background_content = r#"# Background Task Context

You are executing a scheduled background task.

## Behavior
- Complete the task efficiently
- Use tools to accomplish goals (notifications, notes)
- Log important findings
- Return concise summary of actions taken

## Decision Making
- Only notify user for important events
- Create notes for information worth remembering
- Suggest new triggers if patterns emerge
"#;
        std::fs::write(&background_path, background_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, background_path.display());

        // Create background subdirectory
        let background_dir = prompts_dir.join("background");
        std::fs::create_dir_all(&background_dir)?;

        // Compact prompt
        let compact_path = background_dir.join("compact.md");
        let compact_content = r#"# Memory Compaction Context

You are compacting conversation history into a summary.

## Task
- Extract key information from conversations
- Identify important decisions and preferences
- Tag content appropriately
- Be concise but preserve context

## Format
Generate a well-structured summary with:
- Key topics discussed
- Important decisions made
- Action items identified
- User preferences learned
"#;
        std::fs::write(&compact_path, compact_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, compact_path.display());

        // Pattern analysis prompt
        let pattern_path = background_dir.join("pattern.md");
        let pattern_content = r#"# Pattern Recognition Context

You are analyzing user behavior patterns.

## Task
- Identify recurring themes and preferences
- Detect optimal timing for tasks
- Recognize automation opportunities
- Note communication preferences

## Output
Produce insights about:
- Common workflows
- Preferred interaction times
- Tool usage patterns
- Suggested optimizations
"#;
        std::fs::write(&pattern_path, pattern_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, pattern_path.display());

        // Briefing prompt
        let briefing_path = background_dir.join("briefing.md");
        let briefing_content = r#"# Morning Briefing Context

You are preparing a daily briefing for the user.

## Task
- Summarize yesterday's activities
- List pending tasks by priority
- Note upcoming events/deadlines
- Provide relevant reminders

## Style
- Brief and scannable
- Prioritize actionable items
- Highlight urgent matters
- Be encouraging and positive
"#;
        std::fs::write(&briefing_path, briefing_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, briefing_path.display());

        // Create triggers directory and per-trigger prompts
        let triggers_dir = prompts_dir.join("triggers");
        std::fs::create_dir_all(&triggers_dir)?;

        let daily_comp_path = triggers_dir.join("daily_compaction.md");
        let daily_comp_content = r#"# daily_compaction trigger

This trigger runs daily to compact conversations into summaries. Use compact prompt context and include relevant task summary. Ensure output is concise and focuses on key points and action items.
"#;
        std::fs::write(&daily_comp_path, daily_comp_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, daily_comp_path.display());

        let weekly_pattern_path = triggers_dir.join("weekly_pattern.md");
        let weekly_pattern_content = r#"# weekly_pattern trigger

This trigger runs weekly to analyze user behavior and detect patterns. Focus on recurring themes, tool usage, and timing recommendations. Produce actionable suggestions.
"#;
        std::fs::write(&weekly_pattern_path, weekly_pattern_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, weekly_pattern_path.display());

        let monthly_reflect_path = triggers_dir.join("monthly_reflection.md");
        let monthly_reflect_content = r#"# monthly_reflection trigger

This trigger runs monthly for self-reflection and long-term summary. Aggregate monthly progress, highlight trends, and suggest strategic improvements.
"#;
        std::fs::write(&monthly_reflect_path, monthly_reflect_content)?;
        println!("{} Created: {}", nerd_font::categories::Fa::Check, monthly_reflect_path.display());

        println!("\n");
        println!("All example prompts and trigger prompts created successfully!");
        println!("Edit these files to customize Ritsu's behavior.");

        Ok(())
    }

    /// Get client socket path (respects environment variable)
    #[must_use]
    pub fn client_socket_path(&self) -> String {
        std::env::var("RITSU_CLIENT_SOCKET").unwrap_or_else(|_| self.sockets.client_socket.clone())
    }

    /// Get server socket path (respects environment variable)
    #[must_use]
    pub fn server_socket_path(&self) -> String {
        std::env::var("RITSU_SERVER_SOCKET").unwrap_or_else(|_| self.sockets.server_socket.clone())
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
