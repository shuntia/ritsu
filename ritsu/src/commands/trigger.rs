//! Trigger management commands

use anyhow::Result;
use crate::TriggerCommands;

pub async fn handle(cmd: TriggerCommands) -> Result<()> {
    match cmd {
        TriggerCommands::List => list_triggers().await,
        TriggerCommands::Add { name, time } => create_trigger(&name, &time).await,
        TriggerCommands::Disable { name } => disable_trigger(&name).await,
        TriggerCommands::Delete { name } => delete_trigger(&name).await,
    }
}

async fn list_triggers() -> Result<()> {
    let client = crate::ipc::IpcClient::new("/tmp/ritsu.sock".to_string());
    
    let response = client
        .send_request(ritsu_common::protocol::ClientRequest::ListTriggers)
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Triggers { triggers } => {
            if triggers.is_empty() {
                println!("No triggers found");
            } else {
                for trigger in triggers {
                    println!("#{}: {} (enabled: {})", trigger.id, trigger.name, trigger.enabled);
                }
            }
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            eprintln!("Error: {}", message);
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}

async fn create_trigger(_name: &str, _time: &str) -> Result<()> {
    println!("TODO: Implement trigger creation");
    Ok(())
}

async fn disable_trigger(_name: &str) -> Result<()> {
    println!("TODO: Implement trigger disable");
    Ok(())
}

async fn delete_trigger(_name: &str) -> Result<()> {
    println!("TODO: Implement trigger deletion");
    Ok(())
}
