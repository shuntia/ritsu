//! Daemon management commands

use anyhow::{Context, Result};
use std::process::Command;

pub async fn start() -> Result<()> {
    println!("Starting ritsu-server...");
    
    // Check if already running
    if is_running().await {
        println!("Server is already running");
        return Ok(());
    }

    // Spawn ritsu-server as a detached process
    Command::new("ritsu-server")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn ritsu-server")?;

    // Wait a moment for startup
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    if is_running().await {
        println!("✓ Server started successfully");
        Ok(())
    } else {
        anyhow::bail!("Server failed to start");
    }
}

pub async fn stop() -> Result<()> {
    println!("Stopping ritsu-server...");
    
    if !is_running().await {
        println!("Server is not running");
        return Ok(());
    }

    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    match client.send_request(ritsu_common::protocol::ClientRequest::Shutdown).await {
        Ok(_) => {
            println!("✓ Server stopped successfully");
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to send shutdown: {}", e);
            Err(e)
        }
    }
}

pub async fn status() -> Result<()> {
    if is_running().await {
        println!("✓ Server is running");
    } else {
        println!("✗ Server is not running");
    }
    Ok(())
}

pub async fn restart() -> Result<()> {
    stop().await.context("Failed to stop server")?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    start().await.context("Failed to start server")?;
    Ok(())
}

async fn is_running() -> bool {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    client.ping().await.unwrap_or(false)
}
