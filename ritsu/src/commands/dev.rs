//! Developer commands for inspection and debugging

use anyhow::{Context, Result};
use ritsu_common::protocol::{ClientRequest, ServerResponse};
use std::fs;

/// Extract success message from server response
fn extract_message(response: ServerResponse) -> Result<String> {
    match response {
        ServerResponse::Success { message } => Ok(message),
        ServerResponse::Message { content } | ServerResponse::SystemPrompt { content } => Ok(content),
        ServerResponse::Error { message } => anyhow::bail!("Server error: {}", message),
        other => anyhow::bail!("Unexpected response: {:?}", other),
    }
}

pub async fn handle(cmd: crate::DevCommands) -> Result<()> {
    use crate::DevCommands;
    
    match cmd {
        DevCommands::Prompt => show_prompt().await,
        DevCommands::Model => show_model().await,
        DevCommands::Config => show_config().await,
        DevCommands::ClientConfig => show_client_config().await,
        DevCommands::DbStats => show_db_stats().await,
        DevCommands::Ping { count } => ping_server(count).await,
        DevCommands::ClearSession => clear_session().await,
        DevCommands::ClearCache => clear_cache().await,
        DevCommands::ExportDb { output } => export_db(&output).await,
        DevCommands::InspectSession { session_id } => inspect_session(session_id.as_deref()).await,
        DevCommands::ListSessions { limit } => list_sessions(limit).await,
        DevCommands::ToolStats => show_tool_stats().await,
        DevCommands::MemoryStatus => show_memory_status().await,
        DevCommands::ForceCompact { noconfirm } => force_compact(noconfirm).await,
        DevCommands::DbReset { noconfirm } => db_reset(noconfirm).await,
        DevCommands::ReindexDb { noconfirm } => reindex_db(noconfirm).await,
    }
}

async fn show_prompt() -> Result<()> {
    println!("🔍 Fetching effective system prompt...\n");
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::GetSystemPrompt).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to fetch system prompt: {}", e);
            tracing::info!("Make sure the server is running: ritsu server start");
        }
    }
    
    Ok(())
}

async fn show_model() -> Result<()> {
    println!("🔍 Fetching model information...\n");
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::GetModelInfo).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to fetch model info: {}", e);
            tracing::info!("Make sure the server is running: ritsu server start");
        }
    }
    
    Ok(())
}

async fn show_config() -> Result<()> {
    println!("🔍 Server Configuration\n");
    println!("Note: This shows client-side config. For server config, check:");
    println!("  ~/.config/ritsu/config.toml\n");
    
    let config_path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("ritsu")
        .join("config.toml");
    
    if config_path.exists() {
        let contents = fs::read_to_string(&config_path)?;
        println!("📄 Config file: {}\n", config_path.display());
        println!("{}", contents);
    } else {
        println!("⚠️  No server config file found at: {}", config_path.display());
        println!("    Using built-in defaults");
    }
    
    Ok(())
}

async fn show_client_config() -> Result<()> {
    println!("🔍 Client Configuration\n");
    
    let config = crate::config::ClientConfig::load()?;
    
    println!("📍 Sockets:");
    println!("  Client socket: {}", config.client_socket_path());
    println!("  Server socket: {}", config.server_socket_path());
    println!();
    
    println!("⏱️  Timeouts:");
    println!("  Connect: {}s", config.timeouts.connect_seconds);
    println!("  Request: {}s", config.timeouts.request_seconds);
    println!("  Streaming: {}s", config.timeouts.streaming_seconds);
    println!();
    
    println!("🖥️  GUI:");
    println!("  Window: {}x{}", config.gui.window_width, config.gui.window_height);
    println!("  Font size: {}", config.gui.font_size);
    println!("  Notifications: {}", config.gui.notifications_enabled);
    println!("  Max messages: {}", config.gui.max_messages);
    println!();
    
    println!("📁 Paths:");
    println!("  Session file: {}", config.paths.session_file.display());
    println!("  Cache dir: {}", config.paths.cache_dir.display());
    println!();
    
    println!("🔄 Retry:");
    println!("  Max retries: {}", config.retry.max_retries);
    println!("  Retry delay: {}ms", config.retry.retry_delay_ms);
    println!("  Exponential backoff: {}", config.retry.exponential_backoff);
    
    let config_path = crate::config::ClientConfig::config_file_path();
    if config_path.exists() {
        println!("\n📄 Config file: {}", config_path.display());
    } else {
        println!("\n⚠️  No client config file (using defaults)");
        println!("    Create one at: {}", config_path.display());
    }
    
    Ok(())
}

async fn show_db_stats() -> Result<()> {
    println!("🔍 Fetching database statistics...\n");
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::GetDatabaseStats).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to fetch database stats: {}", e);
            tracing::info!("Make sure the server is running: ritsu server start");
        }
    }
    
    Ok(())
}

async fn ping_server(count: u32) -> Result<()> {
    println!("🏓 Pinging server {} time(s)...\n", count);
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    let mut successes = 0;
    let mut total_ms = 0u128;
    
    for i in 1..=count {
        let start = std::time::Instant::now();
        match client.ping().await {
            Ok(true) => {
                let elapsed = start.elapsed().as_millis();
                total_ms += elapsed;
                successes += 1;
                println!("{}. ✓ Pong! ({}ms)", i, elapsed);
            }
            Ok(false) => {
                println!("{}. ❌ Server returned false", i);
            }
            Err(e) => {
                println!("{}. ❌ Failed: {}", i, e);
            }
        }
        
        if i < count {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }
    
    println!("\n📊 Results: {}/{} successful", successes, count);
    if successes > 0 {
        println!("   Average latency: {}ms", total_ms / successes as u128);
    }
    
    Ok(())
}

async fn clear_session() -> Result<()> {
    let config = crate::config::ClientConfig::load()?;
    let session_file = &config.paths.session_file;
    
    if session_file.exists() {
        fs::remove_file(session_file)
            .with_context(|| format!("Failed to remove session file: {}", session_file.display()))?;
        println!("✓ Cleared session file: {}", session_file.display());
    } else {
        println!("ℹ️  No session file to clear");
    }
    
    Ok(())
}

async fn clear_cache() -> Result<()> {
    let config = crate::config::ClientConfig::load()?;
    let cache_dir = &config.paths.cache_dir;
    
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)
            .with_context(|| format!("Failed to remove cache directory: {}", cache_dir.display()))?;
        println!("✓ Cleared cache directory: {}", cache_dir.display());
    } else {
        println!("ℹ️  No cache directory to clear");
    }
    
    Ok(())
}

async fn export_db(output: &str) -> Result<()> {
    println!("🔍 Exporting full database to {}...\n", output);
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::ExportDatabase).await {
        Ok(response) => {
            let content = extract_message(response)?;
            fs::write(output, content)
                .with_context(|| format!("Failed to write to {}", output))?;
            println!("✓ Database exported to: {}", output);
        }
        Err(e) => {
            tracing::error!("❌ Failed to export database: {}", e);
            tracing::info!("Make sure the server is running: ritsu server start");
        }
    }
    
    Ok(())
}

async fn inspect_session(session_id: Option<&str>) -> Result<()> {
    let session = session_id
        .map(String::from)
        .or_else(|| {
            let config = crate::config::ClientConfig::load().ok()?;
            fs::read_to_string(&config.paths.session_file).ok()
        });
    
    let session = session.context("No session ID provided and no current session")?;
    
    println!("🔍 Inspecting session: {}\n", session);
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::InspectSession { session_id: session }).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to inspect session: {}", e);
        }
    }
    
    Ok(())
}

async fn list_sessions(limit: u32) -> Result<()> {
    println!("🔍 Listing recent sessions (limit: {})...\n", limit);
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::ListSessions { limit: Some(limit as usize) }).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to list sessions: {}", e);
        }
    }
    
    Ok(())
}

async fn show_tool_stats() -> Result<()> {
    println!("🔍 Fetching tool usage statistics...\n");
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::GetToolStats).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to fetch tool stats: {}", e);
        }
    }
    
    Ok(())
}

async fn show_memory_status() -> Result<()> {
    println!("🔍 Fetching memory compaction status...\n");
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::GetMemoryStatus).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to fetch memory status: {}", e);
        }
    }
    
    Ok(())
}

async fn force_compact(noconfirm: bool) -> Result<()> {
    if !noconfirm {
        println!("⚠️  WARNING: Force compaction will compress recent conversations!");
        println!("   This is normally done automatically at end of day.");
        println!();
        print!("   Continue? [y/N]: ");
        use std::io::Write;
        std::io::stdout().flush()?;
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }
    
    println!("\n🔄 Forcing memory compaction...\n");
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::ForceCompact).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to force compaction: {}", e);
        }
    }
    
    Ok(())
}

async fn db_reset(noconfirm: bool) -> Result<()> {
    if !noconfirm {
        println!("⚠️  WARNING: This will RESET the entire database and delete ALL data!");
        println!("   This includes conversations, memory, tasks, triggers, preferences, and system prompts.");
        println!();
        print!("   Continue? [y/N]: ");
        use std::io::Write;
        std::io::stdout().flush()?;
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }
    
    println!("\n🔄 Resetting database...\n");
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::ResetDatabase { confirm: true }).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to reset database: {}", e);
            tracing::info!("Make sure the server is running: ritsu server start");
        }
    }
    
    Ok(())
}

async fn reindex_db(noconfirm: bool) -> Result<()> {
    if !noconfirm {
        println!("⚠️  WARNING: Reindexing will rebuild all database indexes!");
        println!("   This may take a while for large databases.");
        println!();
        print!("   Continue? [y/N]: ");
        use std::io::Write;
        std::io::stdout().flush()?;
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }
    
    println!("\n🔄 Reindexing database...\n");
    
    let socket_path = super::get_server_socket()?;
    let client = crate::ipc::IpcClient::new(socket_path);
    
    match client.send_request(ClientRequest::ReindexDatabase).await {
        Ok(response) => {
            let msg = extract_message(response)?;
            println!("{}", msg);
        }
        Err(e) => {
            tracing::error!("❌ Failed to reindex database: {}", e);
        }
    }
    
    Ok(())
}
