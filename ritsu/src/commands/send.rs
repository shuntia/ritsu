//! Send message command

use anyhow::Result;
use ritsu_common::protocol::ClientRequest;
use std::path::PathBuf;

fn get_session_file() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ritsu")
        .join("current_session")
}

fn get_or_create_session_id(force_new: bool) -> Result<String> {
    let session_file = get_session_file();
    
    // If forcing new session, delete old one
    if force_new && session_file.exists() {
        std::fs::remove_file(&session_file).ok();
    }
    
    // Try to read existing session ID
    if session_file.exists() {
        if let Ok(session_id) = std::fs::read_to_string(&session_file) {
            let trimmed = session_id.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }
    
    // Create new session ID
    let session_id = format!("cli_session_{}", chrono::Utc::now().timestamp());
    
    // Ensure directory exists
    if let Some(parent) = session_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    
    // Save session ID
    std::fs::write(&session_file, &session_id)?;
    
    Ok(session_id)
}

pub async fn send_message(message: &str, new_session: bool) -> Result<()> {
    // Connect to client daemon which will proxy to server
    let socket_path = super::get_client_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    let session_id = get_or_create_session_id(new_session)?;
    
    let response = match client
        .send_request(ClientRequest::SendMessage {
            content: message.to_string(),
            session_id: Some(session_id),
        })
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            if e.to_string().contains("No such file or directory") || 
               e.to_string().contains("Connection refused") {
                eprintln!("✗ Client daemon not running. Start it with: ritsu start");
                anyhow::bail!("Client daemon not running")
            } else {
                return Err(e);
            }
        }
    };

    match response {
        ritsu_common::protocol::ServerResponse::Message { content } => {
            println!("🤖 {}", content);
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Ok => {
            println!("✓ Message sent");
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            eprintln!("✗ Error: {}", message);
            anyhow::bail!(message)
        }
        _ => {
            eprintln!("✗ Unexpected response");
            anyhow::bail!("Unexpected response")
        }
    }
}
