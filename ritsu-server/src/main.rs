//! Ritsu Server - Self-triggering AI agent daemon

use anyhow::Result;
use tracing::info;

mod config;
mod database;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Starting ritsu-server v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = config::Config::load()?;
    info!("Configuration loaded from: {:?}", config::Config::config_file_path());

    // Initialize database
    let _db = database::Database::new(&config.server.database_path)?;
    info!("Database initialized at: {}", config.server.database_path);

    info!("ritsu-server started successfully");

    // Keep server running
    tokio::signal::ctrl_c().await?;
    info!("Shutting down ritsu-server");

    Ok(())
}
