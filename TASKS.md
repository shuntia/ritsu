# Ritsu Tasks

## Current Sprint: Chat UI Enhancement

### High Priority
- [x] Implement animated loading indicator (throbber) in chat GUI
- [x] Add side panel with view switcher (Chat, Sessions, Tasks, Memory)
- [ ] Implement session list view - show today's sessions (IPC call needed)
- [ ] Implement session loading - switch to previous conversation (IPC call needed)
- [ ] Implement task manager view (IPC calls needed)
  - [x] View tasks list with filtering (status, priority) - UI complete
  - [ ] Create new tasks from GUI
  - [ ] Update task status/priority
  - [ ] Delete tasks
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
- [ ] Fix trigger execution - no user notification/interaction when triggers fire
- [ ] Add IPC protocol for sessions/tasks data loading in GUI

## Technical Debt
- [ ] Consider switching from postcard to a more maintainable protocol
- [ ] Evaluate replacing tokio::select_all with better pattern
- [ ] Review database schema for optimization opportunities
- [ ] Add database connection pooling if needed
- [ ] Consider lazy loading for large conversation histories

---

**Last Updated**: 2026-02-05
**Version**: 0.2
