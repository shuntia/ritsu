//! System prompt management commands

use anyhow::{Context, Result};
use ritsu_common::protocol::{ClientRequest, ServerResponse};
use crate::PromptCommands;
use std::io::Write;

pub async fn handle(cmd: PromptCommands) -> Result<()> {
    match cmd {
        PromptCommands::Show => show_prompt().await,
        PromptCommands::Set { path } => set_prompt(&path).await,
        PromptCommands::Edit => edit_prompt().await,
    }
}

async fn show_prompt() -> Result<()> {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    let response = client.send_request(ClientRequest::GetSystemPrompt).await?;
    
    match response {
        ServerResponse::SystemPrompt { content } => {
            println!("{}", content);
            Ok(())
        }
        ServerResponse::Error { message } => {
            anyhow::bail!("Failed to get system prompt: {}", message);
        }
        _ => anyhow::bail!("Unexpected response from server"),
    }
}

async fn set_prompt(path: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .context("Failed to read prompt file")?;
    
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    let response = client.send_request(ClientRequest::SetSystemPrompt { 
        content: content.clone() 
    }).await?;
    
    match response {
        ServerResponse::Success { message } => {
            println!("{}", message);
            Ok(())
        }
        ServerResponse::Error { message } => {
            anyhow::bail!("Failed to set system prompt: {}", message);
        }
        _ => anyhow::bail!("Unexpected response from server"),
    }
}

async fn edit_prompt() -> Result<()> {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    
    // Get current prompt
    let response = client.send_request(ClientRequest::GetSystemPrompt).await?;
    
    let current_content = match response {
        ServerResponse::SystemPrompt { content } => content,
        ServerResponse::Error { message } => {
            anyhow::bail!("Failed to get system prompt: {}", message);
        }
        _ => anyhow::bail!("Unexpected response from server"),
    };
    
    // Write to temp file
    let mut temp_file = tempfile::NamedTempFile::new()?;
    temp_file.write_all(current_content.as_bytes())?;
    let temp_path = temp_file.path().to_owned();
    
    // Open in editor
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
    let status = std::process::Command::new(&editor)
        .arg(&temp_path)
        .status()
        .context("Failed to launch editor")?;
    
    if !status.success() {
        anyhow::bail!("Editor exited with error");
    }
    
    // Read edited content
    let new_content = std::fs::read_to_string(&temp_path)
        .context("Failed to read edited prompt")?;
    
    // Send to server
    let response = client.send_request(ClientRequest::SetSystemPrompt { 
        content: new_content.clone()
    }).await?;
    
    match response {
        ServerResponse::Success { message } => {
            let msg = if new_content == current_content {
                "No changes made".to_string()
            } else {
                message
            };
            println!("{}", msg);
            Ok(())
        }
        ServerResponse::Error { message } => {
            anyhow::bail!("Failed to update system prompt: {}", message);
        }
        _ => anyhow::bail!("Unexpected response from server"),
    }
}
