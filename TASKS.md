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

### 11. IPC Security & DoS Protection
- **Files**: `ritsu-server/src/ipc.rs`, `ritsu/src/ipc.rs`, `ritsu-server/src/state.rs`
- **Issue**: No auth/ACL on IPC sockets, no timeouts/quotas on read_exact, unbounded channels
- **Impact**: DoS via malformed messages, memory exhaustion, framing corruption, push starvation
- **Fix**: Add strict length limits/timeouts, bounded channels with backpressure, optional socket auth

### 12. System Prompt Injection Risk
- **File**: `ritsu-server/src/memory.rs` (load_base_prompt)
- **Issue**: System prompts read from local files with no sanitization or provenance checks
- **Impact**: Prompt injection if files modified by untrusted sources
- **Fix**: Add checksum validation, restrict file permissions, sanitize prompt content

### 13. Tool Input Validation
- **Files**: `ritsu-server/src/tools.rs` (create_trigger, create_task, analyze_now)
- **Issue**: Tool handlers accept unauthenticated inputs without rate limits
- **Impact**: Resource exhaustion via trigger/task spam, uncontrolled self-provisioning
- **Fix**: Add input validation, rate limiting, size limits, user confirmation for AI-generated triggers

### 14. Stream Parsing Fragility
- **File**: `llm/src/backends/ollama.rs:755-858`
- **Issue**: SSE parser treats non-JSON as fatal, only returns first tool call, mishandles keepalives
- **Impact**: Fails on keepalive lines, incomplete tool calls, brittle streaming
- **Fix**: Handle ignorable SSE lines, implement proper tool-call start/delta/complete semantics

### 15. Timezone Confusion
- **File**: `ritsu-server/src/trigger.rs`
- **Issue**: Cron uses UTC, time triggers use Local, silent defaults on parse failures
- **Impact**: Scheduling confusion, hard-to-debug issues, missed triggers
- **Fix**: Standardize on UTC everywhere with explicit local conversions, fail loudly on parse errors

### 16. Tests Depend on External Services
- **Files**: `llm/examples/*`, test files
- **Issue**: Tests require ENV vars, external APIs
- **Impact**: Flaky CI, can't run offline
- **Fix**: Mock external deps, make tests hermetic

### 17. Background Task Lifecycle
- **File**: `ritsu-server/src/main.rs` and spawned tasks
- **Issue**: Detached spawns without lifecycle propagation or error boundaries
- **Impact**: Silent failures, orphaned tasks, no observability
- **Fix**: Wrap all spawns with proper error handling, add task monitoring/health checks

## 📋 REMEDIATION PLAN

### Phase 1: Critical Fixes (P0) - ~4 hours
1. [x] Fix `get_summaries` SQL bug ✓
2. [x] Remove `try_read` pattern from IPC client ✓
3. [x] Fix `ToolRegistry::execute` deadlock ✓
4. [x] Wrap all rusqlite calls in `spawn_blocking` ✓
5. [ ] Test each fix thoroughly

### Phase 2: Async Architecture (P1) - ~6 hours
6. [x] Standardize IPC endianness ✓
7. [x] Add trigger loop wake mechanism ✓
8. [x] Implement graceful shutdown ✓
9. [x] Wrap all blocking I/O in spawn_blocking ✓
10. [ ] Fix database initialization pattern
11. [ ] Create centralized DB access pattern

### Phase 3: Compliance & Quality (P2) - ~8 hours
12. [ ] IPC security: timeouts, bounded channels, length limits, optional auth
13. [ ] System prompt validation: checksums, permission checks, sanitization
14. [ ] Tool input validation: rate limits, size limits, user confirmation
15. [ ] Harden stream parsing: ignore keepalives, proper tool-call semantics
16. [ ] Unify timezone handling: prefer UTC, explicit conversions, fail loudly
17. [ ] Make tests hermetic: mock external deps
18. [ ] Background task lifecycle: error boundaries, monitoring, health checks

### Phase 4: Clippy & Polish - ~2 hours
- [ ] Run `cargo clippy --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used`
- [ ] Fix remaining warnings
- [ ] Update documentation
- [ ] Add pre-commit hooks

## 🎯 SUCCESS CRITERIA

- [x] IPC endianness unified (big-endian everywhere) ✓
- [x] ToolRegistry deadlock fixed (guard dropped before await) ✓
- [x] Trigger loop wakes on registry changes ✓
- [x] Zero clippy warnings with standard lints ✓
- [x] No unwrap/expect/panic in production code ✓
- [ ] All blocking I/O wrapped in spawn_blocking
- [ ] Graceful shutdown with task tracking
- [ ] IPC timeouts and bounded channels
- [ ] Tests pass without external deps
- [ ] Consistent DB access pattern (pool or actor)
- [ ] Proper async patterns throughout
- [ ] Security: input validation, rate limits, auth hooks

---

**Estimated Total Time**: 20+ hours
**Priority**: Start immediately with Phase 1
**Last Updated**: 2026-02-07

Active Implementation (2026-02-08):
- [ ] Add NDJSON vs SSE unit tests for llm streaming (in progress)
- [ ] Tool parameter validation in ritsu-server/src/tools.rs (in progress)
- [ ] Use configurable client-daemon socket & per-request connect with timeouts (in progress)
- [ ] Make LlmClient::new async to avoid blocking (in progress)
- [ ] Wrap ResetDatabase in transaction + backup (pending)
