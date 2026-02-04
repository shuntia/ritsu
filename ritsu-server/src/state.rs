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
