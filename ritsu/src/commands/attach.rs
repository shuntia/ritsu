//! Attach to server and client logs

use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Tail both server and client log files and print prefixed output.
pub async fn attach() -> Result<()> {
    let server_log = "/tmp/ritsu-server.log";
    let client_log = "/tmp/ritsu.log";

    println!(
        "Attaching to {} and {} (press Ctrl-C to exit)",
        server_log, client_log
    );

    let mut tail_server = Command::new("tail")
        .arg("-F")
        .arg(server_log)
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to spawn tail for server log")?;

    let mut tail_client = Command::new("tail")
        .arg("-F")
        .arg(client_log)
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to spawn tail for client log")?;

    let server_stdout = tail_server
        .stdout
        .take()
        .context("Failed to take stdout of server tail")?;

    let client_stdout = tail_client
        .stdout
        .take()
        .context("Failed to take stdout of client tail")?;

    let server_reader = BufReader::new(server_stdout);
    let client_reader = BufReader::new(client_stdout);

    let server_task = tokio::spawn(async move {
        let mut lines = server_reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("[server] {}", line);
        }
    });

    let client_task = tokio::spawn(async move {
        let mut lines = client_reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("[client] {}", line);
        }
    });

    // Wait for Ctrl-C
    tokio::signal::ctrl_c()
        .await
        .context("Failed to listen for Ctrl-C")?;
    println!("Received Ctrl-C, shutting down tail processes...");

    // Try to kill child processes; ignore errors
    let _ = tail_server.kill();
    let _ = tail_client.kill();

    // Wait for reader tasks to finish
    let _ = server_task.await;
    let _ = client_task.await;

    Ok(())
}
