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
    Chat,
    
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
        #[arg(long)]
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
        #[arg(long, default_value = "7")]
        days: u32,
    },
    
    /// Query notes
    Notes {
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },
    
    /// Clear all memory (for testing)
    ClearMemory {
        /// Skip confirmation prompt
        #[arg(long)]
        noconfirm: bool,
    },
    
    /// Write example client configuration file
    WriteExampleConfig,
    
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
        #[arg(long)]
        noconfirm: bool,
    },
    
    /// Halt client processes (GUI windows)
    Client {
        /// Skip confirmation prompt
        #[arg(long)]
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
        #[arg(long)]
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
        #[arg(long)]
        status: Option<String>,
    },
    
    /// Add a new task
    Add {
        /// Task title
        title: String,
        
        /// Priority (low, medium, high, urgent)
        #[arg(long, default_value = "medium")]
        priority: String,
        
        /// Due date (YYYY-MM-DD)
        #[arg(long)]
        due: Option<String>,
    },
    
    /// Update task status
    Update {
        /// Task ID
        id: i64,
        
        /// New status
        #[arg(long)]
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
        #[arg(long)]
        noconfirm: bool,
    },
    
    /// Reset the server database (destructive)
    DbReset {
        /// Skip confirmation prompt
        #[arg(long)]
        noconfirm: bool,
    },
    /// Rebuild database indexes
    ReindexDb {
        /// Skip confirmation
        #[arg(long)]
        noconfirm: bool,
    },
}

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::WriteExampleConfig => {
            let config_path = config::ClientConfig::config_file_path();
            config::ClientConfig::write_example(&config_path)?;
            Ok(())
        }
        Commands::Chat => {
            println!("Starting Ritsu GUI...");
            // Run GUI - it creates its own runtime
            gui::run_blocking()
        }
        // All other commands need async runtime
        _ => {
            tokio::runtime::Runtime::new()?.block_on(async {
                match cli.command {
                    Commands::Start => commands::client_daemon::run().await?,
                    Commands::Stop => commands::client_daemon::stop().await?,
                    Commands::Status => commands::client_daemon::status().await?,
                    Commands::Restart => commands::client_daemon::restart().await?,
                    Commands::Server(cmd) => match cmd {
                        ServerCommands::Start { config } => commands::daemon::start(config.as_deref()).await?,
                        ServerCommands::Stop => commands::daemon::stop().await?,
                        ServerCommands::Status => commands::daemon::status().await?,
                        ServerCommands::Restart { config } => commands::daemon::restart(config.as_deref()).await?,
                    },
                    Commands::Halt(cmd) => commands::daemon::handle_halt(cmd).await?,
                    Commands::Send { message, new_session } => commands::send::send_message(&message, new_session).await?,
                    Commands::Trigger(cmd) => commands::trigger::handle(cmd).await?,
                    Commands::Task(cmd) => commands::task::handle(cmd).await?,
                    Commands::Prompt(cmd) => commands::prompt::handle(cmd).await?,
                    Commands::Export { session, output, format } => {
                        commands::export::export_conversation(session.as_deref(), &output, &format).await?;
                    }
                    Commands::Memory { days } => commands::memory::query_memory(Some(days)).await?,
                    Commands::Notes { tag } => commands::memory::query_notes(tag.as_deref()).await?,
                    Commands::ClearMemory { noconfirm } => commands::memory::clear_memory(noconfirm).await?,
                    Commands::Dev(cmd) => commands::dev::handle(cmd).await?,
                    Commands::Chat | Commands::WriteExampleConfig => unreachable!(),
                }
                Ok(())
            })
        }
    }
}
