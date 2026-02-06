//! Export conversation history to various formats

use anyhow::{Context, Result};
use ritsu_common::protocol::{ClientRequest, ServerResponse};
use std::fs::File;
use std::io::Write;

pub async fn export_conversation(session_id: Option<&str>, output_path: &str, format: &str) -> Result<()> {
    // If no session specified, get list and use most recent
    let session_to_export = if let Some(sid) = session_id {
        sid.to_string()
    } else {
        // Get sessions list
        let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
        let response = client.send_request(ClientRequest::ListSessions { limit: Some(1) }).await?;
        
        match response {
            ServerResponse::Sessions { sessions } => {
                if sessions.is_empty() {
                    anyhow::bail!("No sessions found. Specify a session ID with --session");
                }
                sessions[0].session_id.clone()
            }
            ServerResponse::Error { message } => {
                anyhow::bail!("Failed to list sessions: {}", message);
            }
            _ => anyhow::bail!("Unexpected response from server"),
        }
    };
    
    // Get conversation history
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    let response = client.send_request(ClientRequest::GetConversationHistory {
        session_id: session_to_export.clone(),
        limit: 10000, // Large number to get all
    }).await?;
    
    let turns = match response {
        ServerResponse::ConversationHistory { turns } => turns,
        ServerResponse::Error { message } => {
            anyhow::bail!("Failed to get conversation history: {}", message);
        }
        _ => anyhow::bail!("Unexpected response from server"),
    };
    
    if turns.is_empty() {
        println!("No conversation history found for session {}", session_to_export);
        return Ok(());
    }
    
    // Export to file
    let mut file = File::create(output_path)
        .context("Failed to create output file")?;
    
    match format {
        "txt" => export_txt(&mut file, &session_to_export, &turns)?,
        "json" => export_json(&mut file, &session_to_export, &turns)?,
        "md" | "markdown" => export_markdown(&mut file, &session_to_export, &turns)?,
        _ => anyhow::bail!("Unsupported format: {}. Use txt, json, or md", format),
    }
    
    println!("✓ Exported {} turns to {}", turns.len(), output_path);
    Ok(())
}

fn export_txt(file: &mut File, session_id: &str, turns: &[ritsu_common::protocol::ConversationTurn]) -> Result<()> {
    writeln!(file, "Conversation Export: {}", session_id)?;
    writeln!(file, "{}", "=".repeat(60))?;
    writeln!(file)?;
    
    for turn in turns {
        let role = if turn.role == "user" { "You" } else { "Ritsu" };
        writeln!(file, "[{}]", role)?;
        writeln!(file, "{}", turn.content)?;
        writeln!(file)?;
    }
    
    Ok(())
}

fn export_json(file: &mut File, session_id: &str, turns: &[ritsu_common::protocol::ConversationTurn]) -> Result<()> {
    let export_data = serde_json::json!({
        "session_id": session_id,
        "turn_count": turns.len(),
        "turns": turns,
    });
    
    serde_json::to_writer_pretty(file, &export_data)?;
    Ok(())
}

fn export_markdown(file: &mut File, session_id: &str, turns: &[ritsu_common::protocol::ConversationTurn]) -> Result<()> {
    writeln!(file, "# Conversation Export")?;
    writeln!(file)?;
    writeln!(file, "**Session ID:** `{}`", session_id)?;
    writeln!(file, "**Turns:** {}", turns.len())?;
    writeln!(file)?;
    writeln!(file, "---")?;
    writeln!(file)?;
    
    for turn in turns {
        if turn.role == "user" {
            writeln!(file, "## You")?;
        } else {
            writeln!(file, "## Ritsu")?;
        }
        writeln!(file)?;
        writeln!(file, "{}", turn.content)?;
        writeln!(file)?;
    }
    
    Ok(())
}
