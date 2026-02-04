//! Memory query commands

use anyhow::Result;


pub fn query_memory(days: u32) -> Result<()> {
    println!("Querying memory for last {days} days...");
    println!("No recent memory (placeholder)");
    Ok(())
}

pub fn query_notes(tag: Option<&str>) -> Result<()> {
    println!("Querying notes with tag filter: {tag:?}");
    println!("No notes (placeholder)");
    Ok(())
}
