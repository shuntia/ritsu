//! IPC client - Unix domain socket communication

use anyhow::Result;
use ritsu_common::protocol::{ClientRequest, ServerResponse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub struct IpcClient {
    socket_path: String,
}

impl IpcClient {
    pub fn new(socket_path: String) -> Self {
        Self { socket_path }
    }

    pub async fn send_request(&self, request: ClientRequest) -> Result<ServerResponse> {
        let mut stream = UnixStream::connect(&self.socket_path).await?;

        // Serialize request
        let request_data = postcard::to_allocvec(&request)?;
        let request_len = (request_data.len() as u32).to_be_bytes();

        // Send request
        stream.write_all(&request_len).await?;
        stream.write_all(&request_data).await?;
        stream.flush().await?;

        // Read response length
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        // Read response data
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).await?;

        // Deserialize response
        let response: ServerResponse = postcard::from_bytes(&data)?;
        Ok(response)
    }

    pub async fn ping(&self) -> Result<bool> {
        match self.send_request(ClientRequest::Ping).await? {
            ServerResponse::Pong => Ok(true),
            _ => Ok(false),
        }
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
        loop {
            // Read push length
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len = u32::from_be_bytes(len_buf) as usize;
            
            // Read push data
            let mut data = vec![0u8; len];
            stream.read_exact(&mut data).await?;
            
            // Deserialize push
            let push: ritsu_common::protocol::ServerPush = postcard::from_bytes(&data)?;
            return Ok(push);
        }
    }
}

#[cfg(test)]
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
            _ => panic!("Unexpected request type"),
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
            ClientRequest::SendMessage { content, session_id } => {
                assert_eq!(content, "test message");
                assert_eq!(session_id, Some("session123".to_string()));
            }
            _ => panic!("Unexpected request type"),
        }
    }
}
