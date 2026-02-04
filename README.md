# Ritsu - Self-Triggering AI Agent

An autonomous AI agent that can schedule and trigger itself arbitrarily, with persistent memory and task management.

## Features

✅ **Memory System**: Daily/monthly conversation compaction with 40-day rotation  
✅ **Multi-Backend LLM**: Ollama (local) + OpenAI-compatible APIs  
✅ **Task Management**: Full CRUD with priorities, tags, due dates  
✅ **Tool System**: 9 extensible async tools  
✅ **Strict Lints**: All code passes clippy pedantic/nursery  

## Quick Start

### Prerequisites

- Rust 1.93+ 
- Ollama (or OpenAI-compatible API)

### Build

```bash
cargo build --release
```

### Configuration

```bash
mkdir -p ~/.config/ritsu
cp config.toml.example ~/.config/ritsu/config.toml
# Edit config.toml with your settings
```

### Run Server

```bash
./target/release/ritsu-server
```

### Use Client

```bash
./target/release/ritsu status
./target/release/ritsu task list
./target/release/ritsu chat  # GUI (when implemented)
```

## Architecture

- **ritsu-server**: Long-running daemon with SQLite, LLM, tools
- **ritsu**: Unified CLI/GUI client  
- **ritsu-common**: Shared types and IPC protocol

See `AGENTS.md` for detailed architecture and development plan.

## Development Status

**Phase 1-4 Complete:**
- ✅ Foundation (workspace, config, database)
- ✅ Memory System (storage, compaction, tasks)
- ✅ LLM Integration (Ollama + OpenAI)
- ✅ Tool System (9 core tools)

**Remaining:**
- ⏳ Trigger System & Idle Analysis
- ⏳ IPC Implementation
- ⏳ CLI Commands
- ⏳ GUI (iced)

## Development

```bash
# Run clippy (required before commit)
cargo clippy --all-features -- -D warnings

# Run server in dev mode
RUST_LOG=debug cargo run -p ritsu-server

# Run client
cargo run -p ritsu -- --help
```

## License

MIT OR Apache-2.0
