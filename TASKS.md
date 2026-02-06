# Ritsu Tasks

## Current Sprint: GUI Feature Completion

### High Priority
- [x] Implement animated loading indicator (throbber) in chat GUI
- [x] Add side panel with view switcher (Chat, Sessions, Tasks, Memory)
- [x] Make sidebar collapsible with hamburger menu (hidden by default)
- [x] Improve theming and styling for iced application
- [x] Add more GUI animations (sidebar slide, message fade-in)
- [x] Make system prompt configurable and context-aware
- [x] Fix AI behavior to prioritize chat over background tasks
- [x] **IMPLEMENT STREAMING RESPONSES** - FULLY IMPLEMENTED!
  - [x] Reference local llm crate as path dependency
  - [x] Add streaming support to LLM client (`generate_streaming()`)
  - [x] Stream tokens via ServerPush::MessageChunk messages
  - [x] **Token-by-token display in GUI**
    - [x] Pre-create assistant message before streaming
    - [x] Use iced::stream::channel to bridge tokio → futures channels
    - [x] MessageChunk events append to indexed message in real-time
    - [x] Visual updates as each token arrives from LLM
  - [x] Add typing indicator animation while streaming
  - [x] Ready for integration testing with live LLM backend
- [x] Implement session loading - switch to previous conversation **DONE**
  - [x] Add IPC protocol for GetConversationHistory
  - [x] Server handler using ConversationManager
  - [x] GUI loads and displays conversation turns
- [x] Implement task status change and delete in GUI **DONE**
  - [x] Status cycle buttons (pending → in_progress → completed)
  - [x] Delete button for tasks
  - [x] Reload task list after operations
- [ ] Complete task deletion backend
  - [x] UI delete button implemented
  - [ ] Add DeleteTask IPC request
  - [ ] Server-side delete handler
- [ ] Implement client daemon to handle all GUI interactions
  - [ ] Client daemon runs in background
  - [ ] GUI connects to client daemon (not server directly)
  - [ ] Client daemon proxies requests to server
  - [ ] Client daemon handles notifications and window management
- [ ] Make system prompt configurable (load from config/database)
- [ ] Support thinking/reasoning display
- [ ] Generate session titles automatically
- [x] Implement session list view - show today's sessions
- [x] Add typing indicator animation to chat
- [x] Add memory clear functionality for testing (`ritsu clear-memory`)
- [x] Add connection status indicator with retry countdown
- [x] Show "Connected"/"Reconnecting in Xs..." status in top bar
- [x] Auto-retry connection every 5 seconds on disconnect
- [x] Implement tasks list view (loads real data from server)
- [x] Code quality improvements and warning cleanup
- [x] Implement session loading - switch to previous conversation (IPC call) **DONE**
- [x] Implement task status change UI **DONE**
- [ ] Implement task create dialog in GUI
- [ ] Implement memory view - show notes and summaries
- [x] Implement memory view placeholder
- [ ] Add streaming responses from LLM (incremental display)
- [ ] Add connection status indicator in GUI
- [ ] Add message history scrolling and pagination
- [ ] Improve chat UI styling and layout

### Medium Priority
- [ ] Add auto-scroll to latest message
- [ ] Add timestamps to chat messages
- [ ] Add copy button for assistant messages
- [ ] Add keyboard shortcuts (Ctrl+Enter to send, etc.)
- [ ] Add configuration UI for preferences
- [ ] Add trigger management UI

### Low Priority
- [ ] Add message search functionality
- [ ] Add export chat history feature
- [ ] Add themes/dark mode toggle
- [ ] Add notification settings in GUI
- [ ] Add system tray integration

## Phase 8: Polish & Testing (Remaining)
- [x] Fix all clippy lints with strict warnings
- [x] Implement graceful shutdown with signal handling
- [x] Add halt command with confirmation
- [ ] Write integration tests for trigger execution
- [ ] Test memory compaction and rotation
- [ ] Test task management workflows
- [ ] Add configuration validation
- [ ] Write documentation and usage examples
- [ ] Create example configuration files

## Future Features
- [ ] Voice input/output support
- [ ] Multi-session management
- [ ] Collaborative sessions (multiple users)
- [ ] Plugin system for custom tools
- [ ] Web interface alternative to GUI
- [ ] Mobile companion app
- [ ] Advanced analytics dashboard

## Bug Fixes
- [x] Fixed query_memory tool - use correct column names (timestamp vs created_at)
- [x] Removed unused llm_old.rs file
- [x] Enhanced AI context - always provide current time and task summary
- [x] Fixed unused StreamExt import in gui.rs **DONE**
- [ ] Fix trigger execution - no user notification/interaction when triggers fire
- [ ] Add complete IPC protocol for task deletion

## Recent Progress (Session 2026-02-06)
- [x] Removed unused import warning
- [x] Implemented session loading with conversation history
- [x] Added task status change and delete UI
- [x] Created session summary documentation

## Technical Debt
- [ ] Consider switching from postcard to a more maintainable protocol
- [ ] Evaluate replacing tokio::select_all with better pattern
- [ ] Review database schema for optimization opportunities
- [ ] Add database connection pooling if needed
- [ ] Consider lazy loading for large conversation histories

---

**Last Updated**: 2026-02-05
**Version**: 0.2
