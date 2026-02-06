//! Server state management

use std::sync::Arc;
use tokio::sync::{RwLock, Mutex, mpsc};
use tokio::net::UnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use chrono::{DateTime, Utc};
use ritsu_common::protocol::{ServerPush, ServerToClientRequest, ClientToServerResponse};
use tracing::{warn, error};

/// Global server state
pub struct ServerState {
    pub last_activity: Arc<RwLock<DateTime<Utc>>>,
    /// Connected clients for push notifications
    clients: Arc<RwLock<Vec<mpsc::UnboundedSender<ServerPush>>>>,
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

    /// Get seconds since last activity
    pub async fn seconds_since_activity(&self) -> i64 {
        let last = self.last_activity.read().await;
        (Utc::now() - *last).num_seconds()
    }

    /// Check if inactive for given duration
    pub async fn is_inactive_for(&self, seconds: u64) -> bool {
        self.seconds_since_activity().await >= seconds as i64
    }

    /// Register a new client for push notifications
    pub async fn register_client(&self, sender: mpsc::UnboundedSender<ServerPush>) {
        let mut clients = self.clients.write().await;
        clients.push(sender);
    }

    /// Broadcast a push notification to all connected clients
    pub async fn broadcast_push(&self, push: ServerPush) {
        let mut clients = self.clients.write().await;
        clients.retain(|client| client.send(push.clone()).is_ok());
    }

    /// Send a request to the client daemon
    pub async fn send_to_client_daemon(&self, request: ServerToClientRequest) -> anyhow::Result<()> {
        let mut guard = self.client_daemon.lock().await;
        
        // Try to connect if not connected
        if guard.is_none() {
            match UnixStream::connect("/tmp/ritsu-client.sock").await {
                Ok(stream) => {
                    *guard = Some(stream);
                }
                Err(e) => {
                    warn!("Client daemon not available: {}", e);
                    return Err(anyhow::anyhow!("Client daemon not running"));
                }
            }
        }

        let stream = guard.as_mut().ok_or_else(|| anyhow::anyhow!("Client daemon connection lost"))?;

        // Serialize and send request
        let request_bytes = postcard::to_allocvec(&request)?;
        let len_bytes = (request_bytes.len() as u32).to_le_bytes();

        if let Err(e) = stream.write_all(&len_bytes).await {
            error!("Failed to send to client daemon: {}", e);
            *guard = None; // Disconnect
            return Err(anyhow::anyhow!("Failed to send to client daemon"));
        }

        if let Err(e) = stream.write_all(&request_bytes).await {
            error!("Failed to send to client daemon: {}", e);
            *guard = None;
            return Err(anyhow::anyhow!("Failed to send to client daemon"));
        }

        if let Err(e) = stream.flush().await {
            error!("Failed to flush to client daemon: {}", e);
            *guard = None;
            return Err(anyhow::anyhow!("Failed to flush to client daemon"));
        }

        // Read response
        let mut len_buf = [0u8; 4];
        if let Err(e) = stream.read_exact(&mut len_buf).await {
            error!("Failed to read response from client daemon: {}", e);
            *guard = None;
            return Err(anyhow::anyhow!("Failed to read response"));
        }

        let len = u32::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        
        if let Err(e) = stream.read_exact(&mut buf).await {
            error!("Failed to read response from client daemon: {}", e);
            *guard = None;
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
        
        // Initial state should have 0 seconds of inactivity
        let initial = state.seconds_since_activity().await;
        assert!(initial < 1);
        
        // Wait a bit
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        // Should have some inactivity now
        let after_wait = state.seconds_since_activity().await;
        assert!(after_wait >= 0);
        
        // Mark activity
        state.mark_activity().await;
        
        // Should be fresh again
        let after_mark = state.seconds_since_activity().await;
        assert!(after_mark < 1);
    }

    #[tokio::test]
    async fn test_inactivity_detection() {
        let state = ServerState::new();
        
        // Should not be inactive for 10 seconds yet
        assert!(!state.is_inactive_for(10).await);
        
        // Mark activity
        state.mark_activity().await;
        
        // Should still not be inactive for any meaningful duration
        assert!(!state.is_inactive_for(1).await);
    }
}
