# Ritsu - Self-Triggering AI Agent

An autonomous AI agent that can schedule and trigger itself arbitrarily, with persistent memory and task management.

## Project Status

🚧 **Phase 1: Foundation** - In Progress

- [x] Cargo workspace setup
- [x] Strict clippy lints configured
- [x] Basic configuration system
- [x] Database schema initialized
- [ ] IPC protocol implementation
- [ ] Server daemon runtime

## Building

```bash
# Build all crates
cargo build --all-features

# Build server
cargo build -p ritsu-server

# Build client
cargo build -p ritsu

# Build without GUI
cargo build -p ritsu --no-default-features
```

## Running

```bash
# Start the server
cargo run -p ritsu-server

# Use the client (placeholder commands)
cargo run -p ritsu -- status
cargo run -p ritsu -- chat
```

## Development

Before committing, ensure all lints pass:

```bash
cargo clippy --all-features --all-targets -- -D warnings
```

## License

MIT OR Apache-2.0
