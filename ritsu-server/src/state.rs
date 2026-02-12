//! Server state management

use chrono::{DateTime, Utc};
use ritsu_common::protocol::{ClientToServerResponse, ServerPush, ServerToClientRequest};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, warn};

/// Push notification channel capacity
pub const PUSH_CHANNEL_CAPACITY: usize = 100;

/// Global server state
pub struct ServerState {
    pub last_activity: Arc<RwLock<DateTime<Utc>>>,
    /// Connected clients for push notifications
    clients: Arc<RwLock<Vec<mpsc::Sender<ServerPush>>>>,
    /// Connection to client daemon for tool requests
    client_daemon: Arc<Mutex<Option<UnixStream>>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            last_activity: Arc::new(RwLock::new(Utc::now())),
            clients: Arc::new(RwLock::new(Vec::new())),
            client_daemon: Arc::new(Mutex::new(None)),
        }
    }

    /// Update last activity timestamp
    pub async fn mark_activity(&self) {
        let mut last = self.last_activity.write().await;
        *last = Utc::now();
    }

    /// Register a new client for push notifications
    pub async fn register_client(&self, sender: mpsc::Sender<ServerPush>) {
        let mut clients = self.clients.write().await;
        clients.push(sender);
    }

    /// Returns true if any GUI clients are currently connected to the daemon
    pub async fn register_client(&self, sender: mpsc::Sender<ServerPush>) {
        let mut clients = self.clients.write().await;
        clients.push(sender);
    }

    /// Returns true if any GUI clients are currently connected to the daemon
    pub async fn has_gui_clients(&self) -> bool {
        let clients = self.clients.read().await;
        !clients.is_empty()
    }

    /// Broadcast a push notification to all connected clients
    pub async fn broadcast_push(&self, push: ServerPush) {
        let mut clients = self.clients.write().await;
        // Use try_send to avoid blocking on slow clients
        clients.retain(|client| client.try_send(push.clone()).is_ok());
    }

    /// Spawns a background task that attempts to keep a persistent
    /// connection to the client daemon. It will reconnect with a
    /// quadratic backoff and small jitter if the connection is lost.
    pub fn start_client_daemon_reconnector(self: Arc<Self>) {
        tokio::spawn(async move {
            let socket_path = std::env::var("RITSU_CLIENT_SOCKET")
                .unwrap_or_else(|_| "/tmp/ritsu-client.sock".to_string());

            let mut attempt: u64 = 0;

            loop {
                // If we already have a connection, wait and re-check
                {
                    let guard = self.client_daemon.lock().await;
                    if guard.is_some() {
                        drop(guard);
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue;
                    }
                }

                match UnixStream::connect(&socket_path).await {
                    Ok(stream) => {
                        attempt = 0;
                        let mut guard = self.client_daemon.lock().await;
                        *guard = Some(stream);
                        tracing::info!("Connected to client daemon at {}", socket_path);

                        // Give it some time; if connection dies, send_to_client_daemon will clear it
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                    Err(e) => {
                        attempt = attempt.saturating_add(1);
                        // Quadratic backoff (ms) with cap of 5 minutes
                        let base_ms = 500u64;
                        let mut delay_ms = base_ms.saturating_mul(attempt.saturating_mul(attempt));
                        if delay_ms > 5 * 60 * 1000 {
                            delay_ms = 5 * 60 * 1000;
                        }
                        // Small pseudo-random jitter based on current time
                        let jitter =
                            ((chrono::Utc::now().timestamp() as u64).wrapping_mul(997)) % 1000;
                        let delay = std::time::Duration::from_millis(delay_ms + jitter);
                        tracing::warn!(
                            "Failed to connect to client daemon (attempt {}): {}. Retrying in {:?}",
                            attempt,
                            e,
                            delay
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        });
    }

    /// Send a request to the client daemon
    ///
    /// Prefers a long-lived background connection maintained by
    /// `start_client_daemon_reconnector()`. If no persistent connection
    /// is available, falls back to a short per-request connect with a
    /// short timeout.
    pub async fn send_to_client_daemon(
        &self,
        request: ServerToClientRequest,
    ) -> anyhow::Result<()> {
        // Attempt to use an existing persistent connection if present. Take the stream out of the mutex
        // temporarily to avoid holding the lock across I/O; reinsert it after I/O completes.
        {
            let mut guard = self.client_daemon.lock().await;
            if let Some(mut stream) = guard.take() {
                // We have a persistent stream. Drop the lock while performing I/O to avoid holding the mutex across awaits.
                drop(guard);

                // Serialize request
                let request_bytes = postcard::to_allocvec(&request)?;
                let len_bytes = (request_bytes.len() as u32).to_be_bytes();

                let write_timeout = std::time::Duration::from_secs(5);
                // Write with timeout to avoid blocking forever
                if tokio::time::timeout(write_timeout, stream.write_all(&len_bytes)).await.is_err() {
                    warn!("Timeout writing to client daemon (persistent)");
                    // Do not reinsert the stream; let the reconnector re-establish
                    return Err(anyhow::anyhow!("Failed to send to client daemon (timeout)"));
                }
                if let Err(e) = stream.write_all(&request_bytes).await {
                    error!("Failed to send to client daemon (persistent): {}", e);
                    return Err(anyhow::anyhow!("Failed to send to client daemon"));
                }
                if tokio::time::timeout(write_timeout, stream.flush()).await.is_err() {
                    warn!("Timeout flushing to client daemon (persistent)");
                    return Err(anyhow::anyhow!("Failed to flush to client daemon"));
                }

                // Read response with timeout
                let read_timeout = std::time::Duration::from_secs(5);
                let mut len_buf = [0u8; 4];
                if tokio::time::timeout(read_timeout, stream.read_exact(&mut len_buf)).await.is_err() {
                    warn!("Timeout reading response from client daemon (persistent)");
                    return Err(anyhow::anyhow!("Failed to read response"));
                }
                let len = u32::from_be_bytes(len_buf) as usize;
                let mut buf = vec![0u8; len];
                if tokio::time::timeout(read_timeout, stream.read_exact(&mut buf)).await.is_err() {
                    warn!("Timeout reading response from client daemon (persistent)");
                    return Err(anyhow::anyhow!("Failed to read response"));
                }

                let response: ClientToServerResponse = postcard::from_bytes(&buf)?;

                // Reinsert the stream for persistent use
                let mut guard = self.client_daemon.lock().await;
                *guard = Some(stream);

                return match response {
                    ClientToServerResponse::Success => Ok(()),
                    ClientToServerResponse::Error { message } => {
                        Err(anyhow::anyhow!("Client daemon error: {message}"))
                    }
                };
            }
        }

        // Fallback: connect per-request to avoid blocking if persistent connection missing
        let socket_path = std::env::var("RITSU_CLIENT_SOCKET")
            .unwrap_or_else(|_| "/tmp/ritsu-client.sock".to_string());
        // Use a short connect timeout to avoid hanging the server
        let connect_timeout = std::time::Duration::from_secs(5);
        let stream =
            match tokio::time::timeout(connect_timeout, UnixStream::connect(&socket_path)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    warn!("Client daemon not available: {}", e);
                    return Err(anyhow::anyhow!("Client daemon not running"));
                }
                Err(_) => {
                    warn!("Timeout connecting to client daemon at {}", socket_path);
                    return Err(anyhow::anyhow!("Client daemon connection timed out"));
                }
            };

        // Serialize and send request
        let request_bytes = postcard::to_allocvec(&request)?;
        let len_bytes = (request_bytes.len() as u32).to_be_bytes();
        let mut stream = stream;

        if let Err(e) = stream.write_all(&len_bytes).await {
            error!("Failed to send to client daemon: {}", e);
            return Err(anyhow::anyhow!("Failed to send to client daemon"));
        }

        if let Err(e) = stream.write_all(&request_bytes).await {
            error!("Failed to send to client daemon: {}", e);
            return Err(anyhow::anyhow!("Failed to send to client daemon"));
        }

        if let Err(e) = stream.flush().await {
            error!("Failed to flush to client daemon: {}", e);
            return Err(anyhow::anyhow!("Failed to flush to client daemon"));
        }

        // Read response
        let mut len_buf = [0u8; 4];
        if let Err(e) = stream.read_exact(&mut len_buf).await {
            error!("Failed to read response from client daemon: {}", e);
            return Err(anyhow::anyhow!("Failed to read response"));
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];

        if let Err(e) = stream.read_exact(&mut buf).await {
            error!("Failed to read response from client daemon: {}", e);
            return Err(anyhow::anyhow!("Failed to read response"));
        }

        let response: ClientToServerResponse = postcard::from_bytes(&buf)?;

        match response {
            ClientToServerResponse::Success => Ok(()),
            ClientToServerResponse::Error { message } => {
                Err(anyhow::anyhow!("Client daemon error: {message}"))
            }
        }
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_activity_tracking() {
        let state = ServerState::new();

        // Initial state should have ~0 seconds of inactivity
        {
            let last = state.last_activity.read().await;
            let initial = (Utc::now() - *last).num_seconds();
            assert!(initial < 1);
        }

        // Wait a bit
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Should have some inactivity now
        {
            let last = state.last_activity.read().await;
            let after_wait = (Utc::now() - *last).num_seconds();
            assert!(after_wait >= 0);
        }

        // Mark activity
        state.mark_activity().await;

        // Should be fresh again
        {
            let last = state.last_activity.read().await;
            let after_mark = (Utc::now() - *last).num_seconds();
            assert!(after_mark < 1);
        }
    }

    #[tokio::test]
    async fn test_inactivity_detection() {
        let state = ServerState::new();

        // Should not be inactive for 10 seconds yet
        {
            let last = state.last_activity.read().await;
            let inactive = (Utc::now() - *last).num_seconds() >= 10;
            assert!(!inactive);
        }

        // Mark activity
        state.mark_activity().await;

        // Should still not be inactive for any meaningful duration
        {
            let last = state.last_activity.read().await;
            let inactive = (Utc::now() - *last).num_seconds() >= 1;
            assert!(!inactive);
        }
    }
}
