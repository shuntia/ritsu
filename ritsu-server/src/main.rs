//! Ritsu Server - Self-triggering AI agent daemon

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
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

    // Load configuration first to get model name for Ollama warmup
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

    // Auto-start Ollama if installed but not running (with model name from config)
    let model_name = config.llm.backends.first()
        .map_or("llama3.2:3b", |b| b.model.as_str()); // Fallback only if no backends configured
    start_ollama_if_needed(model_name).await;

    // Initialize database
    let db = database::Database::new(&config.server.database_path)?;
    info!("Database initialized at: {}", config.server.database_path);

    // Initialize memory manager
    let memory = std::sync::Arc::new(memory::MemoryManager::new(db.connection.clone(), config.server.database_path.clone()));
    info!("Memory manager initialized");

    // Initialize conversation manager
    let conversation_manager = std::sync::Arc::new(conversations::ConversationManager::new(db.connection.clone()));
    info!("Conversation manager initialized");

    // Initialize preferences manager
    let preferences_manager = std::sync::Arc::new(preferences::PreferencesManager::new(db.connection.clone()));
    info!("Preferences manager initialized");

    // Set up graceful shutdown signal
    let shutdown_flag = std::sync::Arc::new(tokio::sync::Notify::new());

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
    let shutdown_flag_clone = shutdown_flag.clone();
    let trigger_loop_handle = tokio::spawn(async move {
        tokio::select! {
            result = trigger::run_trigger_loop(trigger_registry_clone, memory_clone, task_manager_clone, llm_client_clone, server_state_clone) => {
                if let Err(e) = result {
                    tracing::error!("Trigger loop error: {}", e);
                }
            }
            () = shutdown_flag_clone.notified() => {
                info!("Trigger loop shutting down...");
            }
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
        Arc::new(config.clone()),
    );
    let shutdown_flag_clone = shutdown_flag.clone();
    let ipc_server_handle = tokio::spawn(async move {
        tokio::select! {
            result = ipc_server.run() => {
                if let Err(e) = result {
                    tracing::error!("IPC server error: {}", e);
                }
            }
            () = shutdown_flag_clone.notified() => {
                info!("IPC server shutting down...");
            }
        }
    });

    info!("ritsu-server started successfully");

    // Set up signal handlers
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
    info!("Shutdown signal received, waiting for tasks to complete...");
    
    // Wait for background tasks to finish with timeout
    let shutdown_timeout = std::time::Duration::from_secs(5);
    
    let _ = tokio::time::timeout(shutdown_timeout, async {
        let _ = tokio::join!(trigger_loop_handle, ipc_server_handle);
    }).await;
    
    info!("Shutting down ritsu... Good night!");

    Ok(())
}

/// Check if Ollama is installed and start it if not already running
async fn start_ollama_if_needed(model_name: &str) {
    // Check if ollama is installed (spawn_blocking for sync Command)
    let ollama_installed = tokio::task::spawn_blocking(|| {
        std::process::Command::new("which")
            .arg("ollama")
            .output()
            .ok()
            .is_some_and(|o| o.status.success())
    }).await;
    
    if !ollama_installed.unwrap_or(false) {
        tracing::debug!("Ollama not found in PATH");
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
    
    // Open log file using spawn_blocking
    let log_file_result = tokio::task::spawn_blocking(|| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/ritsu-ollama.log")
    }).await;
    
    let log_file = match log_file_result {
        Ok(Ok(file)) => file,
        Ok(Err(e)) => {
            tracing::warn!("Failed to open /tmp/ritsu-ollama.log: {}", e);
            return;
        }
        Err(e) => {
            tracing::warn!("Task failed: {}", e);
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
            // Wait for Ollama HTTP server to be ready
            tracing::info!("Waiting for Ollama HTTP server to start...");
            for attempt in 1..=15 {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                
                let health_check = tokio::process::Command::new("curl")
                    .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "http://localhost:11434/"])
                    .output()
                    .await;
                
                if let Ok(output) = health_check {
                    let status_code = String::from_utf8_lossy(&output.stdout);
                    if status_code == "200" {
                        tracing::info!("Ollama HTTP server ready after {} seconds", attempt * 2);
                        break;
                    }
                }
                
                if attempt < 15 {
                    tracing::debug!("Ollama not ready yet, retrying... (attempt {}/15)", attempt);
                } else {
                    tracing::warn!("Ollama failed to become ready after 30 seconds");
                    return;
                }
            }
            
            // Now do a warmup request to ensure the model is actually loaded
            tracing::info!("Warming up Ollama model '{model_name}' (this may take 5-10 seconds)...");
            let warmup_body = format!(
                r#"{{"model":"{model_name}","messages":[{{"role":"user","content":"hi"}}],"stream":false}}"#
            );
            let warmup = tokio::time::timeout(
                tokio::time::Duration::from_secs(60),
                tokio::process::Command::new("curl")
                    .args([
                        "-s",
                        "-X", "POST",
                        "http://localhost:11434/api/chat",
                        "-d", &warmup_body,
                        "-H", "Content-Type: application/json"
                    ])
                    .output()
            ).await;
            
            match warmup {
                Ok(Ok(output)) => {
                    if output.status.success() {
                        tracing::info!("Ollama model warmed up successfully");
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!("Ollama warmup request failed: {}", stderr);
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!("Failed to execute Ollama warmup request: {}", e);
                }
                Err(_) => {
                    tracing::warn!("Ollama warmup request timed out after 60 seconds");
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to start Ollama server: {}", e);
        }
    }
}
