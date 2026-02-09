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
    let client = crate::ipc::IpcClient::new(super::get_client_socket()?);
    
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
            tracing::error!("Error: {}", message);
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}

async fn create_trigger(name: &str, time: &str) -> Result<()> {
    let client = crate::ipc::IpcClient::new(super::get_client_socket()?);
    
    let response = client
        .send_request(ritsu_common::protocol::ClientRequest::CreateTrigger {
            name: name.to_string(),
            trigger_type: "time".to_string(),
            schedule: time.to_string(),
            tag: None,
            description: None,
        })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Ok => {
            println!("{} Trigger created: {name}", nerd_font::categories::Fa::Check);
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            tracing::error!("Error: {message}");
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}

async fn disable_trigger(name: &str) -> Result<()> {
    let client = crate::ipc::IpcClient::new(super::get_client_socket()?);
    
    let response = client
        .send_request(ritsu_common::protocol::ClientRequest::DisableTrigger {
            name: name.to_string(),
        })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Ok => {
            println!("{} Trigger disabled: {name}", nerd_font::categories::Fa::Check);
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            tracing::error!("Error: {message}");
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}

async fn delete_trigger(name: &str) -> Result<()> {
    let client = crate::ipc::IpcClient::new(super::get_client_socket()?);
    
    let response = client
        .send_request(ritsu_common::protocol::ClientRequest::DeleteTrigger {
            name: name.to_string(),
        })
        .await?;

    match response {
        ritsu_common::protocol::ServerResponse::Ok => {
            println!("{} Trigger deleted: {name}", nerd_font::categories::Fa::Check);
            Ok(())
        }
        ritsu_common::protocol::ServerResponse::Error { message } => {
            tracing::error!("Error: {message}");
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("Unexpected response"),
    }
}
