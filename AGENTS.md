# Ritsu - Self-Triggering AI Agent

**Ritsu** is an autonomous AI agent that can schedule and trigger itself arbitrarily. It maintains memory through SQLite, compacts conversations into summaries, and can execute tools based on context and scheduled triggers.

## Architecture Overview

### Components
1. **Server (ritsu-server)**: Long-running daemon that:
   - Manages the event loop and trigger scheduling
   - Maintains SQLite database for memory storage
   - Handles LLM interactions for decision-making
   - Executes scheduled tasks and tool calls
   - Listens for client connections

2. **CLI (ritsu-cli)**: Daemon management client that:
   - Starts/stops/restarts the server
   - Manages triggers (list, enable, disable, delete)
   - Queries memory and configuration
   - Provides command-line interface for administration

3. **GUI (ritsu-gui)**: Chat interface client that:
   - Provides iced-based chat UI for conversations
   - Connects to server via IPC
   - Displays conversation history
   - Handles system notifications from server

### Communication
- Server-client communication via IPC (Unix domain sockets or TCP)
- CLI can send admin commands, query status, manage triggers
- GUI connects to server for chat and receives push notifications
- Server pushes system notifications to all connected clients

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

[memory]
daily_rotation_days = 40
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
│   │   └── ipc.rs           # Server-client communication
├── ritsu-cli/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs          # CLI entry point
│   │   └── commands.rs      # Admin commands
├── ritsu-gui/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs          # GUI entry point
│   │   ├── chat.rs          # Chat interface with iced
│   │   ├── notify.rs        # System notifications
│   │   └── ipc.rs           # GUI-server communication
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
- [ ] Implement SQLite schema and database module
- [ ] Create basic IPC protocol between server and client
- [ ] Implement server daemon with tokio runtime

### Phase 2: Memory System
- [ ] Implement daily conversation storage
- [ ] Build daily compaction logic (end-of-day summary)
- [ ] Build monthly compaction logic
- [ ] Implement 40-day rotation cleanup task
- [ ] Create system prompt storage and retrieval
- [ ] Implement idle analysis tables

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

### Phase 5: Trigger System & Idle Analysis
- [ ] Design trigger data structures and persistence
- [ ] Implement time-based triggers with tokio
- [ ] Implement interval-based triggers
- [ ] Build trigger execution loop with `select_all`
- [ ] Implement idle analysis triggers (daily, weekly, bi-weekly, monthly)
- [ ] Build idle analysis logic for each analysis type
- [ ] Implement system prompt composition and updates

### Phase 6: CLI Client
- [ ] Build CLI for daemon management (start, stop, status)
- [ ] Add trigger management commands (list, enable, disable, delete)
- [ ] Add memory query commands
- [ ] Add configuration commands

### Phase 7: GUI Client (iced)
- [ ] Set up iced application structure
- [ ] Build chat interface (message list + input field)
- [ ] Implement IPC connection to server
- [ ] Display conversation history from memory
- [ ] Handle real-time message streaming from LLM
- [ ] Implement system notification handling (notify-rust)
- [ ] Add connection status indicator

### Phase 8: Polish & Testing
- [ ] Write integration tests for trigger execution
- [ ] Test memory compaction and rotation
- [ ] Add configuration validation
- [ ] Write documentation and usage examples

### Future Enhancements
- [ ] Screen time tracking integration
- [ ] Screen lock control
- [ ] Deno runtime for HTTP/JavaScript tools
- [ ] Web UI for trigger and memory management
- [ ] Multi-user support

## Key Conventions

- **Error Handling**: Use `anyhow::Result` for application errors, `thiserror` for library errors
- **Async**: All I/O operations (database, LLM, IPC) must be async
- **Tool Functions**: Always return `Pin<Box<dyn Future<Output = ToolResult> + Send>>` for consistency
- **Configuration**: Load from `~/.config/ritsu/config.toml`, fall back to defaults
- **Logging**: Use `tracing` crate with spans for debugging trigger execution and LLM calls
- **Timestamps**: Always use UTC internally, convert to local time for display

## References

- tokio async runtime: https://docs.rs/tokio
- rusqlite: https://docs.rs/rusqlite or sqlx: https://docs.rs/sqlx
- llm crate: https://docs.rs/llm
- serde: https://docs.rs/serde
- notify-rust (for system notifications): https://docs.rs/notify-rust
- iced (for GUI): https://docs.rs/iced
- cron parser (for trigger scheduling): https://docs.rs/cron
