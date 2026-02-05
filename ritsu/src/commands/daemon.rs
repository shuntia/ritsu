//! Daemon management commands

use anyhow::{Context, Result};
use std::process::Command;

pub async fn start(config_path: Option<&str>) -> Result<()> {
    println!("Starting ritsu-server...");
    
    // Check if already running
    if is_running().await {
        println!("Server is already running");
        return Ok(());
    }

    // Spawn ritsu-server as a detached process
    let mut cmd = Command::new("ritsu-server");
    
    if let Some(config) = config_path {
        cmd.arg("--config").arg(config);
        println!("Using config file: {}", config);
    }
    
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn ritsu-server")?;

    // Wait a moment for startup
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    if is_running().await {
        println!("✓ Server started successfully");
        Ok(())
    } else {
        anyhow::bail!("Server failed to start");
    }
}

pub async fn stop() -> Result<()> {
    println!("Stopping ritsu-server...");
    
    if !is_running().await {
        println!("Server is not running");
        return Ok(());
    }

    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    match client.send_request(ritsu_common::protocol::ClientRequest::Shutdown).await {
        Ok(_) => {
            println!("✓ Server stopped successfully");
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to send shutdown: {}", e);
            Err(e)
        }
    }
}

pub async fn status() -> Result<()> {
    if is_running().await {
        println!("✓ Server is running");
    } else {
        println!("✗ Server is not running");
    }
    Ok(())
}

pub async fn restart(config_path: Option<&str>) -> Result<()> {
    stop().await.context("Failed to stop server")?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    start(config_path).await.context("Failed to start server")?;
    Ok(())
}

async fn is_running() -> bool {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    client.ping().await.unwrap_or(false)
}

pub async fn handle_halt(cmd: crate::HaltCommands) -> Result<()> {
    use crate::HaltCommands;
    
    match cmd {
        HaltCommands::Server { noconfirm } => halt_server(noconfirm).await,
        HaltCommands::Client { noconfirm } => halt_client(noconfirm).await,
    }
}

async fn halt_server(noconfirm: bool) -> Result<()> {
    if !noconfirm {
        println!("⚠️  WARNING: This will forcefully terminate the ritsu server.");
        println!("Any in-progress operations will be interrupted.");
        print!("Are you sure you want to continue? [y/N]: ");
        
        use std::io::Write;
        std::io::stdout().flush()?;
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let response = input.trim().to_lowercase();
        
        if response != "y" && response != "yes" {
            println!("Cancelled.");
            return Ok(());
        }
    }
    
    println!("Halting ritsu-server...");
    
    // Find ritsu-server process
    let output = std::process::Command::new("pgrep")
        .arg("-f")
        .arg("ritsu-server")
        .output()
        .context("Failed to find ritsu-server process")?;
    
    if !output.status.success() || output.stdout.is_empty() {
        println!("No ritsu-server process found");
        return Ok(());
    }
    
    let pids = String::from_utf8_lossy(&output.stdout);
    let mut killed_count = 0;
    
    for pid in pids.lines() {
        if let Ok(pid_num) = pid.trim().parse::<i32>() {
            println!("Sending SIGTERM to PID {}", pid_num);
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(pid.trim())
                .status();
            killed_count += 1;
        }
    }
    
    if killed_count > 0 {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        println!("✓ Halted {} ritsu-server process(es)", killed_count);
    }
    
    Ok(())
}

async fn halt_client(noconfirm: bool) -> Result<()> {
    if !noconfirm {
        println!("⚠️  WARNING: This will forcefully close all ritsu GUI windows.");
        print!("Are you sure you want to continue? [y/N]: ");
        
        use std::io::Write;
        std::io::stdout().flush()?;
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let response = input.trim().to_lowercase();
        
        if response != "y" && response != "yes" {
            println!("Cancelled.");
            return Ok(());
        }
    }
    
    println!("Halting ritsu client processes...");
    
    // Find ritsu chat processes
    let output = std::process::Command::new("pgrep")
        .arg("-f")
        .arg("ritsu chat")
        .output()
        .context("Failed to find ritsu client processes")?;
    
    if !output.status.success() || output.stdout.is_empty() {
        println!("No ritsu client processes found");
        return Ok(());
    }
    
    let pids = String::from_utf8_lossy(&output.stdout);
    let mut killed_count = 0;
    
    for pid in pids.lines() {
        if let Ok(pid_num) = pid.trim().parse::<i32>() {
            println!("Sending SIGTERM to PID {}", pid_num);
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(pid.trim())
                .status();
            killed_count += 1;
        }
    }
    
    if killed_count > 0 {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        println!("✓ Halted {} ritsu client process(es)", killed_count);
    }
    
    Ok(())
}
