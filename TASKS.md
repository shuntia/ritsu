# Ritsu Critical Issues & Remediation Plan

## 🚨 P0 - CRITICAL RUNTIME BUGS (Will Break in Production)

### 1. SQL Bug: Non-existent Table Query
- **File**: `ritsu-server/src/memory.rs:541`
- **Issue**: `get_summaries()` queries non-existent "summaries" table
- **Impact**: `query_memory` tool will crash at runtime
- **Fix**: Query correct tables (daily_summaries/monthly_summaries) or implement proper dispatch

### 2. IPC Framing Corruption
- **File**: `ritsu/src/ipc.rs:29-46`
- **Issue**: `get_connection()` uses `try_read()` which consumes bytes from stream
- **Impact**: Corrupts length-prefixed protocol, causes deserialization failures
- **Fix**: Remove try_read peek pattern, use proper ping/pong or connection validation

### 3. Deadlock in ToolRegistry
- **File**: `ritsu-server/src/tools.rs:68-76`
- **Issue**: `execute()` holds RwLock read guard across await point
- **Impact**: Deadlock if tool handlers need registry access
- **Fix**: Drop guard before awaiting, or restructure lock scope

### 4. Blocking Rusqlite in Async
- **Files**: Throughout `memory.rs`, `conversations.rs`, `preferences.rs`, `tools.rs`, `trigger.rs`
- **Issue**: Synchronous `Connection::open()` and DB operations in async handlers
- **Impact**: Blocks runtime threads, poor concurrency, potential deadlocks
- **Fix**: Use `spawn_blocking` for all DB operations or create async DB pool

## 🔥 P1 - ARCHITECTURAL PROBLEMS

### 5. Inconsistent IPC Endianness
- **Files**: 
  - `ritsu-server/src/ipc.rs` (big-endian)
  - `ritsu-server/src/state.rs` (little-endian)
  - `ritsu/src/commands/client_daemon.rs` (little-endian)
- **Issue**: Mixed endianness in length prefixes across components
- **Impact**: Fragile, error-prone, future corruption risk
- **Fix**: Standardize on one endianness (recommend big-endian/network order)

### 6. Trigger Scheduling Never Wakes
- **File**: `ritsu-server/src/trigger.rs:517-621`
- **Issue**: `run_trigger_loop` sleeps until next trigger, no wake mechanism
- **Impact**: Newly-added triggers delayed arbitrarily until next natural wake
- **Fix**: Add tokio::sync::Notify or channel to interrupt sleep when triggers change

### 7. No Shutdown Coordination
- **File**: `ritsu-server/src/main.rs` and elsewhere
- **Issue**: Background tasks spawned without join handles or cancellation
- **Impact**: Orphaned processes, resource leaks, unclean shutdown
- **Fix**: Track all spawned tasks, implement graceful shutdown with tokio::select cancellation

### 8. Blocking I/O in Async Context
- **Files**: `memory.rs:336-360`, `main.rs:start_ollama_if_needed`
- **Issue**: `std::fs::read_to_string`, `std::process::Command::output` in async code
- **Impact**: Blocks executor threads, poor async performance
- **Fix**: Wrap all blocking I/O with `spawn_blocking`

### 9. Database Initialization Deadlock Risk
- **File**: `ritsu-server/src/database.rs:31-38`
- **Issue**: `block_in_place` + `rt.block_on` + async Mutex
- **Impact**: Runtime misuse, potential deadlock
- **Fix**: Initialize schema synchronously or remove nested async

### 10. Ad-hoc Database Connections
- **Files**: Throughout codebase
- **Issue**: `Connection::open()` called everywhere, no pooling
- **Impact**: Connection overhead, no coordination, hard to audit
- **Fix**: Single connection pool or async-safe DB gateway

## ⚠️ P2 - POLICY VIOLATIONS & TECHNICAL DEBT

### 11. Pervasive unwrap/expect/panic
- **Files**: `llm/*`, `ritsu/*`, `ritsu-server/*`, tests
- **Issue**: Violates project policy "no unwrap/expect/panic"
- **Impact**: Hidden crashes, poor error handling
- **Fix**: Replace all with proper Result propagation

### 12. Suppressed Clippy Lints
- **Files**: Widespread `#[allow(dead_code)]`, `#[allow(clippy::*)]`
- **Issue**: Hides unfinished code, violates pedantic lint policy
- **Impact**: Technical debt hidden
- **Fix**: Remove suppressions, fix underlying issues

### 13. Stream Parsing Fragility
- **File**: `llm/src/backends/ollama.rs:755-858`
- **Issue**: SSE parser treats non-JSON as fatal, only returns first tool call
- **Impact**: Fails on keepalive lines, incomplete tool calls
- **Fix**: Handle ignorable SSE lines, implement proper tool-call streaming

### 14. Timezone Confusion
- **File**: `ritsu-server/src/trigger.rs`
- **Issue**: Cron uses UTC, time triggers use Local
- **Impact**: Scheduling confusion, hard-to-debug issues
- **Fix**: Standardize on one timezone with clear conversions

### 15. Tests Depend on External Services
- **Files**: `llm/examples/*`, test files
- **Issue**: Tests require ENV vars, external APIs
- **Impact**: Flaky CI, can't run offline
- **Fix**: Mock external deps, make tests hermetic

## 📋 REMEDIATION PLAN

### Phase 1: Critical Fixes (P0) - ~4 hours
1. [ ] Fix `get_summaries` SQL bug
2. [ ] Remove `try_read` pattern from IPC client
3. [ ] Fix `ToolRegistry::execute` deadlock
4. [ ] Wrap all rusqlite calls in `spawn_blocking`
5. [ ] Test each fix thoroughly

### Phase 2: Async Architecture (P1) - ~6 hours
6. [ ] Standardize IPC endianness
7. [ ] Add trigger loop wake mechanism
8. [ ] Implement graceful shutdown
9. [ ] Wrap all blocking I/O in spawn_blocking
10. [ ] Fix database initialization pattern
11. [ ] Create centralized DB access pattern

### Phase 3: Compliance & Quality (P2) - ~8 hours
12. [ ] Remove all unwrap/expect/panic
13. [ ] Fix suppressed clippy lints
14. [ ] Harden stream parsing
15. [ ] Unify timezone handling
16. [ ] Make tests hermetic

### Phase 4: Clippy & Polish - ~2 hours
- [ ] Run `cargo clippy --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used`
- [ ] Fix remaining warnings
- [ ] Update documentation
- [ ] Add pre-commit hooks

## 🎯 SUCCESS CRITERIA

- [ ] All P0 bugs fixed and tested
- [ ] Zero clippy warnings with strict flags
- [ ] All tests pass without external deps
- [ ] Clean shutdown with no resource leaks
- [ ] No unwrap/expect/panic in production code
- [ ] Consistent IPC protocol
- [ ] Proper async patterns throughout

---

**Estimated Total Time**: 20+ hours
**Priority**: Start immediately with Phase 1
**Last Updated**: 2026-02-07
