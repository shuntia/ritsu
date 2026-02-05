//! Client daemon for handling notifications and GUI launches

use anyhow::{Context, Result};
use ritsu_common::protocol::{ServerPush, NotificationUrgency};
use std::process::Command;
use tracing::{info, error};

pub async fn run() -> Result<()> {
    info!("Starting Ritsu client daemon...");
    
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    
    info!("Connecting to Ritsu server...");
    info!("Client daemon ready - listening for notifications and chat requests");
    
    // Subscribe to server pushes and handle them in a loop
    loop {
        match client.subscribe_and_wait().await {
            Ok(push) => {
                if let Err(e) = handle_push(push).await {
                    error!("Failed to handle push: {}", e);
                }
            }
            Err(e) => {
                error!("Error receiving push: {}", e);
                // Wait before retrying
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                info!("Reconnecting to server...");
            }
        }
    }
}

async fn handle_push(push: ServerPush) -> Result<()> {
    match push {
        ServerPush::Notification { title, message, urgency } => {
            handle_notification(&title, &message, urgency).await
        }
        ServerPush::OpenChat { message, urgency } => {
            handle_open_chat(message.as_deref(), urgency).await
        }
        ServerPush::ResponseTimeout { context } => {
            // Response timeout is handled by server, client daemon just logs it
            info!("Server timeout context: {}", context);
            Ok(())
        }
    }
}

async fn handle_notification(title: &str, message: &str, urgency: NotificationUrgency) -> Result<()> {
    info!("Showing notification: {} - {}", title, message);
    
    #[cfg(target_os = "linux")]
    {
        use notify_rust::{Notification, Urgency};
        
        let urgency_level = match urgency {
            NotificationUrgency::Low => Urgency::Low,
            NotificationUrgency::Normal => Urgency::Normal,
            NotificationUrgency::Urgent => Urgency::Critical,
        };
        
        Notification::new()
            .summary(title)
            .body(message)
            .icon("dialog-information")
            .appname("Ritsu")
            .urgency(urgency_level)
            .timeout(0) // No timeout
            .show()
            .context("Failed to show notification")?;
    }
    
    #[cfg(not(target_os = "linux"))]
    {
        info!("Notification display not implemented for this platform");
    }
    
    Ok(())
}

async fn handle_open_chat(message: Option<&str>, urgency: NotificationUrgency) -> Result<()> {
    info!("Opening chat window with urgency: {:?}", urgency);
    
    if let Some(msg) = message {
        info!("With initial message: {}", msg);
    }
    
    // Launch ritsu chat GUI as detached process
    Command::new("ritsu")
        .arg("chat")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn ritsu chat")?;
    
    info!("✓ Chat window launched");
    
    Ok(())
}
