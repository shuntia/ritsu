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
}
