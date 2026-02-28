#![allow(clippy::uninlined_format_args)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::unused_async)]
//! Ritsu Client - Unified CLI and GUI client

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod config;
mod gui;
mod gui_icons;
mod ipc;

#[derive(Parser)]
#[command(name = "ritsu")]
#[command(about = "Ritsu - Self-triggering AI agent client", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the client daemon (handles notifications and GUI)
    Start,

    /// Stop the client daemon
    Stop,

    /// Check client daemon status
    Status,

    /// Restart the client daemon
    Restart,

    /// Manage the ritsu-server daemon
    #[command(subcommand)]
    Server(ServerCommands),

    /// Forcefully halt processes
    #[command(subcommand)]
    Halt(HaltCommands),

    /// Open chat GUI interface
    Chat {
        /// Session ID to select on startup
        #[arg(long)]
        session: Option<String>,

        /// Initial message to display in chat GUI
        #[arg(long)]
        message: Option<String>,
    },

    /// Run TUI config editor for system prompts (uses ratatui)
    Config,

    /// Attach to server and client logs
    Attach,

    /// Send a quick message to ritsu
    Send {
        /// Message to send
        message: String,

        /// Start a new session (don't continue previous conversation)
        #[arg(short, long)]
        new_session: bool,
    },

    /// Manage triggers
    #[command(subcommand)]
    Trigger(TriggerCommands),

    /// Manage tasks
    #[command(subcommand)]
    Task(TaskCommands),

    /// Manage system prompt
    #[command(subcommand)]
    Prompt(PromptCommands),

    /// Export conversation history
    Export {
        /// Session ID to export (defaults to current session)
        #[arg(short, long)]
        session: Option<String>,

        /// Output file path
        #[arg(short, long, default_value = "conversation.txt")]
        output: String,

        /// Export format (txt, json, md)
        #[arg(short, long, default_value = "txt")]
        format: String,
    },

    /// Query memory
    Memory {
        /// Number of days to query
        #[arg(short, long, default_value = "7")]
        days: u32,
    },

    /// Query notes
    Notes {
        /// Filter by tag
        #[arg(short, long)]
        tag: Option<String>,
    },

    /// Clear all memory (for testing)
    ClearMemory {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        noconfirm: bool,
    },

    /// Initialize example files (config and prompts)
    Init {
        /// Write example client configuration file
        #[arg(short = 'c', long)]
        config: bool,

        /// Write example prompts
        #[arg(short = 'p', long)]
        prompts: bool,
    },

    /// Developer tools and inspection commands (hidden)
    #[command(subcommand, hide = true)]
    Dev(DevCommands),
}

#[derive(Subcommand)]
enum ServerCommands {
    /// Start the ritsu-server daemon
    Start {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
    },

    /// Stop the ritsu-server daemon
    Stop,

    /// Check ritsu-server status
    Status,

    /// Restart the ritsu-server daemon
    Restart {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
    },
}

#[derive(Subcommand)]
enum HaltCommands {
    /// Halt the server daemon (forceful shutdown)
    Server {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        noconfirm: bool,
    },

    /// Halt client processes (GUI windows)
    Client {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        noconfirm: bool,
    },
}

#[derive(Subcommand)]
enum TriggerCommands {
    /// List all triggers
    List,

    /// Add a new trigger
    Add {
        /// Trigger name
        name: String,

        /// Cron-like schedule
        #[arg(short, long)]
        time: String,
    },

    /// Disable a trigger
    Disable {
        /// Trigger name
        name: String,
    },

    /// Delete a trigger
    Delete {
        /// Trigger name
        name: String,
    },
}

#[derive(Subcommand)]
enum TaskCommands {
    /// List tasks
    List {
        /// Filter by status
        #[arg(short, long)]
        status: Option<String>,
    },

    /// Add a new task
    Add {
        /// Task title
        title: String,

        /// Priority (low, medium, high, urgent)
        #[arg(short, long, default_value = "medium")]
        priority: String,

        /// Due date (YYYY-MM-DD)
        #[arg(short = 'd', long)]
        due: Option<String>,
    },

    /// Update task status
    Update {
        /// Task ID
        id: i64,

        /// New status
        #[arg(short, long)]
        status: String,
    },
}

#[derive(Subcommand)]
enum PromptCommands {
    /// Show current system prompt
    Show,

    /// Set system prompt from file
    Set {
        /// Path to prompt file
        path: String,
    },

    /// Edit system prompt in $EDITOR
    Edit,
}

#[derive(Subcommand)]
enum DevCommands {
    /// Show effective system prompt
    Prompt,

    /// Show model information and stats
    Model,

    /// Show server configuration
    Config,

    /// Show client configuration
    ClientConfig,

    /// Show database statistics
    DbStats,

    /// Test connection to server
    Ping {
        /// Number of ping attempts
        #[arg(short, long, default_value = "1")]
        count: u32,
    },

    /// Clear session file
    ClearSession,

    /// Clear client cache
    ClearCache,

    /// Export full database
    ExportDb {
        /// Output file path
        #[arg(short, long, default_value = "ritsu_dump.json")]
        output: String,
    },

    /// Inspect conversation session
    InspectSession {
        /// Session ID (defaults to current)
        session_id: Option<String>,
    },

    /// List all sessions
    ListSessions {
        /// Limit number of results
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },

    /// Show tool usage statistics
    ToolStats,

    /// Show memory compaction status
    MemoryStatus,

    /// Force memory compaction (dangerous)
    ForceCompact {
        /// Skip confirmation
        #[arg(short = 'y', long)]
        noconfirm: bool,
    },

    /// Reset the server database (destructive)
    DbReset {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        noconfirm: bool,
    },
    /// Rebuild database indexes
    ReindexDb {
        /// Skip confirmation
        #[arg(short = 'y', long)]
        noconfirm: bool,
    },
}

fn main() -> Result<()> {
    // Initialize tracing subscriber for CLI and GUI
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_timer(tracing_subscriber::fmt::time::LocalTime::rfc_3339())
        .init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { config, prompts } => {
            let config_path = config::ClientConfig::config_file_path();
            // If no flags provided, write both config and prompts
            if !config && !prompts {
                config::ClientConfig::write_example_config(&config_path)?;
                config::ClientConfig::write_example_prompts()?;
            } else {
                if config {
                    config::ClientConfig::write_example_config(&config_path)?;
                }
                if prompts {
                    config::ClientConfig::write_example_prompts()?;
                }
            }
            Ok(())
        }
        Commands::Chat { session, message } => {
            // Detect another running chat instance and exit this new one to avoid spawning duplicates that can cause resource issues.
            #[cfg(target_os = "linux")]
            {
                if let Ok(output) = std::process::Command::new("pgrep").arg("-f").arg("ritsu chat").output() {
                    if output.status.success() && !output.stdout.is_empty() {
                        let my_pid = std::process::id().to_string();
                        let pids = String::from_utf8_lossy(&output.stdout);
                        for pid in pids.lines() {
                            if pid.trim() != my_pid {
                                eprintln!("Detected another ritsu chat instance (PID {}). Exiting new instance to avoid duplicates.", pid.trim());
                                std::process::exit(0);
                            }
                        }
                    }
                }
            }

            println!("Starting Ritsu GUI...");
            // Run GUI - it creates its own runtime
            gui::run_blocking(session, message)
        }
        // All other commands need async runtime
        _ => tokio::runtime::Runtime::new()?.block_on(async {
            match cli.command {
                Commands::Start => commands::client_daemon::run().await?,
                Commands::Stop => commands::client_daemon::stop().await?,
                Commands::Status => commands::client_daemon::status().await?,
                Commands::Restart => commands::client_daemon::restart().await?,
                Commands::Server(cmd) => match cmd {
                    ServerCommands::Start { config } => {
                        commands::daemon::start(config.as_deref()).await?;
                    }
                    ServerCommands::Stop => commands::daemon::stop().await?,
                    ServerCommands::Status => commands::daemon::status().await?,
                    ServerCommands::Restart { config } => {
                        commands::daemon::restart(config.as_deref()).await?;
                    }
                },
                Commands::Halt(cmd) => commands::daemon::handle_halt(cmd).await?,
                Commands::Send {
                    message,
                    new_session,
                } => commands::send::send_message(&message, new_session).await?,
                Commands::Trigger(cmd) => commands::trigger::handle(cmd).await?,
                Commands::Task(cmd) => commands::task::handle(cmd).await?,
                Commands::Config => commands::config::handle().await?,
                Commands::Prompt(cmd) => commands::prompt::handle(cmd).await?,
                Commands::Export {
                    session,
                    output,
                    format,
                } => {
                    commands::export::export_conversation(session.as_deref(), &output, &format)
                        .await?;
                }
                Commands::Memory { days } => commands::memory::query_memory(Some(days)).await?,
                Commands::Notes { tag } => commands::memory::query_notes(tag.as_deref()).await?,
                Commands::ClearMemory { noconfirm } => {
                    commands::memory::clear_memory(noconfirm).await?;
                }
                Commands::Dev(cmd) => commands::dev::handle(cmd).await?,
                Commands::Attach => commands::attach::attach().await?,
                Commands::Chat { session: _, message: _ } | Commands::Init { config: _, prompts: _ } => {}
            }
            Ok(())
        }),
    }
}
