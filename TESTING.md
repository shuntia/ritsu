# Ritsu Testing Guide

## Overview

Ritsu includes comprehensive testing at multiple levels:

1. **Unit Tests** - Test individual components in isolation
2. **Integration Tests** - Test IPC and component interactions
3. **E2E Tests** - Test full system with real server and client daemons

## Running Tests

### Unit Tests

```bash
# Run all unit tests
cargo test

# Run tests for a specific package
cargo test -p ritsu-server
cargo test -p ritsu-common
cargo test -p ritsu
```

### Integration Tests

```bash
# Run integration tests
cargo test --test ipc_test
```

### End-to-End Tests

E2E tests start real server and client daemon processes and test the full interaction flow.

```bash
# Build binaries and run all E2E tests
./test-e2e.sh --build

# Run all E2E tests (binaries must exist)
./test-e2e.sh

# Run specific E2E test
./test-e2e.sh ping
./test-e2e.sh send_message
./test-e2e.sh session_persistence
./test-e2e.sh concurrent_messages

# Run E2E tests manually with cargo
cargo test --test e2e_test -- --ignored --nocapture --test-threads=1
```

**Note:** E2E tests are marked with `#[ignore]` by default and require:
- Built binaries in `target/debug/`
- Ollama running (optional but recommended)
- Clean test environment (no conflicting processes)

## E2E Test Scenarios

### test_server_startup_and_ping
Tests that server and client daemon start correctly and can communicate.

### test_send_message_and_receive_response
Tests sending a message through the full stack and receiving a response.

### test_session_persistence
Tests that conversation sessions are maintained across multiple messages.

### test_task_management
Tests creating and listing tasks through the CLI.

### test_notes_and_memory
Tests querying notes and memory.

### test_trigger_management
Tests listing triggers.

### test_client_daemon_restart
Tests that the client daemon can restart and continue working.

### test_concurrent_messages
Tests multiple concurrent message sends to verify thread safety.

### test_full_conversation_flow
Tests a complete multi-turn conversation.

## Test Environment

E2E tests use isolated temporary directories:
- Test DB: `/tmp/ritsu_test_<testname>/test.db`
- Test sockets: `/tmp/ritsu_test_<testname>/*.sock`
- Test config: `/tmp/ritsu_test_<testname>/config.toml`

Tests clean up after themselves, but manual cleanup may be needed after failures:

```bash
# Kill any leftover processes
pkill -f ritsu-server
pkill -f "ritsu.*start"

# Remove leftover sockets
rm -f /tmp/ritsu*.sock

# Remove test directories
rm -rf /tmp/ritsu_test_*
```

## Prerequisites for E2E Tests

1. **Built Binaries**
   ```bash
   cargo build --bin ritsu --bin ritsu-server
   ```

2. **Ollama (Recommended)**
   ```bash
   # Check if running
   curl http://localhost:11434/api/tags
   
   # Start if needed
   ollama serve
   
   # Pull test model
   ollama pull llama3.2:3b
   ```

3. **Clean Environment**
   - No ritsu processes running
   - No conflicting socket files

## Writing New Tests

### Unit Test Example

```rust
#[test]
fn test_my_function() {
    let result = my_function();
    assert_eq!(result, expected);
}
```

### Async Test Example

```rust
#[tokio::test]
async fn test_async_operation() {
    let result = my_async_function().await.unwrap();
    assert!(result);
}
```

### E2E Test Example

```rust
#[tokio::test]
#[ignore]
async fn test_new_feature() -> Result<()> {
    let mut fixture = TestFixture::new("feature").await?;
    
    fixture.start_server().await?;
    fixture.start_client_daemon().await?;
    
    let (success, output) = run_command(&["my", "command"]).await?;
    
    assert!(success, "Command should succeed");
    assert!(output.contains("expected text"));
    
    Ok(())
}
```

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      
      - name: Install Ollama
        run: |
          curl -fsSL https://ollama.com/install.sh | sh
          ollama serve &
          ollama pull llama3.2:3b
      
      - name: Run unit tests
        run: cargo test
      
      - name: Build binaries
        run: cargo build --bin ritsu --bin ritsu-server
      
      - name: Run E2E tests
        run: ./test-e2e.sh
```

## Troubleshooting

### Tests hang or timeout
- Check if processes are stuck: `ps aux | grep ritsu`
- Kill stuck processes: `pkill -9 -f ritsu`
- Remove sockets: `rm -f /tmp/ritsu*.sock`

### Connection refused errors
- Ensure server starts successfully
- Check socket permissions
- Verify socket paths in test config

### LLM-related failures
- Some tests may fail if Ollama isn't running
- Check Ollama status: `curl http://localhost:11434/api/tags`
- Tests should handle LLM failures gracefully

### Port/socket conflicts
- Ensure no other ritsu instances are running
- Run cleanup: `./test-e2e.sh` will clean up automatically

## Performance Testing

For performance testing, use:

```bash
# Run with timing
cargo test --test e2e_test -- --ignored --nocapture --test-threads=1 --show-output

# Profile with flamegraph
cargo flamegraph --test e2e_test -- --ignored test_concurrent_messages
```

## Test Coverage

Check test coverage with:

```bash
# Install tarpaulin
cargo install cargo-tarpaulin

# Run coverage
cargo tarpaulin --test e2e_test --ignore-tests --out Html

# View report
open tarpaulin-report.html
```
