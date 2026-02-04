//! Task management commands

use anyhow::Result;

use crate::TaskCommands;

pub fn handle(cmd: TaskCommands) -> Result<()> {
    match cmd {
        TaskCommands::List { status } => list(status.as_deref()),
        TaskCommands::Add { title, priority, due } => add(&title, &priority, due.as_deref()),
        TaskCommands::Update { id, status } => update(id, &status),
    }
}

fn list(status: Option<&str>) -> Result<()> {
    println!("Listing tasks with status filter: {status:?}");
    println!("No tasks (placeholder)");
    Ok(())
}

fn add(title: &str, priority: &str, due: Option<&str>) -> Result<()> {
    println!("Adding task '{title}' with priority '{priority}' and due date {due:?}");
    Ok(())
}

fn update(id: i64, status: &str) -> Result<()> {
    println!("Updating task {id} to status '{status}'");
    Ok(())
}
