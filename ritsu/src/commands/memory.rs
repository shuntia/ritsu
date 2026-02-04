//! Memory and notes commands

use anyhow::Result;
use ritsu_common::protocol::{ClientRequest, MemoryQueryType};

pub async fn query_memory(days: Option<u32>) -> Result<()> {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    
    let _days = days.unwrap_or(7);
    let response = client
        .send_request(ClientRequest::QueryMemory {
            query_type: MemoryQueryType::Recent,
            date_range: None,
        })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Memory { content } => {
            println!("{}", content);
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            eprintln!("Error: {}", message);
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}

pub async fn query_notes(_tag: Option<&str>) -> Result<()> {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    
    let response = client
        .send_request(ClientRequest::QueryMemory {
            query_type: MemoryQueryType::Notes,
            date_range: None,
        })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Memory { content } => {
            println!("{}", content);
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            eprintln!("Error: {}", message);
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}
