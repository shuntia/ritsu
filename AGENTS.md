# Ritsu - Self-Triggering AI Agent

**Ritsu** is an autonomous AI agent that can schedule and trigger itself arbitrarily. It maintains memory through SQLite, compacts conversations into summaries, and can execute tools based on context and scheduled triggers.

---

## AI Agent Development Guidelines

**You are a professional, pedantic developer strictly following best practices.**

### Core Principles

1. **Follow Best Practices Religiously**
   - Write clean, maintainable, well-documented code
   - Follow SOLID principles and DRY
   - Use proper error handling (no unwrap/expect/panic)
   - Write comprehensive tests for all features
   - Follow Rust idioms and conventions

2. **TODO.md is Your Source of Truth**
   - **ALWAYS** check TODO.md before starting work
   - **ALWAYS** update TODO.md as you complete tasks
   - Break down large features into small, testable increments
   - Mark tasks as complete with [x] when done
   - Add new discovered tasks as they arise

3. **Commit Frequently Without GPG Signing**
   - Make small, atomic commits for each logical change
   - Use clear, descriptive commit messages
   - Always commit with `--no-gpg-sign` flag
   - Commit after each completed subtask or bug fix
   - Never bundle unrelated changes in one commit

4. **Quality Gates Before Every Commit**
   ```bash
   # 1. Run clippy with strict lints
   cargo clippy --all-features --all-targets -- -D warnings
   
   # 2. Run all tests
   cargo test --all-features
   
   # 3. Only commit if both pass
   git add -A
   git commit --no-gpg-sign -m "Clear, concise message"
   ```

5. **Code Review Standards**
   - No hard-coded values (use config, constants, or enums)
   - No `.unwrap()`, `.expect()`, or `panic!()`
   - All errors properly propagated with context
   - Timeouts on all blocking operations
   - Loose coupling via traits/interfaces
   - Comprehensive documentation for public APIs
   - Tests for all new functionality

6. **Documentation Requirements**
   - Update AGENTS.md for architecture changes
   - Update TODO.md for task progress
   - Add inline comments for complex logic
   - Document all public functions with rustdoc
   - Keep README.md current with usage examples

### Workflow

1. Read TODO.md to understand current state
2. Pick next uncompleted task
3. Implement with tests
4. Run quality gates (clippy + tests)
5. Update TODO.md marking task complete
6. Commit with --no-gpg-sign
7. Repeat

### Commit Message Format
```
<type>: <short summary>

<optional detailed description>

Related task: <TODO.md reference>
```

Types: feat, fix, refactor, test, docs, chore

---

## Architecture Overview

### Components
1. **Server (ritsu-server)**: Long-running daemon that:
   - Manages the event loop and trigger scheduling
   - Maintains SQLite database for memory storage
   - Handles LLM interactions for decision-making
   - Executes scheduled tasks and tool calls
   - Listens for client connections

2. **Client (ritsu)**: Unified client binary with subcommands:
   - **Daemon management**: `ritsu start`, `ritsu stop`, `ritsu status`, `ritsu restart`
   - **Trigger management**: `ritsu trigger list`, `ritsu trigger add`, etc.
   - **Memory queries**: `ritsu memory`, `ritsu notes`
   - **Task management**: `ritsu task list`, `ritsu task add`, etc.
   - **Chat GUI**: `ritsu chat` (launches iced-based GUI)
   - **Quick messages**: `ritsu send "message"`

3. **Client Daemon (ritsu)**: Background process for UI interactions:
   - **Notification daemon**: `ritsu start-client-daemon`
   - Subscribes to server push notifications
   - Displays system notifications via notify-rust
   - Launches GUI windows on demand
   - Runs separately from server for clean separation

### Communication
- Server-client communication via IPC (Unix domain sockets)
- CLI subcommands send admin/query requests to server
- Client daemon subscribes to push notifications (ServerPush protocol)
- `ritsu chat` launches GUI for interactive conversations
- Server broadcasts to client daemon for notifications and GUI launches

## Core Technologies

- **Language**: Rust
- **Async Runtime**: tokio (for async/await and scheduling)
- **Database**: SQLite (via rusqlite or sqlx)
- **LLM Integration**: `llm` crate (prioritize local ollama, support multi-backend remote APIs)
- **GUI Framework**: iced (for chat interface)
- **Serialization**: serde + serde_json for configuration and IPC

## Memory System

### SQLite Schema

#### 1. Notes Table
```sql
CREATE TABLE notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    content TEXT NOT NULL,
    tags TEXT -- JSON array of tags
);
```

#### 2. Daily Summary Table
```sql
CREATE TABLE daily_summaries (
    date DATE PRIMARY KEY,
    summary TEXT NOT NULL,
    tags TEXT, -- JSON array of tags
    conversation_count INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### 3. Monthly Summary Table
```sql
CREATE TABLE monthly_summaries (
    year_month TEXT PRIMARY KEY, -- Format: "YYYY-MM"
    summary TEXT NOT NULL,
    tags TEXT, -- JSON array of tags
    days_included INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### 4. Daily Conversations Table
```sql
CREATE TABLE daily_conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date DATE NOT NULL,
    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    role TEXT NOT NULL, -- "user", "assistant", "system"
    content TEXT NOT NULL,
    INDEX idx_date (date)
);
```

#### 5. Triggers Table
```sql
CREATE TABLE triggers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    trigger_type TEXT NOT NULL, -- "time", "interval", "event"
    schedule TEXT NOT NULL, -- cron-like or ISO8601 for time-based
    enabled BOOLEAN DEFAULT TRUE,
    created_by TEXT, -- "user" or "ai"
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    metadata TEXT -- JSON for additional trigger config
);
```

#### 6. System Prompt Table
```sql
CREATE TABLE system_prompts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    prompt_type TEXT NOT NULL, -- "base", "ai_generated"
    content TEXT NOT NULL,
    version INTEGER DEFAULT 1,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    active BOOLEAN DEFAULT TRUE
);
```

#### 7. Idle Analysis Table
```sql
CREATE TABLE idle_analyses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    analysis_type TEXT NOT NULL, -- "conversation", "patterns", "tools", "self_reflection"
    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    findings TEXT NOT NULL, -- JSON with analysis results
    prompted_changes TEXT -- Suggested prompt modifications
);
```

#### 8. Tasks Table
```sql
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL, -- "pending", "in_progress", "completed", "cancelled"
    priority TEXT NOT NULL, -- "low", "medium", "high", "urgent"
    tags TEXT, -- JSON array of tags
    due_date TIMESTAMP,
    created_by TEXT NOT NULL, -- "user" or "ai"
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP
);
```

### Memory Compaction Rules

1. **Daily Compaction**: At end of each day (or first trigger next day):
   - Aggregate all `daily_conversations` for the previous day
   - Generate summary via LLM with extracted tags
   - Insert into `daily_summaries` with date, summary, tags, conversation count
   - Retain conversations in `daily_conversations` for 40 days

2. **Monthly Compaction**: At end of each month:
   - Aggregate all `daily_summaries` for the month
   - Generate monthly summary via LLM with extracted tags
   - Insert into `monthly_summaries` with year_month, summary, tags, days count

3. **40-Day Rotation**: 
   - Delete `daily_conversations` older than 40 days
   - Daily summaries are already created, so deletion is safe
   - Keep `daily_summaries` and `monthly_summaries` indefinitely

## System Prompt & Idle Analysis

### System Prompt Architecture

The system prompt is composed of two parts:

1. **Base System Prompt**: Preconfigured, user-editable prompt stored in database
   - Defines ritsu's core personality, goals, and behavior
   - Stored with `prompt_type = "base"` and `active = true`

2. **AI-Generated Prompt Additions**: Dynamically generated during idle analysis
   - Appended to base prompt to create effective system prompt
   - Based on learned patterns, user preferences, interaction history
   - Stored with `prompt_type = "ai_generated"` and `active = true`
   - Persistent until AI revises them during future analysis

### Effective System Prompt Construction

```rust
async fn build_system_prompt(db: &Database) -> String {
    let base = db.get_active_prompt("base").await;
    let ai_additions = db.get_active_prompt("ai_generated").await;
    
    format!("{}\n\n--- AI-Generated Context ---\n{}", base, ai_additions)
}
```

### Idle Analysis System

Idle analysis runs at various intervals based on analysis type:

#### 1. Conversation Analysis (After inactivity or daily)
- **Trigger**: No activity for 30+ minutes, or daily at 2:00 AM
- **Analyzes**: Recent conversations, user requests, tool usage
- **Produces**: Insights about user preferences, common patterns
- **Prompt Updates**: Adjust tone, add context about user schedule/habits

#### 2. Pattern Recognition (Weekly)
- **Trigger**: Every Sunday at 3:00 AM
- **Analyzes**: Week's worth of daily summaries
- **Produces**: Recurring patterns, optimal trigger timings
- **Prompt Updates**: Note recurring tasks, preferred interaction styles

#### 3. Tool Effectiveness Analysis (Bi-weekly)
- **Trigger**: 1st and 15th of month at 4:00 AM
- **Analyzes**: Tool usage frequency, success rates, user feedback
- **Produces**: Which tools are most useful, which need improvement
- **Prompt Updates**: Prioritize frequently-used tools, note tool preferences

#### 4. Self-Reflection (Monthly)
- **Trigger**: Last day of month at 5:00 AM (after monthly compaction)
- **Analyzes**: Monthly summary, AI-generated triggers, prompt evolution
- **Produces**: Long-term behavioral adjustments
- **Prompt Updates**: Major personality/approach adjustments

#### 5. AI-Initiated Analysis (Dynamic)
- **Trigger**: AI decides during any interaction when it notices significant patterns
- **Analyzes**: Immediate context that warrants prompt update
- **Produces**: Quick adaptations to user needs
- **Prompt Updates**: Add newly discovered preferences or constraints

### Idle Analysis Flow

```rust
async fn perform_idle_analysis(
    analysis_type: AnalysisType,
    db: Arc<Database>,
    llm: Arc<LLMClient>
) -> Result<()> {
    // 1. Gather relevant data from memory
    let data = match analysis_type {
        AnalysisType::Conversation => db.get_recent_conversations(24).await?,
        AnalysisType::Pattern => db.get_daily_summaries(7).await?,
        AnalysisType::Tools => db.get_tool_usage_stats(14).await?,
        AnalysisType::SelfReflection => db.get_monthly_summary(current_month()).await?,
        AnalysisType::Dynamic => db.get_relevant_context().await?,
    };
    
    // 2. Send to LLM for analysis
    let analysis_prompt = format!(
        "Analyze the following data and identify patterns:\n{}\n\
         Suggest modifications to your system prompt to better serve the user.",
        data
    );
    
    let analysis = llm.analyze(analysis_prompt).await?;
    
    // 3. Store analysis results
    db.store_idle_analysis(analysis_type, &analysis).await?;
    
    // 4. Update system prompt if needed
    if let Some(prompt_changes) = analysis.prompt_modifications {
        db.update_ai_generated_prompt(&prompt_changes).await?;
    }
    
    Ok(())
}
```

### Idle Trigger Integration

Idle analysis is integrated into the trigger system:

```rust
// Built-in idle analysis triggers (created on server startup)
let idle_triggers = vec![
    Trigger::new("idle_conversation", "0 2 * * *"), // Daily 2 AM
    Trigger::new("idle_pattern", "0 3 * * 0"),      // Sunday 3 AM
    Trigger::new("idle_tools", "0 4 1,15 * *"),     // 1st & 15th, 4 AM
    Trigger::new("idle_reflection", "0 5 L * *"),   // Last day of month, 5 AM
];
```

AI can also trigger analysis dynamically via the `analyze_now` tool during conversations.

## Tool System

### Tool Definition
```rust
type ToolFunction = Box<dyn Fn(HashMap<String, String>) -> Pin<Box<dyn Future<Output = ToolResult> + Send>> + Send + Sync>;

struct ToolRegistry {
    tools: HashMap<String, ToolFunction>,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}
```

### Built-in Tools (v1)

1. **notify_client**: Send system notification to connected client(s)
   - Args: `{"title": "...", "message": "...", "urgency": "low|normal|critical"}`
   - Clients receive notification and display via OS native notifications (libnotify on Linux, etc.)

2. **create_trigger**: AI can schedule new triggers
   - Args: `{"name": "...", "type": "time|interval", "schedule": "..."}`
   - Dynamically adds trigger to registry and database

3. **create_note**: Store a note in memory
   - Args: `{"content": "...", "tags": "tag1,tag2"}`

4. **query_memory**: Retrieve information from memory (notes, summaries)
   - Args: `{"type": "notes|daily|monthly", "query": "...", "date_range": "..."}`

5. **analyze_now**: Trigger immediate idle analysis
   - Args: `{"type": "conversation|pattern|tools|self_reflection|dynamic"}`
   - Allows AI to analyze current context and update its prompt during conversation

6. **open_chat**: Open chat window and request focus
   - Args: `{"message": "...", "urgency": "low|normal|urgent"}`
   - Launches `ritsu chat` if not running, brings window to focus
   - Used in conjunction with notifications for important interactions

7. **create_task**: Create a new task
   - Args: `{"title": "...", "description": "...", "priority": "low|medium|high|urgent", "due_date": "ISO8601", "tags": "tag1,tag2"}`
   - Both AI and user can create tasks

8. **update_task**: Update task status or details
   - Args: `{"id": "123", "status": "...", "priority": "...", ...}`

9. **list_tasks**: Query tasks with filters
   - Args: `{"status": "pending|in_progress|completed", "priority": "...", "tags": "...", "due_before": "..."}`

### Timeouts and Error Handling

All blocking operations have configurable timeouts:

1. **User Response Timeout**: 5 minutes (default)
   - When AI sends a message/notification and waits for response
   - On timeout: AI is notified that user ignored the message
   - AI decides next action (retry later, cancel, create reminder, etc.)
   - Configurable per interaction via metadata

2. **HTTP Request Timeout**: 30 seconds (default)
   - For future HTTP tool calls (when Deno integration added)
   - Prevents hanging on unresponsive endpoints
   - Configurable per request

3. **LLM Request Timeout**: 60 seconds (default)
   - For LLM API calls
   - Falls back to alternative backend on timeout

```rust
pub struct TimeoutConfig {
    pub user_response: Duration,      // Default: 5 minutes
    pub http_request: Duration,        // Default: 30 seconds
    pub llm_request: Duration,         // Default: 60 seconds
}

async fn wait_for_user_response(
    timeout: Duration,
    llm: Arc<LLMClient>
) -> Result<Option<String>> {
    match tokio::time::timeout(timeout, receive_user_message()).await {
        Ok(response) => Ok(Some(response?)),
        Err(_) => {
            // Timeout occurred - notify AI
            llm.notify_timeout("User did not respond within timeout").await?;
            Ok(None)
        }
    }
}
```

### Future Tools (post-v1)
- HTTP requests (if Deno runtime added)
- Screen time tracking integration
- Screen lock control

## Trigger System

### Trigger Types

1. **Time-based**: Fire at specific time(s)
   - Example: `"0 7 * * *"` (7:00 AM daily, cron-like syntax)
   - Use `tokio::time::sleep_until` for scheduling

2. **Interval-based**: Fire periodically
   - Example: `"every 30m"` (every 30 minutes)
   - Use `tokio::time::interval`

3. **Event-based** (future): Fire on system events
   - Example: system startup, user login, file change

### Trigger Execution Flow

```rust
async fn run_trigger_loop(registry: Arc<TriggerRegistry>, db: Arc<Database>, llm: Arc<LLMClient>) {
    let mut futures: Vec<Pin<Box<dyn Future<Output = RitsuTrigger>>>> = vec![];
    
    // Load triggers from database
    for trigger in registry.get_all_enabled() {
        futures.push(Box::pin(wait_for_trigger(trigger)));
    }
    
    loop {
        // Wait for any trigger to fire
        let (trigger, _index, remaining) = select_all(futures).await;
        
        // Execute trigger handler
        handle_trigger(trigger, &db, &llm).await;
        
        // Reschedule or remove trigger
        futures = remaining;
        if trigger.is_recurring() {
            futures.push(Box::pin(wait_for_trigger(trigger.clone())));
        }
    }
}
```

### AI-Generated Triggers

- AI can analyze context and create triggers autonomously via the `create_trigger` tool
- Example: "User mentioned school at 8:30 AM" → AI creates trigger at 7:00 AM with alarm notification
- User can review, approve, or revoke AI-generated triggers via client

## LLM Integration

### Backend Configuration

Use the `llm` crate to support multiple backends:

1. **Primary**: Local Ollama endpoint (http://localhost:11434)
2. **Fallback**: Remote APIs (OpenAI-compatible endpoints)

```rust
struct LLMConfig {
    backends: Vec<LLMBackend>,
    default: String,
}

struct LLMBackend {
    name: String,
    endpoint: String,
    model: String,
    api_key: Option<String>,
}
```

### Configuration File (TOML)

```toml
# ~/.config/ritsu/config.toml

[llm]
default_backend = "ollama"

[[llm.backends]]
name = "ollama"
endpoint = "http://localhost:11434"
model = "llama3.2"

[[llm.backends]]
name = "openai"
endpoint = "https://api.openai.com/v1"
model = "gpt-4"
api_key_env = "OPENAI_API_KEY"

[server]
socket_path = "/tmp/ritsu.sock"
database_path = "~/.local/share/ritsu/ritsu.db"
client_binary_path = "/usr/bin/ritsu"  # For launching `ritsu chat`

[memory]
daily_rotation_days = 40

[timeouts]
user_response_seconds = 300     # 5 minutes
http_request_seconds = 30       # 30 seconds
llm_request_seconds = 60        # 60 seconds
```

## Project Structure

```
ritsu-sonnet/
├── Cargo.toml (workspace)
├── ritsu-server/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs          # Server entry point
│   │   ├── database.rs      # SQLite operations
│   │   ├── memory.rs        # Memory compaction logic
│   │   ├── triggers.rs      # Trigger system
│   │   ├── tools.rs         # Tool registry and execution
│   │   ├── llm.rs           # LLM client wrapper
│   │   ├── ipc.rs           # Server-client communication
│   │   └── focus.rs         # GUI launch and focus control
├── ritsu/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs          # CLI entry point with subcommands
│   │   ├── commands/
│   │   │   ├── daemon.rs    # start, stop, status, restart
│   │   │   ├── trigger.rs   # trigger management
│   │   │   ├── memory.rs    # memory queries
│   │   │   ├── task.rs      # task management
│   │   │   ├── chat.rs      # Launch GUI chat interface
│   │   │   └── send.rs      # Quick message sending
│   │   ├── gui/
│   │   │   ├── mod.rs       # GUI module entry
│   │   │   ├── app.rs       # iced application
│   │   │   ├── chat.rs      # Chat interface
│   │   │   └── notify.rs    # System notifications
│   │   └── ipc.rs           # Client-server communication
└── ritsu-common/
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        ├── protocol.rs      # IPC protocol definitions
        └── types.rs         # Shared types
```

## Development Phases

### Phase 1: Foundation
- [ ] Set up Cargo workspace (server, client, common crates)
- [ ] Configure strict clippy lints in workspace Cargo.toml and clippy.toml
- [ ] Implement SQLite schema and database module with migrations
- [ ] Create basic IPC protocol between server and client
- [ ] Implement server daemon with tokio runtime
- [ ] Set up configuration system with TOML parsing and defaults

### Phase 2: Memory System
- [ ] Implement daily conversation storage
- [ ] Build daily compaction logic (end-of-day summary)
- [ ] Build monthly compaction logic
- [ ] Implement 40-day rotation cleanup task
- [ ] Create system prompt storage and retrieval
- [ ] Implement idle analysis tables
- [ ] Implement tasks database and CRUD operations

### Phase 3: LLM Integration
- [ ] Integrate `llm` crate with Ollama support
- [ ] Add multi-backend configuration support
- [ ] Implement conversation context building from memory
- [ ] Add tool calling to LLM prompts (function calling)

### Phase 4: Tool System
- [ ] Build tool registry with HashMap of async functions
- [ ] Implement `notify_client` tool
- [ ] Implement `create_note` and `query_memory` tools
- [ ] Implement `create_trigger` tool (AI self-triggering)
- [ ] Implement `analyze_now` tool for dynamic idle analysis
- [ ] Implement `open_chat` tool (launch GUI and request focus)
- [ ] Implement task management tools (create_task, update_task, list_tasks)
- [ ] Add timeout handling for user responses

### Phase 5: Trigger System & Idle Analysis
- [ ] Design trigger data structures and persistence
- [ ] Implement time-based triggers with tokio
- [ ] Implement interval-based triggers
- [ ] Build trigger execution loop with `select_all`
- [ ] Implement idle analysis triggers (daily, weekly, bi-weekly, monthly)
- [ ] Build idle analysis logic for each analysis type
- [ ] Implement system prompt composition and updates

### Phase 6: CLI Client
- [ ] Set up clap for CLI subcommand structure
- [ ] Implement daemon commands (start, stop, status, restart)
- [ ] Implement trigger management commands
- [ ] Implement memory query commands
- [ ] Implement task management commands
- [ ] Implement quick send command

### Phase 7: GUI Client (iced)
- [ ] Set up iced application in `ritsu chat` subcommand
- [ ] Build chat interface (message list + input field)
- [ ] Implement IPC connection to server
- [ ] Display conversation history from memory
- [ ] Handle real-time message streaming from LLM
- [ ] Implement system notification handling (notify-rust)
- [ ] Add connection status indicator
- [ ] Implement window focus and launch control
- [ ] Handle focus requests from server

### Phase 8: Polish & Testing
- [ ] Write integration tests for trigger execution
- [ ] Test memory compaction and rotation
- [ ] Test task management workflows
- [ ] Add configuration validation
- [ ] Run `cargo clippy --all-features --all-targets` and fix all warnings
- [ ] Write documentation and usage examples
- [ ] Create example configuration files

### Future Enhancements
- [ ] Screen time tracking integration
- [ ] Screen lock control
- [ ] Deno runtime for HTTP/JavaScript tools
- [ ] Web UI for trigger and memory management
- [ ] Multi-user support
- [ ] Task subtasks and dependencies
- [ ] Task time tracking and estimates
- [ ] Recurring tasks

## Key Conventions

### Design Principles

- **No Hard-Coding**: All configuration values, paths, timeouts, schedules must be configurable
  - Load from config files with documented defaults
  - Use environment variables where appropriate
  - Avoid magic numbers and strings - use constants/enums
  
- **Loose Coupling**: Components communicate through well-defined interfaces
  - Use trait objects for extensibility (tools, LLM backends, storage)
  - IPC protocol should be version-agnostic with feature negotiation
  - Database schema changes should be handled via migrations
  - Tools should not depend on each other or specific implementations

### Code Quality

- **Strict Clippy Lints**: Enforced via `clippy.toml` configuration
  ```toml
  # clippy.toml
  warn-on-all-wildcard-imports = true
  disallowed-methods = []
  ```
  
  And in `Cargo.toml`:
  ```toml
  [workspace.lints.clippy]
  all = "warn"
  pedantic = "warn"
  nursery = "warn"
  cargo = "warn"
  unwrap_used = "deny"
  expect_used = "deny"
  panic = "deny"
  todo = "warn"
  unimplemented = "deny"
  ```

- **Pre-Commit Validation**: Always run before committing
  ```bash
  cargo clippy --all-features --all-targets -- -D warnings
  ```
  Only commit if all lints pass

### Implementation Guidelines

- **Error Handling**: Use `anyhow::Result` for application errors, `thiserror` for library errors
  - Never use `.unwrap()` or `.expect()` - use proper error propagation
  - Log errors with context using `tracing`
  
- **Async**: All I/O operations (database, LLM, IPC) must be async
  - Use `tokio::spawn` for concurrent tasks
  - Use `tokio::select!` for multiplexing futures
  
- **Tool Functions**: Always return `Pin<Box<dyn Future<Output = ToolResult> + Send>>` for consistency
  - Tools are registered at runtime, not compile-time
  - Each tool should be self-contained and testable
  
- **Configuration**: Load from `~/.config/ritsu/config.toml`, fall back to defaults
  - All defaults must be documented in example config
  - Support environment variable overrides
  
- **Logging**: Use `tracing` crate with spans for debugging trigger execution and LLM calls
  - Structured logging with contextual information
  - Different log levels per module configurable
  
- **Timestamps**: Always use UTC internally, convert to local time for display

- **Timeouts**: All blocking operations (user input, HTTP, LLM) must have timeouts; on user timeout, notify AI to decide next action

- **Focus Control**: Use platform-specific APIs (X11/Wayland on Linux) to launch and focus GUI windows

## CLI Usage Examples

### Daemon Management
```bash
ritsu start           # Start the server daemon
ritsu stop            # Stop the server
ritsu restart         # Restart the server
ritsu status          # Check server status
```

### Chat Interface
```bash
ritsu chat            # Open GUI chat window
ritsu send "Hello"    # Send quick message without opening GUI
```

### Task Management
```bash
ritsu task list                              # List all tasks
ritsu task list --status pending             # Filter by status
ritsu task add "Buy groceries" --priority high --due 2026-02-05
ritsu task update 123 --status completed
```

### Trigger Management
```bash
ritsu trigger list                           # List all triggers
ritsu trigger add "morning-alarm" --time "0 7 * * *"
ritsu trigger disable morning-alarm
ritsu trigger delete morning-alarm
```

### Memory Queries
```bash
ritsu memory --days 7                        # Last 7 days summary
ritsu notes --tag important                  # Filter notes by tag
```

## Development Workflow

### Before Every Commit

1. **Run clippy with all features**:
   ```bash
   cargo clippy --all-features --all-targets -- -D warnings
   ```

2. **Only commit if all lints pass** - no warnings or errors allowed

3. **Run tests** (when available):
   ```bash
   cargo test --all-features
   ```

4. **Commit without GPG signing**:
   ```bash
   git commit --no-gpg-sign -m "Your message"
   ```

### Code Review Checklist

- [ ] No hard-coded values (use config or constants)
- [ ] No `.unwrap()`, `.expect()`, or `panic!()`
- [ ] All errors properly propagated with context
- [ ] Timeouts on all blocking operations
- [ ] Loose coupling via traits/interfaces
- [ ] Async operations don't block
- [ ] Configuration documented in example config
- [ ] Clippy passes with all features

## References

- tokio async runtime: https://docs.rs/tokio
- rusqlite: https://docs.rs/rusqlite or sqlx: https://docs.rs/sqlx
- llm crate: https://docs.rs/llm
- serde: https://docs.rs/serde
- notify-rust (for system notifications): https://docs.rs/notify-rust
- iced (for GUI): https://docs.rs/iced
- cron parser (for trigger scheduling): https://docs.rs/cron
