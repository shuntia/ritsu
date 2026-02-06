//! Client daemon for handling notifications, GUI launches, and request proxying

use anyhow::{Context, Result};
use ritsu_common::protocol::{ClientToServerResponse, NotificationUrgency, ServerToClientRequest};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

const CLIENT_DAEMON_SOCKET: &str = "/tmp/ritsu-client.sock";
const SERVER_SOCKET: &str = "/tmp/ritsu.sock";

pub async fn run() -> Result<()> {
    info!("Starting Ritsu client daemon...");

    // Remove existing socket if present
    if Path::new(CLIENT_DAEMON_SOCKET).exists() {
        std::fs::remove_file(CLIENT_DAEMON_SOCKET)
            .context("Failed to remove existing client daemon socket")?;
    }

    // Bind to client daemon socket for GUI connections
    let listener = UnixListener::bind(CLIENT_DAEMON_SOCKET)
        .context("Failed to bind client daemon socket")?;
    info!("Client daemon listening on {}", CLIENT_DAEMON_SOCKET);

    // Also maintain connection to server for tool requests
    let server_connection = Arc::new(Mutex::new(None::<UnixStream>));
    let server_conn_clone = Arc::clone(&server_connection);

    // Spawn task to handle incoming tool requests from server
    tokio::spawn(async move {
        loop {
            match connect_to_server().await {
                Ok(stream) => {
                    info!("Connected to server for tool requests");
                    *server_conn_clone.lock().await = Some(stream);
                    
                    // Wait for tool requests from server
                    if let Err(e) = handle_server_tool_requests(&server_conn_clone).await {
                        error!("Error handling server tool requests: {}", e);
                    }
                    
                    *server_conn_clone.lock().await = None;
                }
                Err(e) => {
                    error!("Failed to connect to server: {}", e);
                }
            }
            
            // Retry connection every 5 seconds
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });

    // Accept GUI connections and proxy requests
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                info!("New GUI connection accepted");
                tokio::spawn(async move {
                    if let Err(e) = handle_gui_connection(stream).await {
                        error!("Error handling GUI connection: {}", e);
                    } else {
                        info!("GUI connection closed gracefully");
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}

async fn connect_to_server() -> Result<UnixStream> {
    UnixStream::connect(SERVER_SOCKET)
        .await
        .context("Failed to connect to server")
}

async fn handle_server_tool_requests(
    server_connection: &Arc<Mutex<Option<UnixStream>>>,
) -> Result<()> {
    loop {
        let mut guard = server_connection.lock().await;
        let stream = guard
            .as_mut()
            .context("Server connection not established")?;

        // Read request from server
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            return Err(anyhow::anyhow!("Server disconnected"));
        }

        let len = u32::from_le_bytes(len_buf) as usize;
        if len > 10_000_000 {
            return Err(anyhow::anyhow!("Message too large"));
        }

        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;

        let request: ServerToClientRequest = postcard::from_bytes(&buf)?;
        drop(guard); // Release lock while handling request

        // Handle tool request
        let response = match handle_tool_request(request).await {
            Ok(()) => ClientToServerResponse::Success,
            Err(e) => ClientToServerResponse::Error {
                message: e.to_string(),
            },
        };

        // Send response back to server
        let response_bytes = postcard::to_allocvec(&response)?;
        let len_bytes = (response_bytes.len() as u32).to_le_bytes();

        let mut guard = server_connection.lock().await;
        if let Some(stream) = guard.as_mut() {
            stream.write_all(&len_bytes).await?;
            stream.write_all(&response_bytes).await?;
            stream.flush().await?;
        }
    }
}

async fn handle_tool_request(request: ServerToClientRequest) -> Result<()> {
    match request {
        ServerToClientRequest::NotifyUser {
            title,
            message,
            urgency,
        } => handle_notification(&title, &message, urgency).await,
        ServerToClientRequest::OpenChat { message, session_id } => {
            handle_open_chat(message.as_deref(), session_id.as_deref()).await
        }
        ServerToClientRequest::FocusChat => handle_focus_chat().await,
    }
}

async fn handle_gui_connection(mut stream: UnixStream) -> Result<()> {
    info!("Handling new GUI connection, connecting to server...");
    let mut server_stream = connect_to_server().await?;
    info!("Connected to server, starting proxy loop");

    loop {
        // Read request from GUI (using big-endian to match IpcClient)
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            info!("GUI disconnected");
            break;
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 10_000_000 {
            warn!("Message too large from GUI: {} bytes", len);
            break;
        }

        info!("Received {} bytes from GUI, proxying to server", len);
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;

        // Check if this is a SendMessage request (needs streaming support)
        let is_send_message = if let Ok(req) = postcard::from_bytes::<ritsu_common::protocol::ClientRequest>(&buf) {
            matches!(req, ritsu_common::protocol::ClientRequest::SendMessage { .. })
        } else {
            false
        };

        // Proxy to server (keep big-endian)
        let len_bytes = (buf.len() as u32).to_be_bytes();
        server_stream.write_all(&len_bytes).await?;
        server_stream.write_all(&buf).await?;
        server_stream.flush().await?;

        if is_send_message {
            // For SendMessage, we need to forward multiple push notifications
            info!("Proxying streaming responses...");
            loop {
                // Read push notification from server
                let mut len_buf = [0u8; 4];
                if server_stream.read_exact(&mut len_buf).await.is_err() {
                    error!("Server disconnected during streaming");
                    break;
                }

                let len = u32::from_be_bytes(len_buf) as usize;
                let mut buf = vec![0u8; len];
                server_stream.read_exact(&mut buf).await?;

                // Forward to GUI
                let len_bytes = (buf.len() as u32).to_be_bytes();
                stream.write_all(&len_bytes).await?;
                stream.write_all(&buf).await?;
                stream.flush().await?;

                // Check if this is the final chunk
                if let Ok(ritsu_common::protocol::ServerPush::MessageChunk { is_final: true, .. }) = 
                    postcard::from_bytes::<ritsu_common::protocol::ServerPush>(&buf)
                {
                    info!("Final chunk received, streaming complete");
                    break;
                }
            }
        } else {
            // For other requests, simple request/response
            info!("Waiting for server response...");
            let mut len_buf = [0u8; 4];
            server_stream.read_exact(&mut len_buf).await?;

            let len = u32::from_be_bytes(len_buf) as usize;
            info!("Server responded with {} bytes, forwarding to GUI", len);
            let mut buf = vec![0u8; len];
            server_stream.read_exact(&mut buf).await?;

            // Forward to GUI
            let len_bytes = (buf.len() as u32).to_be_bytes();
            stream.write_all(&len_bytes).await?;
            stream.write_all(&buf).await?;
            stream.flush().await?;
            info!("Response forwarded successfully");
        }
    }

    Ok(())
}

async fn handle_notification(
    title: &str,
    message: &str,
    urgency: NotificationUrgency,
) -> Result<()> {
    info!("Showing notification: {} - {}", title, message);

    #[cfg(target_os = "linux")]
    {
        use notify_rust::{Notification, Urgency};

        let urgency_level = match urgency {
            NotificationUrgency::Low => Urgency::Low,
            NotificationUrgency::Normal => Urgency::Normal,
            NotificationUrgency::Critical => Urgency::Critical,
        };

        Notification::new()
            .summary(title)
            .body(message)
            .icon("dialog-information")
            .appname("Ritsu")
            .urgency(urgency_level)
            .timeout(0) // No timeout
            .show()
            .context("Failed to show notification")?;
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Fallback for non-Linux platforms
        eprintln!("[NOTIFICATION] {}: {}", title, message);
    }

    Ok(())
}

async fn handle_open_chat(message: Option<&str>, session_id: Option<&str>) -> Result<()> {
    info!("Opening chat GUI");

    let mut cmd = Command::new("ritsu");
    cmd.arg("chat");

    if let Some(sid) = session_id {
        cmd.arg("--session");
        cmd.arg(sid);
    }

    if let Some(msg) = message {
        cmd.arg("--message");
        cmd.arg(msg);
    }

    // Spawn GUI process (don't wait for it)
    cmd.spawn().context("Failed to launch GUI")?;

    Ok(())
}

async fn handle_focus_chat() -> Result<()> {
    info!("Attempting to focus chat GUI");

    #[cfg(target_os = "linux")]
    {
        // Try using xdotool to focus the window
        let output = Command::new("xdotool")
            .args(["search", "--name", "Ritsu", "windowactivate"])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                info!("Successfully focused chat window");
                return Ok(());
            }
            Ok(_) => warn!("xdotool failed to find/focus window"),
            Err(e) => warn!("xdotool not available: {}", e),
        }
    }

    // If focus fails or not supported, just log
    warn!("Window focus not supported on this platform or window not found");
    Ok(())
}

pub async fn stop() -> Result<()> {
    println!("Stopping client daemon...");
    
    // Check if daemon is running by checking socket
    if !Path::new(CLIENT_DAEMON_SOCKET).exists() {
        println!("Client daemon is not running");
        return Ok(());
    }
    
    // Find and kill the process
    let output = Command::new("pgrep")
        .args(["-f", "ritsu.*start"])
        .output()?;
    
    if output.status.success() {
        let pids = String::from_utf8_lossy(&output.stdout);
        for pid in pids.lines() {
            if let Ok(pid_num) = pid.parse::<i32>() {
                let _ = Command::new("kill")
                    .arg(pid_num.to_string())
                    .status();
                println!("Stopped client daemon (PID: {})", pid_num);
            }
        }
    } else {
        println!("Client daemon process not found");
    }
    
    // Clean up socket in both cases
    let _ = std::fs::remove_file(CLIENT_DAEMON_SOCKET);
    
    Ok(())
}

pub async fn status() -> Result<()> {
    if Path::new(CLIENT_DAEMON_SOCKET).exists() {
        // Try to connect to verify it's actually running
        match UnixStream::connect(CLIENT_DAEMON_SOCKET).await {
            Ok(_) => {
                println!("Client daemon: Running");
                println!("Socket: {}", CLIENT_DAEMON_SOCKET);
            }
            Err(_) => {
                println!("Client daemon: Socket exists but not responding (stale?)");
            }
        }
    } else {
        println!("Client daemon: Not running");
    }
    
    Ok(())
}

pub async fn restart() -> Result<()> {
    println!("Restarting client daemon...");
    stop().await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Spawn as background process
    Command::new("ritsu")
        .arg("start")
        .spawn()
        .context("Failed to spawn client daemon")?;
    
    println!("Client daemon restarting in background");
    Ok(())
}
