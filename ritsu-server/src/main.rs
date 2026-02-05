//! Ritsu Server - Self-triggering AI agent daemon

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;

mod config;
mod conversations;
mod database;
mod ipc;
mod llm;
mod memory;
mod preferences;
mod state;
mod tasks;
mod tools;
mod trigger;

#[derive(Parser)]
#[command(name = "ritsu-server")]
#[command(about = "Ritsu server daemon - Self-triggering AI agent", long_about = None)]
#[command(version)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Socket path for IPC (overrides config file)
    #[arg(short, long, value_name = "PATH")]
    socket: Option<String>,

    /// Database path (overrides config file)
    #[arg(short, long, value_name = "PATH")]
    database: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing with human-readable timestamps
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_timer(tracing_subscriber::fmt::time::LocalTime::rfc_3339())
        .init();

    let cli = Cli::parse();

    info!("Starting ritsu-server v{}", env!("CARGO_PKG_VERSION"));

    // Auto-start Ollama if installed but not running
    start_ollama_if_needed().await;

    // Load configuration
    let mut config = if let Some(config_path) = cli.config {
        info!("Loading configuration from: {}", config_path.display());
        config::Config::load_from_path(&config_path)?
    } else {
        let default_path = config::Config::config_file_path();
        info!("Loading configuration from default path: {}", default_path.display());
        config::Config::load()?
    };

    // Apply CLI overrides
    if let Some(socket_path) = cli.socket {
        info!("Overriding socket path to: {}", socket_path);
        config.server.socket_path = socket_path;
    }
    if let Some(database_path) = cli.database {
        info!("Overriding database path to: {}", database_path);
        config.server.database_path = database_path;
    }

    // Initialize database
    let db = database::Database::new(&config.server.database_path)?;
    info!("Database initialized at: {}", config.server.database_path);

    // Initialize memory manager
    let memory = std::sync::Arc::new(memory::MemoryManager::new(db.connection.clone()));
    info!("Memory manager initialized");

    // Initialize conversation manager
    let conversation_manager = std::sync::Arc::new(conversations::ConversationManager::new(db.connection.clone()));
    info!("Conversation manager initialized");

    // Initialize preferences manager
    let preferences_manager = std::sync::Arc::new(preferences::PreferencesManager::new(db.connection.clone()));
    info!("Preferences manager initialized");

    // Initialize task manager
    let task_manager = std::sync::Arc::new(tasks::TaskManager::new(db.connection.clone()));
    info!("Task manager initialized");

    // Initialize trigger registry
    let trigger_registry = std::sync::Arc::new(trigger::TriggerRegistry::new(config.server.database_path.clone()));
    trigger_registry.register_builtin_triggers().await?;
    info!("Trigger registry initialized with {} triggers", trigger_registry.get_all_triggers().await.len());

    // Initialize server state
    let server_state = std::sync::Arc::new(state::ServerState::new());
    info!("Server state initialized");

    // Initialize tool registry (needs memory, task_manager, trigger_registry, database, server_state, preferences)
    let tool_registry = std::sync::Arc::new(
        tools::ToolRegistry::new().with_database(db.connection.clone())
    );
    tools::register_all_tools(&tool_registry, memory.clone(), task_manager.clone(), trigger_registry.clone(), server_state.clone(), preferences_manager.clone()).await;
    info!("Tool registry initialized with usage tracking");

    // Initialize LLM client (needs tool registry for tool calling)
    let llm_client = std::sync::Arc::new(llm::LlmClient::new(&config.llm, &config.timeouts, tool_registry.clone())?);
    info!("LLM client initialized with {} backend(s)", config.llm.backends.len());

    // Start trigger loop in background
    let trigger_registry_clone = trigger_registry.clone();
    let memory_clone = memory.clone();
    let task_manager_clone = task_manager.clone();
    let llm_client_clone = llm_client.clone();
    let server_state_clone = server_state.clone();
    tokio::spawn(async move {
        if let Err(e) = trigger::run_trigger_loop(trigger_registry_clone, memory_clone, task_manager_clone, llm_client_clone, server_state_clone).await {
            tracing::error!("Trigger loop error: {}", e);
        }
    });

    // Start IPC server in background
    let ipc_server = ipc::IpcServer::new(
        config.server.socket_path.clone(),
        memory.clone(),
        conversation_manager.clone(),
        task_manager.clone(),
        trigger_registry.clone(),
        llm_client.clone(),
        server_state.clone(),
    );
    tokio::spawn(async move {
        if let Err(e) = ipc_server.run().await {
            tracing::error!("IPC server error: {}", e);
        }
    });

    info!("ritsu-server started successfully");

    // Set up graceful shutdown with signal handling
    let shutdown_flag = std::sync::Arc::new(tokio::sync::Notify::new());
    let shutdown_flag_clone = shutdown_flag.clone();

    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                info!("Received SIGINT! Shutting down gracefully...");
                shutdown_flag_clone.notify_waiters();
            }
            Err(e) => {
                tracing::error!("Failed to listen for SIGINT: {}", e);
            }
        }
    });

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let shutdown_flag_clone = shutdown_flag.clone();
        tokio::spawn(async move {
            // Signal setup failure is fatal - we need graceful shutdown capability
            #[allow(clippy::expect_used)]
            let mut sigterm = signal(SignalKind::terminate()).expect("Failed to setup SIGTERM handler");
            sigterm.recv().await;
            info!("Received SIGTERM! Shutting down gracefully...");
            shutdown_flag_clone.notify_waiters();
        });
    }

    // Wait for shutdown signal
    shutdown_flag.notified().await;
    info!("Shutting down ritsu... Good night!");

    Ok(())
}

/// Check if Ollama is installed and start it if not already running
async fn start_ollama_if_needed() {
    use std::process::Command;
    
    // Check if ollama is installed
    let ollama_check = Command::new("which")
        .arg("ollama")
        .output();
    
    if let Ok(output) = ollama_check {
        if !output.status.success() {
            tracing::debug!("Ollama not found in PATH");
            return;
        }
    } else {
        tracing::debug!("Failed to check for ollama");
        return;
    }
    
    // Check if Ollama is already running by trying to connect
    let health_check = tokio::process::Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "http://localhost:11434/"])
        .output()
        .await;
    
    if let Ok(output) = health_check {
        let status_code = String::from_utf8_lossy(&output.stdout);
        if status_code == "200" {
            tracing::info!("Ollama server already running");
            return;
        }
    }
    
    // Start Ollama in background
    tracing::info!("Starting Ollama server (output: /tmp/ritsu-ollama.log)...");
    
    // Open log file
    let log_file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/ritsu-ollama.log")
    {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!("Failed to open /tmp/ritsu-ollama.log: {}", e);
            return;
        }
    };
    
    let stdout_file = match log_file.try_clone() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Failed to clone log file handle: {}", e);
            return;
        }
    };
    
    let result = tokio::process::Command::new("ollama")
        .arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(stdout_file)
        .stderr(log_file)
        .spawn();
    
    match result {
        Ok(_child) => {
            tracing::info!("Ollama server started in background");
            // Give it a moment to start
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
        Err(e) => {
            tracing::warn!("Failed to start Ollama server: {}", e);
        }
    }
}
