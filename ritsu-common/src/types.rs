//! Shared types and utilities

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Common error type for ritsu operations
#[derive(Debug, Error)]
pub enum RitsuError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(String),
    
    #[error("Database error: {0}")]
    Database(String),
    
    #[error("Configuration error: {0}")]
    Config(String),
    
    #[error("IPC error: {0}")]
    Ipc(String),
    
    #[error("Server not running")]
    ServerNotRunning,
    
    #[error("Timeout: {0}")]
    Timeout(String),
}

pub type Result<T> = std::result::Result<T, RitsuError>;

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolResult {
    #[must_use]
    pub const fn success(output: String) -> Self {
        Self {
            success: true,
            output,
            error: None,
        }
    }
    
    #[must_use]
    pub const fn error(message: String) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(message),
        }
    }
}
