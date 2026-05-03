//! Client daemon for handling notifications, GUI launches, and request proxying

use anyhow::{Context, Result};
use ritsu_common::protocol::{
    ClientToServerResponse, NotificationUrgency, ServerPush, ServerToClientRequest,
};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

pub async fn run() -> Result<()> {
    info!("Starting Ritsu client daemon...");

    // Load config
    let config = crate::config::ClientConfig::load()?;
    let client_socket = config.client_socket_path();
    let server_socket = config.server_socket_path();

    // Remove existing socket if present
    if Path::new(&client_socket).exists() {
        std::fs::remove_file(&client_socket)
            .context("Failed to remove existing client daemon socket")?;
    }

    // Bind to client daemon socket for GUI connections
    let listener =
        UnixListener::bind(&client_socket).context("Failed to bind client daemon socket")?;
    info!("Client daemon listening on {}", client_socket);

    // Also maintain connection to server for tool requests
    let server_connection = Arc::new(Mutex::new(None::<UnixStream>));
    let server_conn_clone = Arc::clone(&server_connection);
    let server_socket_clone = server_socket.clone();

    // Registry of GUI push channels so server->client tool requests can be forwarded to active GUIs
    let gui_pushers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
    let gui_pushers_for_reconnector = gui_pushers.clone();

    // Spawn task to handle incoming tool requests from server
    // Use quadratic backoff for reconnect attempts: delay = base * attempt^2 (ms), capped to 5 minutes
    let retry_cfg = config.retry.clone();
    tokio::spawn(async move {
        let mut attempt: u64 = 0;
        loop {
            match connect_to_server(&server_socket_clone).await {
                Ok(stream) => {
                    info!("Connected to server for tool requests");
                    // Reset attempt counter on successful connection
                    attempt = 0;
                    *server_conn_clone.lock().await = Some(stream);

                    // Wait for tool requests from server
                    if let Err(e) = handle_server_tool_requests(
                        &server_conn_clone,
                        gui_pushers_for_reconnector.clone(),
                    )
                    .await
                    {
                        error!("Error handling server tool requests: {}", e);
                    }

                    *server_conn_clone.lock().await = None;
                }
                Err(e) => {
                    attempt = attempt.saturating_add(1);
                    // Quadratic backoff in milliseconds
                    let base_delay_ms = retry_cfg.retry_delay_ms;
                    let max_delay_ms: u64 = 5 * 60 * 1000; // 5 minutes
                    let mut delay_ms =
                        base_delay_ms.saturating_mul(attempt.saturating_mul(attempt));
                    if delay_ms == 0 {
                        delay_ms = base_delay_ms.max(1);
                    }
                    if delay_ms > max_delay_ms {
                        delay_ms = max_delay_ms;
                    }
                    warn!(
                        "Could not connect to server: {}, retrying in {}ms (attempt {})",
                        e, delay_ms, attempt
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
            }
        }
    });

    // Accept GUI/CLI connections
    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                info!("New connection accepted on client socket");

                // Try to read an initial frame quickly to determine whether this is
                // a GUI client (which sends a ClientRequest immediately) or the
                // server connecting for server->client requests (may be idle).
                use std::time::Duration;
                let initial_frame = tokio::time::timeout(Duration::from_millis(50), async {
                    let mut len_buf = [0u8; 4];
                    // Try read length
                    if stream.read_exact(&mut len_buf).await.is_err() {
                        return None;
                    }
                    let len = u32::from_be_bytes(len_buf) as usize;
                    if len > 10_000_000 {
                        return None;
                    }
                    let mut body = vec![0u8; len];
                    // Give some time to read the body
                    let _ = tokio::time::timeout(
                        Duration::from_millis(200),
                        stream.read_exact(&mut body),
                    )
                    .await;
                    Some(body)
                })
                .await
                .ok()
                .flatten();

                let server_socket_clone = server_socket.clone();
                let gui_pushers_clone = gui_pushers.clone();

                // Decide handler based on initial frame content if any
                if let Some(body) = initial_frame {
                    // Try parsing as ServerToClientRequest (server connection)
                    if postcard::from_bytes::<ServerToClientRequest>(&body).is_ok() {
                        info!("Accepted connection identified as server->client connection");
                        tokio::spawn(async move {
                            if let Err(e) = handle_incoming_server_connection(
                                stream,
                                Some(body),
                                gui_pushers_clone,
                            )
                            .await
                            {
                                error!("Error handling incoming server connection: {}", e);
                            }
                        });
                        continue;
                    }

                    // Try parsing as ClientRequest (GUI)
                    if postcard::from_bytes::<ritsu_common::protocol::ClientRequest>(&body).is_ok()
                    {
                        info!("Accepted connection identified as GUI client (initial request present)");
                        tokio::spawn(async move {
                            if let Err(e) = handle_gui_connection_with_initial(
                                stream,
                                &server_socket_clone,
                                gui_pushers_clone,
                                Some(body),
                            )
                            .await
                            {
                                error!("Error handling GUI connection: {}", e);
                            }
                        });
                        continue;
                    }

                    // Unknown initial payload - default to GUI handler
                    info!("Accepted connection with unknown initial payload; treating as GUI");
                    tokio::spawn(async move {
                        if let Err(e) = handle_gui_connection_with_initial(
                            stream,
                            &server_socket_clone,
                            gui_pushers_clone,
                            Some(body),
                        )
                        .await
                        {
                            error!("Error handling GUI connection: {}", e);
                        }
                    });
                } else {
                    // No initial data: likely the server connecting (idle). Treat as incoming server connection.
                    info!("No immediate data on accepted connection; treating as server->client connection");
                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_incoming_server_connection(stream, None, gui_pushers_clone).await
                        {
                            error!("Error handling incoming server connection: {}", e);
                        }
                    });
                }
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}

async fn connect_to_server(server_socket: &str) -> Result<UnixStream> {
    UnixStream::connect(server_socket)
        .await
        .context("Failed to connect to server")
}

async fn handle_server_tool_requests(
    server_connection: &Arc<Mutex<Option<UnixStream>>>,
    gui_pushers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>,
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

        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 10_000_000 {
            return Err(anyhow::anyhow!("Message too large"));
        }

        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;

        let request: ServerToClientRequest = postcard::from_bytes(&buf)?;
        drop(guard); // Release lock while handling request

        // Handle tool request, forwarding GUI-focused requests to GUI pushers when possible
        let response = match handle_tool_request(request, gui_pushers.clone()).await {
            Ok(()) => ClientToServerResponse::Success,
            Err(e) => ClientToServerResponse::Error {
                message: e.to_string(),
            },
        };

        // Send response back to server
        let response_bytes = postcard::to_allocvec(&response)?;
        let len_bytes = (response_bytes.len() as u32).to_be_bytes();

        let mut guard = server_connection.lock().await;
        if let Some(stream) = guard.as_mut() {
            stream.write_all(&len_bytes).await?;
            stream.write_all(&response_bytes).await?;
            stream.flush().await?;
        }
    }
}

async fn handle_tool_request(
    request: ServerToClientRequest,
    gui_pushers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>,
) -> Result<()> {
    match request {
        ServerToClientRequest::NotifyUser {
            title,
            message,
            urgency,
        } => handle_notification(&title, &message, urgency).await,
        ServerToClientRequest::OpenChat { message, session_id } => {
            // Prefer requesting focus/open on connected GUI clients. If none accept, spawn the GUI.
            let push = ServerPush::OpenChat {
                message: message.clone(),
                urgency: NotificationUrgency::Normal,
            };
            let body = postcard::to_allocvec(&push)?;

            // Snapshot current pushers
            let senders = {
                let guard = gui_pushers.lock().await;
                guard.clone()
            };

            if !senders.is_empty() {
                // Try most-recent first
                for sender in senders.into_iter().rev() {
                    if sender.clone().send(body.clone()).await.is_ok() {
                        info!("OpenChat push accepted by GUI client");
                        return Ok(());
                    }
                }

                // No GUI accepted the push - spawn GUI as fallback
                warn!("No GUI accepted OpenChat push; launching GUI as fallback");
            }

            // No GUI clients connected or none accepted the push - spawn GUI
            handle_open_chat(message.as_deref(), session_id.as_deref()).await
        }
        ServerToClientRequest::FocusChat => handle_focus_chat(gui_pushers).await,
    }
}

async fn handle_gui_connection_with_initial(
    stream: UnixStream,
    server_socket: &str,
    gui_pushers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>,
    initial_req: Option<Vec<u8>>,
) -> Result<()> {
    info!("Handling new GUI connection, connecting to server...");
    let mut server_stream = connect_to_server(server_socket).await?;
    info!("Connected to server, starting proxy loop");

    // Create a dedicated outgoing queue for this GUI connection and register it
    let (push_tx, mut push_rx) = mpsc::channel::<Vec<u8>>(32);
    {
        let mut guard = gui_pushers.lock().await;
        guard.push(push_tx.clone());
        debug!("Registered GUI pusher, total clients: {}", guard.len());
    }

    // Split GUI stream into read/write halves so writer task can own writer
    let (mut gui_reader, mut gui_writer) = tokio::io::split(stream);

    // Spawn writer task that serializes all outgoing writes to the GUI to avoid concurrent writes
    let writer_handle = tokio::spawn(async move {
        while let Some(body) = push_rx.recv().await {
            let len_bytes = (body.len() as u32).to_be_bytes();
            if let Err(e) = gui_writer.write_all(&len_bytes).await {
                error!("Failed to write length to GUI: {}", e);
                break;
            }
            if let Err(e) = gui_writer.write_all(&body).await {
                error!("Failed to write body to GUI: {}", e);
                break;
            }
            let _ = gui_writer.flush().await;
        }
        debug!("GUI writer task exiting");
    });

    // If an initial request was already read from socket, process it first
    if let Some(initial) = initial_req {
        info!("Processing initial GUI request read during accept");
        let is_send_message = postcard::from_bytes::<ritsu_common::protocol::ClientRequest>(
            &initial,
        )
        .is_ok_and(|req| {
            matches!(
                req,
                ritsu_common::protocol::ClientRequest::SendMessage { .. }
            )
        });

        // Proxy initial request to server
        let len_bytes = (initial.len() as u32).to_be_bytes();
        server_stream.write_all(&len_bytes).await?;
        server_stream.write_all(&initial).await?;
        server_stream.flush().await?;

        if is_send_message {
            // Forward streaming responses for this initial SendMessage
            info!("Proxying streaming responses for initial SendMessage...");
            loop {
                let mut len_buf = [0u8; 4];
                if server_stream.read_exact(&mut len_buf).await.is_err() {
                    error!("Server disconnected during streaming");
                    break;
                }
                let len = u32::from_be_bytes(len_buf) as usize;
                let mut buf = vec![0u8; len];
                server_stream.read_exact(&mut buf).await?;

                if push_tx.send(buf.clone()).await.is_err() {
                    error!("Failed to forward push to GUI writer (channel closed)");
                    break;
                }

                if let Ok(ritsu_common::protocol::ServerPush::MessageChunk {
                    is_final: true, ..
                }) = postcard::from_bytes::<ritsu_common::protocol::ServerPush>(&buf)
                {
                    info!("Final chunk received for initial request, streaming complete");
                    // Try to forward final ServerResponse if any
                    use std::time::Duration;
                    let mut resp_len_buf = [0u8; 4];
                    if tokio::time::timeout(
                        Duration::from_secs(2),
                        server_stream.read_exact(&mut resp_len_buf),
                    )
                    .await
                    .is_ok()
                    {
                        let resp_len = u32::from_be_bytes(resp_len_buf) as usize;
                        if resp_len <= 10_000_000 {
                            let mut resp_data = vec![0u8; resp_len];
                            if tokio::time::timeout(
                                Duration::from_secs(2),
                                server_stream.read_exact(&mut resp_data),
                            )
                            .await
                            .is_ok()
                            && push_tx.send(resp_data).await.is_ok()
                            {
                                info!(
                                    "Forwarded final ServerResponse to GUI for initial request"
                                );
                            }
                        }
                    }
                    break;
                }
            }
        } else {
            // Wait for server response and forward
            let mut len_buf = [0u8; 4];
            server_stream.read_exact(&mut len_buf).await?;
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            server_stream.read_exact(&mut buf).await?;
            if push_tx.send(buf).await.is_err() {
                error!("Failed to forward server response to GUI (channel closed)");
            }
        }
    }

    loop {
        // Read request from GUI (using big-endian to match IpcClient)
        let mut len_buf = [0u8; 4];
        if gui_reader.read_exact(&mut len_buf).await.is_err() {
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
        gui_reader.read_exact(&mut buf).await?;

        // Check if this is a SendMessage request (needs streaming support)
        let is_send_message = postcard::from_bytes::<ritsu_common::protocol::ClientRequest>(&buf)
            .is_ok_and(|req| {
                matches!(
                    req,
                    ritsu_common::protocol::ClientRequest::SendMessage { .. }
                )
            });

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

                // Forward to GUI via push channel
                if push_tx.send(buf.clone()).await.is_err() {
                    error!("Failed to forward push to GUI writer (channel closed)");
                    break;
                }

                // Check if this is the final chunk
                if let Ok(ritsu_common::protocol::ServerPush::MessageChunk {
                    is_final: true, ..
                }) = postcard::from_bytes::<ritsu_common::protocol::ServerPush>(&buf)
                {
                    info!("Final chunk received, streaming complete");

                    // Attempt to read and forward a final ServerResponse (if present) with a short timeout
                    {
                        use std::time::Duration;
                        let mut resp_len_buf = [0u8; 4];
                        if tokio::time::timeout(
                            Duration::from_secs(2),
                            server_stream.read_exact(&mut resp_len_buf),
                        )
                        .await
                        .is_ok()
                        {
                            let resp_len = u32::from_be_bytes(resp_len_buf) as usize;
                            if resp_len <= 10_000_000 {
                                let mut resp_data = vec![0u8; resp_len];
                                if tokio::time::timeout(
                                    Duration::from_secs(2),
                                    server_stream.read_exact(&mut resp_data),
                                )
                                .await
                                .is_ok()
                                {
                                    // Forward to GUI via push channel
                                    if push_tx.send(resp_data).await.is_ok() {
                                        info!("Forwarded final ServerResponse to GUI");
                                    }
                                }
                            }
                        } else {
                            info!("No final ServerResponse or timed out after final chunk");
                        }
                    }

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

            // Forward to GUI via push channel
            if push_tx.send(buf).await.is_err() {
                error!("Failed to forward server response to GUI (channel closed)");
                break;
            }

            info!("Response forwarded successfully");
        }
    }

    // Writer task will exit when push_tx is dropped; ensure it is dropped and wait for writer
    drop(push_tx);
    let _ = writer_handle.await;

    Ok(())
}

async fn handle_incoming_server_connection(
    mut stream: UnixStream,
    initial_body: Option<Vec<u8>>,
    gui_pushers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>,
) -> Result<()> {
    info!("Handling incoming server->client connection");

    // If there's an initial body, handle it first
    if let Some(body) = initial_body {
        match postcard::from_bytes::<ServerToClientRequest>(&body) {
            Ok(req) => {
                let res = match handle_tool_request(req, gui_pushers.clone()).await {
                    Ok(()) => ClientToServerResponse::Success,
                    Err(e) => ClientToServerResponse::Error {
                        message: e.to_string(),
                    },
                };
                let res_bytes = postcard::to_allocvec(&res)?;
                let len_bytes = (res_bytes.len() as u32).to_be_bytes();
                stream.write_all(&len_bytes).await?;
                stream.write_all(&res_bytes).await?;
                stream.flush().await?;
            }
            Err(e) => {
                warn!("Initial payload on incoming server connection not parseable as ServerToClientRequest: {}", e);
            }
        }
    }

    loop {
        // Read request frame
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            info!("Incoming server connection closed");
            return Ok(());
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 10_000_000 {
            warn!("Incoming server connection sent too-large message: {}", len);
            return Ok(());
        }
        let mut buf = vec![0u8; len];
        if stream.read_exact(&mut buf).await.is_err() {
            info!("Incoming server connection closed during read");
            return Ok(());
        }

        let request = match postcard::from_bytes::<ServerToClientRequest>(&buf) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "Failed to parse ServerToClientRequest on accepted connection: {}",
                    e
                );
                continue;
            }
        };

        let response = match handle_tool_request(request, gui_pushers.clone()).await {
            Ok(()) => ClientToServerResponse::Success,
            Err(e) => ClientToServerResponse::Error {
                message: e.to_string(),
            },
        };

        let response_bytes = postcard::to_allocvec(&response)?;
        let response_len_bytes = (response_bytes.len() as u32).to_be_bytes();
        if stream.write_all(&response_len_bytes).await.is_err()
            || stream.write_all(&response_bytes).await.is_err()
            || stream.flush().await.is_err()
        {
            warn!("Failed writing response to incoming server connection");
            return Ok(());
        }
    }
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
        tracing::info!("[NOTIFICATION] {}: {}", title, message);
    }

    Ok(())
}

async fn handle_open_chat(message: Option<&str>, session_id: Option<&str>) -> Result<()> {
    info!("Opening chat GUI");

    // If a chat GUI is already running, prefer focusing it instead of spawning another instance.
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("pgrep").arg("-f").arg("ritsu chat").output() {
            if output.status.success() && !output.stdout.is_empty() {
                info!("Chat GUI already running; attempting to focus instead of spawning a new instance");
                // Try xdotool to focus existing window if available
                if let Ok(xout) = std::process::Command::new("xdotool").args(["search", "--name", "Ritsu", "windowactivate"]).output() {
                    if xout.status.success() {
                        info!("Focused existing chat window via xdotool");
                    } else {
                        warn!("xdotool failed to focus existing window");
                    }
                    return Ok(());
                }
                info!("xdotool not available; not spawning new chat since one is already running");
                return Ok(());
            }
        }
    }

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

async fn handle_focus_chat(gui_pushers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>) -> Result<()> {
    info!("Attempting to focus chat GUI via GUI push channel");

    // Prepare ServerPush::OpenChat payload
    let push = ServerPush::OpenChat {
        message: None,
        urgency: NotificationUrgency::Normal,
    };
    let body = postcard::to_allocvec(&push)?;

    // Snapshot current pushers without holding lock during send
    let senders = {
        let guard = gui_pushers.lock().await;
        guard.clone()
    };

    if senders.is_empty() {
        info!(
            "No GUI clients connected via client daemon, falling back to launching GUI or xdotool"
        );

        // Fallback: try to open chat if no GUI connected
        #[cfg(target_os = "linux")]
        {
            // Try launching GUI if not present
            if let Err(e) = handle_open_chat(None, None).await {
                warn!("Failed to launch GUI fallback: {}", e);
            }
        }

        // Try xdotool as a last resort on Linux
        #[cfg(target_os = "linux")]
        {
            let output = Command::new("xdotool")
                .args(["search", "--name", "Ritsu", "windowactivate"])
                .output();

            match output {
                Ok(output) if output.status.success() => {
                    info!("Successfully focused chat window via xdotool");
                    return Ok(());
                }
                Ok(_) => warn!("xdotool failed to find/focus window"),
                Err(e) => warn!("xdotool not available: {}", e),
            }
        }

        warn!("Window focus not supported on this platform or window not found");
        return Ok(());
    }

    // Prefer a single GUI client: try the most recently registered pusher (LIFO).
    let mut guard = gui_pushers.lock().await;
    while let Some(sender) = guard.pop() {
        match sender.clone().send(body.clone()).await {
            Ok(()) => {
                info!("Focus push accepted by one GUI client");
                return Ok(());
            }
            Err(e) => {
                warn!("Failed to send focus push to GUI (removing pusher): {}", e);
            }
        }
    }

    // No GUI accepted the push; fallback to launching/xdotool
    warn!("No GUI accepted focus push; falling back to launch/xdotool");
    #[cfg(target_os = "linux")]
    {
        let _ = handle_open_chat(None, None).await;
    }

    Ok(())
}

pub async fn stop() -> Result<()> {
    tracing::info!("Stopping client daemon...");

    let config = crate::config::ClientConfig::load()?;
    let client_socket = config.client_socket_path();

    // Check if daemon is running by checking socket
    if !Path::new(&client_socket).exists() {
        tracing::info!("Client daemon is not running");
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
                let _ = Command::new("kill").arg(pid_num.to_string()).status();
                tracing::info!("Stopped client daemon (PID: {})", pid_num);
            }
        }
    } else {
        tracing::info!("Client daemon process not found");
    }

    // Clean up socket in both cases
    let _ = std::fs::remove_file(&client_socket);

    Ok(())
}

pub async fn status() -> Result<()> {
    let config = crate::config::ClientConfig::load()?;
    let client_socket = config.client_socket_path();

    if Path::new(&client_socket).exists() {
        // Try to connect to verify it's actually running
        match UnixStream::connect(&client_socket).await {
            Ok(_) => {
                tracing::info!("Client daemon: Running");
                tracing::info!("Socket: {}", client_socket);
            }
            Err(_) => {
                tracing::info!("Client daemon: Socket exists but not responding (stale?)");
            }
        }
    } else {
        tracing::info!("Client daemon: Not running");
    }

    Ok(())
}

pub async fn restart() -> Result<()> {
    tracing::info!("Restarting client daemon...");
    stop().await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Spawn as background process
    Command::new("ritsu")
        .arg("start")
        .spawn()
        .context("Failed to spawn client daemon")?;

    tracing::info!("Client daemon restarting in background");
    Ok(())
}
