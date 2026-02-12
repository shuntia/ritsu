//! IPC client - Unix domain socket communication

use anyhow::Result;
use ritsu_common::protocol::{ClientRequest, ServerPush, ServerResponse};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, Mutex};

#[derive(Clone)]
pub struct IpcClient {
    socket_path: String,
    connection: Arc<Mutex<Option<UnixStream>>>,
}

impl IpcClient {
    pub fn new(socket_path: String) -> Self {
        Self {
            socket_path,
            connection: Arc::new(Mutex::new(None)),
        }
    }

    /// Get or establish connection to the daemon without holding the lock across await.
    async fn get_connection(&self) -> Result<tokio::sync::MutexGuard<'_, Option<UnixStream>>> {
        // Fast path: if a connection exists, return it while holding the lock.
        {
            let guard = self.connection.lock().await;
            if guard.is_some() {
                return Ok(guard);
            }
            // Drop the guard and create connection outside of the lock
        }

        // Establish a new connection without holding the mutex
        let stream = UnixStream::connect(&self.socket_path).await?;

        // Insert the new stream under the lock
        let mut guard = self.connection.lock().await;
        if guard.is_none() {
            *guard = Some(stream);
        }
        Ok(guard)
    }

    pub async fn send_request(&self, request: ClientRequest) -> Result<ServerResponse> {
        // Pre-serialize the request before taking the stream
        let request_data = postcard::to_allocvec(&request)?;
        let request_len = (request_data.len() as u32).to_be_bytes();

        // Ensure a connection exists
        let mut guard = self.get_connection().await?;
        // Take ownership of the stream so we don't hold the mutex across awaits
        let mut stream = guard
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to establish connection"))?;
        drop(guard);

        use std::time::Duration;
        let write_timeout = Duration::from_secs(5);
        // Write length
        match tokio::time::timeout(write_timeout, stream.write_all(&request_len)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(anyhow::anyhow!("Timeout writing to IPC socket")),
        }
        // Write data
        match tokio::time::timeout(write_timeout, stream.write_all(&request_data)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(anyhow::anyhow!("Timeout writing to IPC socket")),
        }
        // Flush
        match tokio::time::timeout(write_timeout, stream.flush()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(anyhow::anyhow!("Timeout flushing IPC socket")),
        }

        // Read response length
        let read_timeout = Duration::from_secs(5);
        let mut len_buf = [0u8; 4];
        match tokio::time::timeout(read_timeout, stream.read_exact(&mut len_buf)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Timeout reading response length from IPC socket"
                ))
            }
        }
        let len = u32::from_be_bytes(len_buf) as usize;

        // Read response data
        let mut data = vec![0u8; len];
        match tokio::time::timeout(read_timeout, stream.read_exact(&mut data)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(anyhow::anyhow!("Timeout reading response from IPC socket")),
        }

        // Deserialize response
        let response: ServerResponse = postcard::from_bytes(&data)?;

        // Reinsert the stream for reuse
        let mut guard = self.connection.lock().await;
        *guard = Some(stream);

        Ok(response)
    }

    pub async fn ping(&self) -> Result<bool> {
        match self.send_request(ClientRequest::Ping).await? {
            ServerResponse::Pong => Ok(true),
            _ => Ok(false),
        }
    }

    /// Get conversation history for a session
    pub async fn get_conversation_history(
        &self,
        session_id: String,
        limit: i64,
    ) -> Result<Vec<ritsu_common::protocol::ConversationTurn>> {
        let request = ClientRequest::GetConversationHistory { session_id, limit };
        match self.send_request(request).await? {
            ServerResponse::ConversationHistory { turns } => Ok(turns),
            ServerResponse::Error { message } => anyhow::bail!(message),
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    /// Send a message and subscribe to streaming responses
    /// Returns a channel receiver that yields message chunks
    /// Note: This creates a dedicated connection for streaming since it's long-lived
    pub async fn send_message_streaming(
        &self,
        content: String,
        session_id: Option<String>,
    ) -> Result<mpsc::Receiver<ServerPush>> {
        // For streaming, create a dedicated connection since it's long-lived
        // and we can't hold the main connection lock for the duration
        let mut stream = UnixStream::connect(&self.socket_path).await?;

        // Send message request
        let request = ClientRequest::SendMessage {
            content,
            session_id,
        };
        let request_data = postcard::to_allocvec(&request)?;
        let request_len = (request_data.len() as u32).to_be_bytes();

        stream.write_all(&request_len).await?;
        stream.write_all(&request_data).await?;
        stream.flush().await?;

        // Create channel for streaming chunks
        let (tx, rx) = mpsc::channel(32);

        // Spawn a task to read push notifications from the stream
        tokio::spawn(async move {
            loop {
                // Read push length
                let mut len_buf = [0u8; 4];
                if stream.read_exact(&mut len_buf).await.is_err() {
                    break;
                }
                let len = u32::from_be_bytes(len_buf) as usize;

                // Read push data
                let mut data = vec![0u8; len];
                if stream.read_exact(&mut data).await.is_err() {
                    break;
                }

                // Deserialize push
                if let Ok(push) = postcard::from_bytes::<ServerPush>(&data) {
                    // Check if this is the final chunk
                    let is_final = matches!(push, ServerPush::MessageChunk { is_final: true, .. });

                    if tx.send(push).await.is_err() {
                        break;
                    }

                    if is_final {
                        // After the final push, attempt to read the server's final ServerResponse (if any)
                        // Use a short timeout to avoid blocking indefinitely if the server doesn't send one.
                        use std::time::Duration;
                        let mut resp_len_buf = [0u8; 4];
                        if tokio::time::timeout(
                            Duration::from_secs(2),
                            stream.read_exact(&mut resp_len_buf),
                        )
                        .await
                        .is_ok()
                        {
                            let resp_len = u32::from_be_bytes(resp_len_buf) as usize;
                            let mut resp_data = vec![0u8; resp_len];
                            // Try to read and parse the ServerResponse; ignore errors/timeouts
                            let _ = tokio::time::timeout(
                                Duration::from_secs(2),
                                stream.read_exact(&mut resp_data),
                            )
                            .await;
                            let _ = postcard::from_bytes::<ServerResponse>(&resp_data);
                        }
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        Ok(rx)
    }

    /// Subscribe to server pushes and wait for next push notification
    pub async fn subscribe_and_wait(&self) -> Result<ritsu_common::protocol::ServerPush> {
        let mut stream = UnixStream::connect(&self.socket_path).await?;

        // Send subscribe request
        let request = ClientRequest::Subscribe;
        let request_data = postcard::to_allocvec(&request)?;
        let request_len = (request_data.len() as u32).to_be_bytes();

        stream.write_all(&request_len).await?;
        stream.write_all(&request_data).await?;
        stream.flush().await?;

        // Read subscription acknowledgment
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).await?;
        let _response: ServerResponse = postcard::from_bytes(&data)?;

        // Now wait for push notifications on this connection
        // Read push length
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        // Read push data
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).await?;

        // Deserialize push
        let push: ritsu_common::protocol::ServerPush = postcard::from_bytes(&data)?;
        Ok(push)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = IpcClient::new("/tmp/test.sock".to_string());
        assert_eq!(client.socket_path, "/tmp/test.sock");
    }

    #[tokio::test]
    async fn test_protocol_serialization() {
        // Test that protocol messages can be serialized/deserialized
        let request = ClientRequest::Ping;
        let serialized = postcard::to_allocvec(&request).unwrap();
        let deserialized: ClientRequest = postcard::from_bytes(&serialized).unwrap();

        match deserialized {
            ClientRequest::Ping => (),
            _ => unreachable!("Unexpected request type"),
        }
    }

    #[tokio::test]
    async fn test_send_message_serialization() {
        let request = ClientRequest::SendMessage {
            content: "test message".to_string(),
            session_id: Some("session123".to_string()),
        };

        let serialized = postcard::to_allocvec(&request).unwrap();
        let deserialized: ClientRequest = postcard::from_bytes(&serialized).unwrap();

        match deserialized {
            ClientRequest::SendMessage {
                content,
                session_id,
            } => {
                assert_eq!(content, "test message");
                assert_eq!(session_id, Some("session123".to_string()));
            }
            _ => unreachable!("Unexpected request type"),
        }
    }
}
