//! Command modules

#![allow(clippy::unnecessary_wraps)]

pub mod attach;
pub mod client_daemon;
pub mod daemon;
pub mod dev;
pub mod export;
pub mod memory;
pub mod prompt;
pub mod send;
pub mod task;
pub mod trigger;

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
