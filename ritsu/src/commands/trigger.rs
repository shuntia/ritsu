//! Trigger management commands

use anyhow::Result;

use crate::TriggerCommands;

pub fn handle(cmd: TriggerCommands) -> Result<()> {
    match cmd {
        TriggerCommands::List => list(),
        TriggerCommands::Add { name, time } => add(&name, &time),
        TriggerCommands::Disable { name } => disable(&name),
        TriggerCommands::Delete { name } => delete(&name),
    }
}

fn list() -> Result<()> {
    println!("Listing triggers...");
    println!("No triggers (placeholder)");
    Ok(())
}

fn add(name: &str, time: &str) -> Result<()> {
    println!("Adding trigger '{name}' with schedule '{time}'");
    Ok(())
}

fn disable(name: &str) -> Result<()> {
    println!("Disabling trigger '{name}'");
    Ok(())
}

fn delete(name: &str) -> Result<()> {
    println!("Deleting trigger '{name}'");
    Ok(())
}
