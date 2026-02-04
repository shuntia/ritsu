# Ritsu - Self-Triggering AI Agent

An autonomous AI agent that can schedule and trigger itself arbitrarily, with persistent memory and task management.

## Features

✅ **Memory System**: Daily/monthly conversation compaction with 40-day rotation  
✅ **Multi-Backend LLM**: Ollama (local) + OpenAI-compatible APIs  
✅ **Task Management**: Full CRUD with priorities, tags, due dates  
✅ **Tool System**: 9 extensible async tools  
✅ **IPC Architecture**: Unix domain socket client-server communication  
✅ **GUI & CLI**: Unified client with iced-powered chat interface  
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
# Edit config.toml with your LLM backend settings
```

### Usage

```bash
# Start the server daemon
./target/release/ritsu start

# Check server status
./target/release/ritsu status

# Send a quick message
./target/release/ritsu send "Hello Ritsu!"

# Task management
./target/release/ritsu task list
./target/release/ritsu task add "Buy groceries" --priority high --due 2026-03-01
./target/release/ritsu task update 1 --status completed

# Open GUI chat interface
./target/release/ritsu chat

# Query memory
./target/release/ritsu memory --days 7

# Stop the server
./target/release/ritsu stop
```

## Architecture

- **ritsu-server**: Long-running daemon with SQLite, LLM integration, IPC server
- **ritsu**: Unified CLI/GUI client communicating via Unix domain sockets
- **ritsu-common**: Shared types and IPC protocol definitions

### IPC Protocol

Length-prefixed bincode serialization over Unix domain socket (`/tmp/ritsu.sock`):
- 4-byte big-endian length header
- Bincode-serialized request/response payload

See `AGENTS.md` for detailed architecture and development plan.

## Development Status

**Completed Phases (1-7):**
- ✅ Foundation (workspace, config, database)
- ✅ Memory System (storage, compaction, tasks)
- ✅ LLM Integration (Ollama + OpenAI)
- ✅ Tool System (9 core tools)
- ✅ Trigger System (basic infrastructure)
- ✅ IPC Client-Server Implementation
- ✅ GUI (iced chat interface)

**Remaining:**
- ⏳ Trigger Create/Delete/Disable Implementation
- ⏳ Idle Analysis & Self-Modification
- ⏳ Memory Query Real Implementation
- ⏳ Notification System Integration

## Development

```bash
# Run clippy (required before commit)
cargo clippy --all-features -- -D warnings

# Run server directly (for debugging)
RUST_LOG=debug cargo run -p ritsu-server

# Run end-to-end test
export PATH="$(pwd)/target/debug:$PATH"
./test-ritsu.sh
```

## Testing

The project includes an end-to-end test script that validates:
- Server start/stop lifecycle
- Message sending via IPC
- Task creation, listing, and updates
- Trigger listing
- Memory queries

## License

MIT OR Apache-2.0
