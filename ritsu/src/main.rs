//! Ritsu Client - Unified CLI and GUI client

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser)]
#[command(name = "ritsu")]
#[command(about = "Ritsu - Self-triggering AI agent client", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the ritsu server daemon
    Start,
    
    /// Stop the ritsu server daemon
    Stop,
    
    /// Check server status
    Status,
    
    /// Restart the server daemon
    Restart,
    
    /// Open chat GUI interface
    Chat,
    
    /// Send a quick message to ritsu
    Send {
        /// Message to send
        message: String,
    },
    
    /// Manage triggers
    #[command(subcommand)]
    Trigger(TriggerCommands),
    
    /// Manage tasks
    #[command(subcommand)]
    Task(TaskCommands),
    
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
        Commands::Start => commands::daemon::start()?,
        Commands::Stop => commands::daemon::stop()?,
        Commands::Status => commands::daemon::status()?,
        Commands::Restart => commands::daemon::restart()?,
        Commands::Chat => {
            #[cfg(feature = "gui")]
            commands::chat::run()?;
            
            #[cfg(not(feature = "gui"))]
            {
                eprintln!("GUI feature not enabled. Rebuild with --features gui");
                std::process::exit(1);
            }
        }
        Commands::Send { message } => commands::send::send_message(&message)?,
        Commands::Trigger(cmd) => commands::trigger::handle(cmd)?,
        Commands::Task(cmd) => commands::task::handle(cmd)?,
        Commands::Memory { days } => commands::memory::query_memory(days)?,
        Commands::Notes { tag } => commands::memory::query_notes(tag.as_deref())?,
    }

    Ok(())
}
