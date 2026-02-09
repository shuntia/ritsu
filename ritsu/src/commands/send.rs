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
    let session_id = get_or_create_session_id(new_session)?;
    
    // Connect directly to collect streaming response
    let mut stream = tokio::net::UnixStream::connect(&socket_path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound || 
           e.kind() == std::io::ErrorKind::ConnectionRefused {
            tracing::error!("✗ Client daemon not running. Start it with: ritsu start");
            anyhow::anyhow!("Client daemon not running")
        } else {
            tracing::error!("Failed to connect: {}", e);
            anyhow::anyhow!("Failed to connect: {}", e)
        }
    })?;
    
    // Send request
    let request = ClientRequest::SendMessage {
        content: message.to_string(),
        session_id: Some(session_id),
    };
    let request_data = postcard::to_allocvec(&request)?;
    let request_len = (request_data.len() as u32).to_be_bytes();
    
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.write_all(&request_len).await?;
    stream.write_all(&request_data).await?;
    stream.flush().await?;
    
    // Collect streaming response
    let mut full_response = String::new();
    loop {
        // Read message length
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        
        // Read message data
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).await?;
        
        // Try to deserialize as ServerPush (streaming chunks)
        if let Ok(push) = postcard::from_bytes::<ritsu_common::protocol::ServerPush>(&data) {
            if let ritsu_common::protocol::ServerPush::MessageChunk { content, is_final } = push {
                print!("{}", content);
                use std::io::Write;
                let _ = std::io::stdout().flush();
                full_response.push_str(&content);
                
                if is_final {
                    println!(); // Final newline
                    break;
                }
            }
        }
        // Try to deserialize as ServerResponse (final response)
        else if let Ok(response) = postcard::from_bytes::<ritsu_common::protocol::ServerResponse>(&data) {
            match response {
                ritsu_common::protocol::ServerResponse::Ok => {
                    if full_response.is_empty() {
                        println!("✓ Message sent");
                    }
                    return Ok(());
                }
                ritsu_common::protocol::ServerResponse::Error { message } => {
                    if full_response.is_empty() {
                        tracing::error!("✗ Error: {}", message);
                    }
                    anyhow::bail!(message)
                }
                ritsu_common::protocol::ServerResponse::Message { content } => {
                    println!("🤖 {}", content);
                    return Ok(());
                }
                _ => {
                    tracing::error!("✗ Unexpected response");
                    anyhow::bail!("Unexpected response")
                }
            }
        } else {
            tracing::error!("✗ Failed to parse response");
            anyhow::bail!("Failed to parse response")
        }
    }
    
    Ok(())
}
