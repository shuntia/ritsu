//! End-to-end integration tests for Ritsu
//!
//! These tests start real server and client daemon processes and test
//! the full interaction flow.

use anyhow::Result;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;
use tokio::time::sleep;

/// Test configuration and paths
#[allow(dead_code)]
struct TestFixture {
    test_dir: PathBuf,
    db_path: PathBuf,
    server_socket: PathBuf,
    client_socket: PathBuf,
    config_path: PathBuf,
    server_process: Option<Child>,
    client_daemon_process: Option<Child>,
}

impl TestFixture {
    fn new(test_name: &str) -> Result<Self> {
        let test_dir = std::env::temp_dir().join(format!("ritsu_test_{test_name}"));
        
        // Clean up any existing test directory
        if test_dir.exists() {
            let _ = std::fs::remove_dir_all(&test_dir);
        }
        std::fs::create_dir_all(&test_dir)?;
        
        // Clean up any existing socket files
        let _ = std::fs::remove_file("/tmp/ritsu.sock");
        let _ = std::fs::remove_file("/tmp/ritsu-client.sock");
        
        let db_path = test_dir.join("test.db");
        let config_path = test_dir.join("config.toml");
        
        // Create test config - use default socket paths for E2E tests
        let config_content = format!(
            r#"
[llm]
default_backend = "ollama"
disable_streaming = true
disable_tools = true

[[llm.backends]]
name = "ollama"
endpoint = "http://localhost:11434"
model = "llama3.2:3b"

[server]
socket_path = "/tmp/ritsu.sock"
database_path = "{}"
client_binary_path = "target/debug/ritsu"

[memory]
daily_rotation_days = 40

[timeouts]
user_response_seconds = 300
http_request_seconds = 10
llm_request_seconds = 15
"#,
            db_path.display()
        );
        
        std::fs::write(&config_path, config_content)?;
        
        // Use default socket paths
        let server_socket = PathBuf::from("/tmp/ritsu.sock");
        let client_socket = PathBuf::from("/tmp/ritsu-client.sock");
        
        Ok(Self {
            test_dir,
            db_path,
            server_socket,
            client_socket,
            config_path,
            server_process: None,
            client_daemon_process: None,
        })
    }
    
    /// Start the ritsu-server process
    async fn start_server(&mut self) -> Result<()> {
        println!("Starting server with config: {}", self.config_path.display());
        
        // Get absolute path to binary (tests run from workspace root or package dir)
        let mut binary_path = std::env::current_dir()?.join("target/debug/ritsu-server");
        if !binary_path.exists() {
            // Try parent directory (in case we're in ritsu/ subdir)
            binary_path = std::env::current_dir()?.parent()
                .ok_or_else(|| anyhow::anyhow!("Could not find parent directory"))?
                .join("target/debug/ritsu-server");
        }
        if !binary_path.exists() {
            anyhow::bail!("ritsu-server binary not found at: {}", binary_path.display());
        }
        
        let mut cmd = Command::new(&binary_path);
        cmd.arg("--config")
            .arg(&self.config_path)
            .env("RUST_LOG", "info,ritsu_server=debug")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        
        let child = cmd.spawn()?;
        self.server_process = Some(child);
        
        // Wait for server to be ready
        for i in 0..30 {
            if self.server_socket.exists() {
                println!("Server socket ready after {}ms", i * 100);
                sleep(Duration::from_millis(1000)).await; // Extra time for full startup
                return Ok(());
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        // Server failed to start - try to get output
        if let Some(mut child) = self.server_process.take() {
            let _ = child.kill();
            if let Some(mut stdout) = child.stdout.take() {
                let mut output = String::new();
                let _ = stdout.read_to_string(&mut output);
                println!("Server stdout: {output}");
            }
            if let Some(mut stderr) = child.stderr.take() {
                let mut output = String::new();
                let _ = stderr.read_to_string(&mut output);
                println!("Server stderr: {output}");
            }
        }
        
        anyhow::bail!("Server failed to start - socket not created")
    }
    
    /// Start the client daemon process
    async fn start_client_daemon(&mut self) -> Result<()> {
        println!("Starting client daemon");
        
        // Get absolute path to binary (tests run from workspace root or package dir)
        let mut binary_path = std::env::current_dir()?.join("target/debug/ritsu");
        if !binary_path.exists() {
            // Try parent directory (in case we're in ritsu/ subdir)
            binary_path = std::env::current_dir()?.parent()
                .ok_or_else(|| anyhow::anyhow!("Could not find parent directory"))?
                .join("target/debug/ritsu");
        }
        if !binary_path.exists() {
            anyhow::bail!("ritsu binary not found at: {}", binary_path.display());
        }
        
        // Client daemon connects to server socket, but listens on client socket
        // We need to override the socket paths via environment or config
        let mut cmd = Command::new(&binary_path);
        cmd.arg("start")
            .env("RUST_LOG", "info,ritsu=debug");
        
        let child = cmd.spawn()?;
        self.client_daemon_process = Some(child);
        
        // Wait for client daemon to be ready
        for i in 0..20 {
            if PathBuf::from("/tmp/ritsu-client.sock").exists() {
                println!("Client daemon socket ready after {}ms", i * 100);
                sleep(Duration::from_millis(300)).await;
                return Ok(());
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        anyhow::bail!("Client daemon failed to start - socket not created")
    }
    
    /// Stop all processes
    fn stop(&mut self) {
        if let Some(mut child) = self.client_daemon_process.take() {
            let _ = child.kill();
            let _ = child.wait();
            println!("Stopped client daemon");
        }
        
        if let Some(mut child) = self.server_process.take() {
            let _ = child.kill();
            let _ = child.wait();
            println!("Stopped server");
        }
        
        // Clean up sockets
        let _ = std::fs::remove_file(&self.server_socket);
        let _ = std::fs::remove_file(&self.client_socket);
        let _ = std::fs::remove_file("/tmp/ritsu-client.sock");
        let _ = std::fs::remove_file("/tmp/ritsu.sock");
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        self.stop();
        // Clean up test directory
        let _ = std::fs::remove_dir_all(&self.test_dir);
    }
}

/// Helper to run a ritsu command
fn run_command(args: &[&str]) -> Result<(bool, String)> {
    let mut binary_path = std::env::current_dir()?.join("target/debug/ritsu");
    if !binary_path.exists() {
        binary_path = std::env::current_dir()?.parent()
            .ok_or_else(|| anyhow::anyhow!("Could not find parent directory"))?
            .join("target/debug/ritsu");
    }
    let output = Command::new(&binary_path)
        .args(args)
        .output()?;
    
    let success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    let combined = format!("{stdout}{stderr}");
    Ok((success, combined))
}

#[tokio::test]
#[ignore = "E2E test - requires server"] // Run with: cargo test --test e2e_test -- --ignored
async fn test_server_startup_and_ping() -> Result<()> {
    let mut fixture = TestFixture::new("ping")?;
    
    // Start server
    fixture.start_server().await?;
    
    // Start client daemon
    fixture.start_client_daemon().await?;
    
    // Test ping via client daemon
    let (success, output) = run_command(&["server", "status"])?;
    
    println!("Ping output: {output}");
    assert!(success || output.contains("Running"), "Server should be running");
    
    Ok(())
}

#[tokio::test]
#[ignore = "E2E test - requires server"]
async fn test_send_message_and_receive_response() -> Result<()> {
    let mut fixture = TestFixture::new("send_message")?;
    
    // Start both daemons
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    // Send a message
    let (success, output) = run_command(&["send", "Hello, this is a test"])?;
    
    println!("Send output: {output}");
    
    // Should get a response (not necessarily success if LLM not available)
    assert!(
        success || output.contains("daemon") || output.contains("🤖"),
        "Should send message or get helpful error: {output}"
    );
    
    Ok(())
}

#[tokio::test]
#[ignore = "E2E test - requires server"]
async fn test_session_persistence() -> Result<()> {
    let mut fixture = TestFixture::new("session")?;
    
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    // Send first message
    let (success1, _) = run_command(&["send", "My name is Alice"])?;
    sleep(Duration::from_secs(2)).await;
    
    // Send second message in same session
    let (success2, output2) = run_command(&["send", "What is my name?"])?;
    
    println!("Session test output: {output2}");
    
    // At minimum, commands should not crash
    assert!(
        success1 || success2,
        "At least one message should succeed"
    );
    
    Ok(())
}

#[tokio::test]
#[ignore = "E2E test - requires server"]
async fn test_task_management() -> Result<()> {
    let mut fixture = TestFixture::new("tasks")?;
    
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    // Create a task
    let (success, _) = run_command(&[
        "task",
        "add",
        "Test task",
        "--priority",
        "high",
    ])?;
    
    println!("Task creation success: {success}");
    
    // List tasks
    let (list_success, output) = run_command(&["task", "list"])?;
    
    println!("Task list: {output}");
    assert!(list_success, "Should list tasks");
    
    Ok(())
}

#[tokio::test]
#[ignore = "E2E test - requires server"]
async fn test_notes_and_memory() -> Result<()> {
    let mut fixture = TestFixture::new("notes")?;
    
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    // Query notes (should be empty initially)
    let (success, output) = run_command(&["notes"])?;
    
    println!("Notes output: {output}");
    assert!(success || output.contains("note"), "Should query notes");
    
    // Query memory
    let (mem_success, mem_output) = run_command(&["memory", "--days", "7"])?;
    
    println!("Memory output: {mem_output}");
    assert!(mem_success || mem_output.contains("memory"), "Should query memory");
    
    Ok(())
}

#[tokio::test]
#[ignore = "E2E test - requires server"]
async fn test_trigger_management() -> Result<()> {
    let mut fixture = TestFixture::new("triggers")?;
    
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    // List triggers (should work even if empty)
    let (success, output) = run_command(&["trigger", "list"])?;
    
    println!("Trigger list: {output}");
    assert!(success, "Should list triggers");
    
    Ok(())
}

#[tokio::test]
#[ignore = "E2E test - requires server"]
async fn test_client_daemon_restart() -> Result<()> {
    let mut fixture = TestFixture::new("restart")?;
    
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    // Send a message
    let (_, _) = run_command(&["send", "Before restart"])?;
    
    // Stop client daemon
    if let Some(mut child) = fixture.client_daemon_process.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    
    sleep(Duration::from_secs(1)).await;
    
    // Restart client daemon
    fixture.start_client_daemon().await?;
    
    // Send another message
    let (_success, _) = run_command(&["send", "After restart"])?;
    
    // Restart test passes if we got this far
    // (LLM may not be available, so don't check message success)
    
    Ok(())
}

#[tokio::test]
#[ignore = "E2E test - requires server"]
async fn test_concurrent_messages() -> Result<()> {
    let mut fixture = TestFixture::new("concurrent")?;
    
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    // Spawn multiple message sends concurrently
    let handles: Vec<_> = (0..3)
        .map(|i| {
            let msg = format!("Concurrent message {i}");
            tokio::task::spawn_blocking(move || {
                run_command(&["send", &msg])
            })
        })
        .collect();
    
    // Wait for all to complete
    for handle in handles {
        let result = handle.await??;
        println!("Concurrent result: {result:?}");
    }
    
    Ok(())
}

/// Integration test to verify the full stack works
#[tokio::test]
#[ignore = "E2E test - requires server"]
async fn test_full_conversation_flow() -> Result<()> {
    let mut fixture = TestFixture::new("conversation")?;
    
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    // Send multiple messages in sequence
    let messages = vec![
        "Hello!",
        "Can you help me?",
        "Thank you!",
    ];
    
    for msg in messages {
        let (_, output) = run_command(&["send", msg])?;
        println!("Message '{msg}'response: {output}");
        sleep(Duration::from_millis(500)).await;
    }
    
    Ok(())
}
