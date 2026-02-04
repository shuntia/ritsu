//! Ritsu Server - Self-triggering AI agent daemon

use anyhow::Result;
use tracing::info;

mod config;
mod database;
mod ipc;
mod llm;
mod memory;
mod state;
mod tasks;
mod tools;
mod trigger;

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
    let db = database::Database::new(&config.server.database_path)?;
    info!("Database initialized at: {}", config.server.database_path);

    // Initialize memory manager
    let memory = std::sync::Arc::new(memory::MemoryManager::new(db.connection.clone()));
    info!("Memory manager initialized");

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

    // Initialize tool registry (needs memory, task_manager, trigger_registry, database, server_state)
    let tool_registry = std::sync::Arc::new(
        tools::ToolRegistry::new().with_database(db.connection.clone())
    );
    tools::register_all_tools(&tool_registry, memory.clone(), task_manager.clone(), trigger_registry.clone(), server_state.clone()).await;
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

    // Keep server running
    tokio::signal::ctrl_c().await?;
    info!("Shutting down ritsu-server");

    Ok(())
}
