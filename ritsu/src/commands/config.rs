//! TUI configuration editor for Ritsu using ratatui + crossterm
#![allow(clippy::too_many_lines)]

use anyhow::{Context, Result};
use crossterm::event::{self, Event as CEvent, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::terminal::Frame;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Terminal;
use std::env;
use std::fs;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// Public handler invoked by CLI
pub async fn handle() -> Result<()> {
    run_tui().await.context("running config TUI")
}

fn prompt_entries() -> Vec<(String, PathBuf)> {
    let mut entries = Vec::new();
    // Preferred user config location
    if let Some(home) = dirs::home_dir() {
        let base = home.join(".config/ritsu/prompts");
        entries.push(("Base system prompt".to_string(), base.join("system_base.md")));
        entries.push(("Chat prompt".to_string(), base.join("chat.md")));
        entries.push(("Background prompt".to_string(), base.join("background.md")));
        entries.push(("Compact (daily/monthly) prompt".to_string(), base.join("background/compact.md")));
        entries.push(("Pattern (weekly) prompt".to_string(), base.join("background/pattern.md")));
        entries.push(("Briefing prompt".to_string(), base.join("background/briefing.md")));
        entries.push(("Trigger prompts (directory)".to_string(), base.join("triggers")));
    } else {
        // Fallback to repository-relative path
        let base = PathBuf::from(".config/ritsu/prompts");
        entries.push(("Base system prompt".to_string(), base.join("system_base.md")));
        entries.push(("Chat prompt".to_string(), base.join("chat.md")));
        entries.push(("Background prompt".to_string(), base.join("background.md")));
        entries.push(("Compact (daily/monthly) prompt".to_string(), base.join("background/compact.md")));
        entries.push(("Pattern (weekly) prompt".to_string(), base.join("background/pattern.md")));
        entries.push(("Briefing prompt".to_string(), base.join("background/briefing.md")));
        entries.push(("Trigger prompts (directory)".to_string(), base.join("triggers")));
    }
    entries
}

async fn run_tui() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let items = prompt_entries();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(0));
    }

    loop {
        terminal.draw(|f| ui(f, &items, &mut state))?;

        // Poll for events
        if crossterm::event::poll(Duration::from_millis(200))? {
            match event::read()? {
                CEvent::Key(KeyEvent { code: KeyCode::Char('q'), .. }) => break,
                CEvent::Key(KeyEvent { code: KeyCode::Up, .. }) => {
                    let i = state.selected().unwrap_or(0);
                    if i > 0 {
                        state.select(Some(i - 1));
                    }
                }
                CEvent::Key(KeyEvent { code: KeyCode::Down, .. }) => {
                    let i = state.selected().unwrap_or(0);
                    if i + 1 < items.len() {
                        state.select(Some(i + 1));
                    }
                }
                CEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
                    let idx = state.selected().unwrap_or(0);
                    let path = items[idx].1.clone();

                    // Clean up terminal state before launching editor
                    terminal.show_cursor()?;
                    drop(terminal);
                    disable_raw_mode()?;
                    execute!(io::stdout(), LeaveAlternateScreen)?;

                    // Open editor
                    if let Err(e) = open_editor(&path) {
                        eprintln!("Failed to open editor: {}", e);
                    }

                    // Re-create terminal
                    enable_raw_mode()?;
                    execute!(io::stdout(), EnterAlternateScreen)?;
                    let backend = CrosstermBackend::new(io::stdout());
                    terminal = Terminal::new(backend)?;
                }
                _ => {}
            }
        }
    }

    // Restore terminal
    terminal.show_cursor()?;
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn ui(f: &mut Frame<CrosstermBackend<Stdout>>, items: &[(String, PathBuf)], state: &mut ListState) {
    let size = f.size();
    let block = Block::default().borders(Borders::ALL).title("Ritsu Config - Enter to edit, q to quit");
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(&[Constraint::Percentage(100)])
        .split(size);

    let list_items: Vec<ListItem> = items
        .iter()
        .map(|(name, path)| {
            let hint = format!("{}", path.display());
            let content = format!("{}\n{}", name, hint);
            ListItem::new(content)
        })
        .collect();

    let list = List::new(list_items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_stateful_widget(list, chunks[0], state);
}

fn open_editor(path: &PathBuf) -> Result<()> {
    // If directory selected, open editor in that directory (e.g., nvim .)
    if path.is_dir() {
        let editor = env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string());
        let mut cmd = Command::new(editor);
        cmd.arg(".");
        cmd.current_dir(path);
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("Editor exited with non-zero status")
        }
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    if !path.exists() {
        fs::write(path, "# Ritsu prompt\n\n")?;
    }

    let editor = env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string());
    let status = Command::new(editor).arg(path).status()?;
    if !status.success() {
        anyhow::bail!("Editor exited with non-zero status")
    }
    Ok(())
}
