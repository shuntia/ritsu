//! Ritsu Server - Self-triggering AI agent daemon

use anyhow::Result;
use clap::{Parser, Subcommand};
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
mod prompt;
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

    /// Write example configuration file and exit (legacy)
    #[arg(long)]
    write_example_config: bool,

    /// Write example system prompts and exit (legacy)
    #[arg(long)]
    write_example_prompts: bool,

    /// Optional subcommand mode (e.g., init)
    #[command(subcommand)]
    command: Option<ServerCommand>,
}

#[derive(Subcommand)]
enum ServerCommand {
    /// Initialize example files (config and prompts)
    Init {
        /// Write example configuration file
        #[arg(short = 'c', long)]
        config: bool,

        /// Write example prompts
        #[arg(short = 'p', long)]
        prompts: bool,
    },
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

    // Handle subcommands (e.g. init)
    if let Some(ServerCommand::Init { config: init_cfg, prompts: init_prompts }) = &cli.command {
        let config_path = config::Config::config_file_path();
        // If no flags provided, write both
        if !*init_cfg && !*init_prompts {
            config::Config::write_example(&config_path)?;
            write_example_prompts()?;
        } else {
            if *init_cfg {
                config::Config::write_example(&config_path)?;
            }
            if *init_prompts {
                write_example_prompts()?;
            }
        }
        return Ok(());
    }

    // Handle --write-example-config (legacy)
    if cli.write_example_config {
        let config_path = config::Config::config_file_path();
        config::Config::write_example(&config_path)?;
        return Ok(());
    }

    // Handle --write-example-prompts (legacy)
    if cli.write_example_prompts {
        write_example_prompts()?;
        return Ok(());
    }

    info!("Starting ritsu-server v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration first to get model name for Ollama warmup
    let mut config = if let Some(config_path) = cli.config {
        info!("Loading configuration from: {}", config_path.display());
        config::Config::load_from_path(&config_path)?
    } else {
        let default_path = config::Config::config_file_path();
        info!(
            "Loading configuration from default path: {}",
            default_path.display()
        );
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
    // Choose model name from the configured default backend, falling back to the first configured backend or a hardcoded default
    let model_name = config
        .llm
        .backends
        .iter()
        .find(|b| b.name == config.llm.default_backend)
        .or_else(|| config.llm.backends.first())
        .map(|b| b.model.as_str())
        .unwrap_or("llama3.2:3b");
    start_ollama_if_needed(model_name).await;

    // Initialize database
    let db = database::Database::new(&config.server.database_path).await?;
    info!("Database initialized at: {}", config.server.database_path);

    // Initialize memory manager
    let memory = std::sync::Arc::new(memory::MemoryManager::new(
        db.connection.clone(),
        config.server.database_path.clone(),
        config.memory.include_ai_generated,
    ));
    info!("Memory manager initialized");

    // Initialize conversation manager
    let conversation_manager = std::sync::Arc::new(conversations::ConversationManager::new(
        db.connection.clone(),
    ));
    info!("Conversation manager initialized");

    // Initialize preferences manager
    let preferences_manager =
        std::sync::Arc::new(preferences::PreferencesManager::new(db.connection.clone()));
    info!("Preferences manager initialized");

    // Set up graceful shutdown signal
    let shutdown_flag = std::sync::Arc::new(tokio::sync::Notify::new());

    // Initialize task manager
    let task_manager = std::sync::Arc::new(tasks::TaskManager::new(db.connection.clone()));
    info!("Task manager initialized");

    // Initialize trigger registry
    let trigger_registry = std::sync::Arc::new(trigger::TriggerRegistry::new(
        config.server.database_path.clone(),
    ));
    trigger_registry.register_builtin_triggers(&config).await?;
    info!(
        "Trigger registry initialized with {} triggers",
        trigger_registry.get_all_triggers().await.len()
    );

    // Initialize server state
    let server_state = std::sync::Arc::new(state::ServerState::new());
    info!("Server state initialized");

    // Start background reconnection task to client daemon (preferred persistent connection)
    server_state.clone().start_client_daemon_reconnector();

    // Initialize tool registry (needs memory, task_manager, trigger_registry, database, server_state, preferences)
    let tool_registry =
        std::sync::Arc::new(tools::ToolRegistry::new().with_database(db.connection.clone()));
    tools::register_all_tools(
        &tool_registry,
        memory.clone(),
        conversation_manager.clone(),
        task_manager.clone(),
        trigger_registry.clone(),
        server_state.clone(),
        preferences_manager.clone(),
        std::sync::Arc::new(config.clone()),
    )
    .await;
    info!("Tool registry initialized with usage tracking");

    // Initialize LLM client (needs tool registry for tool calling)
    let llm_client = std::sync::Arc::new(
        llm::LlmClient::new(&config.llm, &config.timeouts, tool_registry.clone()).await?,
    );
    info!(
        "LLM client initialized with {} backend(s)",
        config.llm.backends.len()
    );

    // Register default pre-LLM prompt hooks (inject task summaries into prompts)
    {
        let task_manager_for_hook = task_manager.clone();
        let hook = Arc::new(move || {
            let task_manager = task_manager_for_hook.clone();
            let fut = async move {
                match task_manager.get_task_summary().await {
                    Ok(s) if !s.is_empty() => {
                        Ok(Some(format!("[lucide:clipboard] Current Tasks:\n{}", s)))
                    }
                    _ => Ok(None),
                }
            };
            Box::pin(fut)
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = anyhow::Result<Option<String>>> + Send>,
                >
        });
        // Register hook
        crate::prompt::register_pre_hook(hook).await;
    }

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
            let mut sigterm =
                signal(SignalKind::terminate()).expect("Failed to setup SIGTERM handler");
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
    })
    .await;

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
    })
    .await;

    if !ollama_installed.unwrap_or(false) {
        tracing::debug!("Ollama not found in PATH");
        return;
    }

    // Check if Ollama is already running by trying to connect
    let health_check = tokio::process::Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "http://localhost:11434/",
        ])
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
            .truncate(true)
            .write(true)
            .open("/tmp/ritsu-ollama.log")
    })
    .await;

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
                    .args([
                        "-s",
                        "-o",
                        "/dev/null",
                        "-w",
                        "%{http_code}",
                        "http://localhost:11434/",
                    ])
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
            tracing::info!(
                "Warming up Ollama model '{model_name}' (this may take 5-10 seconds)..."
            );
            let warmup_body = format!(
                r#"{{"model":"{model_name}","messages":[{{"role":"user","content":"hi"}}],"stream":false}}"#
            );
            let warmup = tokio::time::timeout(
                tokio::time::Duration::from_secs(60),
                tokio::process::Command::new("curl")
                    .args([
                        "-s",
                        "-X",
                        "POST",
                        "http://localhost:11434/api/chat",
                        "-d",
                        &warmup_body,
                        "-H",
                        "Content-Type: application/json",
                    ])
                    .output(),
            )
            .await;

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

/// Write example system prompt files
fn write_example_prompts() -> Result<()> {
    use std::fs;

    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let prompts_dir = home.join(".config/ritsu/prompts");

    // Create directory
    fs::create_dir_all(&prompts_dir)?;

    // Base system prompt
    let base_path = prompts_dir.join("system_base.md");
    let base_content = r"# Ritsu System Prompt

You are Ritsu, an autonomous AI assistant with the following capabilities:

## Core Identity
- Self-triggering agent that can schedule and execute tasks independently
- Maintain long-term memory through conversation summaries
- Learn user preferences over time
- Proactive in suggesting improvements and automations

## Interaction Style
- Concise and direct in responses
- Ask clarifying questions when needed
- Provide context for your actions
- Be transparent about limitations

## Key Responsibilities
1. Manage scheduled tasks and triggers
2. Maintain organized memory (notes, summaries)
3. Execute tools to help the user
4. Learn from conversations to improve service
5. Suggest optimizations and automations

## Available Tools
You have access to various tools for:
- Creating notes and managing memory
- Scheduling triggers for future actions
- Sending notifications to the user
- Managing tasks with priorities and due dates
- Analyzing patterns in user behavior

Use these tools proactively to assist the user effectively.
";
    fs::write(&base_path, base_content)?;
    println!(
        "{} Created: {}",
        nerd_font::categories::Fa::Check,
        base_path.display()
    );

    // Chat context prompt
    let chat_path = prompts_dir.join("chat.md");
    let chat_content = r"# Chat Context

You are in an interactive chat session with the user.

## Behavior
- Respond naturally and conversationally
- Keep responses focused and relevant
- Use tools when appropriate (notifications, notes, tasks)
- Reference relevant memory when helpful

## Response Style
- Be helpful and attentive
- Clarify ambiguous requests
- Provide actionable suggestions
";
    fs::write(&chat_path, chat_content)?;
    println!(
        "{} Created: {}",
        nerd_font::categories::Fa::Check,
        chat_path.display()
    );

    // Background task prompt
    let background_path = prompts_dir.join("background.md");
    let background_content = r"# Background Task Context

You are executing a scheduled background task.

## Behavior
- Complete the task efficiently
- Use tools to accomplish goals (notifications, notes)
- Log important findings
- Return concise summary of actions taken

## Decision Making
- Only notify user for important events
- Create notes for information worth remembering
- Suggest new triggers if patterns emerge
";
    fs::write(&background_path, background_content)?;
    println!(
        "{} Created: {}",
        nerd_font::categories::Fa::Check,
        background_path.display()
    );

    // Create subdirectory for specialized prompts
    let background_dir = prompts_dir.join("background");
    fs::create_dir_all(&background_dir)?;

    // Compaction prompt
    let compact_path = background_dir.join("compact.md");
    let compact_content = r"# Memory Compaction Context

You are compacting conversation history into a summary.

## Task
- Extract key information from conversations
- Identify important decisions and preferences
- Tag content appropriately
- Be concise but preserve context

## Format
Generate a well-structured summary with:
- Key topics discussed
- Important decisions made
- Action items identified
- User preferences learned
";
    fs::write(&compact_path, compact_content)?;
    println!(
        "{} Created: {}",
        nerd_font::categories::Fa::Check,
        compact_path.display()
    );

    // Pattern analysis prompt
    let pattern_path = background_dir.join("pattern.md");
    let pattern_content = r"# Pattern Recognition Context

You are analyzing user behavior patterns.

## Task
- Identify recurring themes and preferences
- Detect optimal timing for tasks
- Recognize automation opportunities
- Note communication preferences

## Output
Produce insights about:
- Common workflows
- Preferred interaction times
- Tool usage patterns
- Suggested optimizations
";
    fs::write(&pattern_path, pattern_content)?;
    println!(
        "{} Created: {}",
        nerd_font::categories::Fa::Check,
        pattern_path.display()
    );

    // Morning briefing prompt
    let briefing_path = background_dir.join("briefing.md");
    let briefing_content = r"# Morning Briefing Context

You are preparing a daily briefing for the user.

## Task
- Summarize yesterday's activities
- List pending tasks by priority
- Note upcoming events/deadlines
- Provide relevant reminders

## Style
- Brief and scannable
- Prioritize actionable items
- Highlight urgent matters
- Be encouraging and positive
";
    fs::write(&briefing_path, briefing_content)?;
    println!(
        "{} Created: {}",
        nerd_font::categories::Fa::Check,
        briefing_path.display()
    );

    println!();
    println!("All example prompts created successfully!");
    println!("Edit these files to customize Ritsu's behavior.");

    Ok(())
}
