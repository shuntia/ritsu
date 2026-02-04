//! Send message command

use anyhow::Result;
use ritsu_common::protocol::ClientRequest;

pub async fn send_message(message: &str) -> Result<()> {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    
    let response = client
        .send_request(ClientRequest::SendMessage {
            content: message.to_string(),
        })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Ok => {
            println!("✓ Message sent");
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            eprintln!("Error: {}", message);
            anyhow::bail!(message)
        }
        _ => {
            eprintln!("Unexpected response");
            anyhow::bail!("Unexpected response")
        }
    }
}
