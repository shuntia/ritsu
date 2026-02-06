//! Memory and notes commands

use anyhow::Result;
use ritsu_common::protocol::{ClientRequest, MemoryQueryType};

pub async fn query_memory(days: Option<u32>) -> Result<()> {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
    
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
    let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
    
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

pub async fn clear_memory(noconfirm: bool) -> Result<()> {
    if !noconfirm {
        println!("⚠️  WARNING: This will permanently delete ALL memory data:");
        println!("   - All conversation sessions and turns");
        println!("   - All daily conversations");
        println!("   - All daily and monthly summaries");
        println!("   - All notes");
        println!("   - All tool usage statistics");
        println!("   - All idle analyses");
        println!();
        print!("Type 'yes' to confirm: ");
        
        use std::io::{self, Write};
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        if input.trim() != "yes" {
            println!("Cancelled.");
            return Ok(());
        }
    }
    
    let client = crate::ipc::IpcClient::new("/tmp/ritsu-client.sock".to_string());
    
    println!("Clearing all memory...");
    let response = client
        .send_request(ClientRequest::ClearMemory { confirm: true })
        .await?;
    
    match response {
        ritsu_common::protocol::ServerResponse::Ok => {
            println!("✓ All memory cleared successfully");
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            eprintln!("✗ Error: {}", message);
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}
