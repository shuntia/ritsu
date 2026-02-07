//! Command modules

#![allow(clippy::unnecessary_wraps)]

pub mod daemon;
pub mod client_daemon;
pub mod send;
pub mod trigger;
pub mod task;
pub mod prompt;
pub mod export;
pub mod memory;

use anyhow::Result;

/// Get client socket path from config
pub fn get_client_socket() -> Result<String> {
    let config = crate::config::ClientConfig::load()?;
    Ok(config.client_socket_path())
}

/// Get server socket path from config
pub fn get_server_socket() -> Result<String> {
    let config = crate::config::ClientConfig::load()?;
    Ok(config.server_socket_path())
}
