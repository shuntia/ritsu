//! Server state management

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use chrono::{DateTime, Utc};
use ritsu_common::protocol::ServerPush;

/// Global server state
pub struct ServerState {
    pub last_activity: Arc<RwLock<DateTime<Utc>>>,
    /// Connected clients for push notifications
    clients: Arc<RwLock<Vec<mpsc::UnboundedSender<ServerPush>>>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            last_activity: Arc::new(RwLock::new(Utc::now())),
            clients: Arc::new(RwLock::new(Vec::new())),
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
