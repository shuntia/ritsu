# Ritsu Tasks

## Current Sprint: Trigger System Refactoring & Custom Triggers

### High Priority - Trigger Cleanup (IN PROGRESS)
- [x] Remove biweekly_tools trigger from builtin registration
- [x] Remove inactivity_analysis trigger from builtin registration  
- [x] Delete tools.md prompt file (no longer needed)
- [x] Remove tools analysis case from execute_idle_analysis()
- [ ] Update memory.rs to remove "tools" from load_context_prompt()
- [ ] Run clippy and commit trigger cleanup

### High Priority - Custom Trigger Support (TODO)
- [ ] Design CustomAction trigger metadata structure
  - [ ] message: String (notification/chat message)
  - [ ] open_chat: bool (whether to open GUI)
  - [ ] urgency: String (low/normal/critical)
  - [ ] repeat: Option<CronExpression> (for recurring triggers)
- [ ] Add cron expression parsing (use cron crate)
- [ ] Add TriggerType::Cron(String) variant
- [ ] Add "custom" analysis_type handler in execute_idle_analysis()
- [ ] Update create_trigger tool to support:
  - [ ] message parameter
  - [ ] open_chat parameter
  - [ ] urgency parameter
  - [ ] cron parameter for repeating triggers
- [ ] Return future/handle for awaiting trigger execution
- [ ] Ensure trigger persistence across restarts

### High Priority - Prompt Updates
- [ ] Update system_base.md with custom trigger guidelines
  - [ ] When to create triggers (meaningful, user-requested)
  - [ ] When NOT to create triggers (ephemeral, trivial)
  - [ ] Examples of good vs bad triggers
- [ ] Update background.md with trigger decision criteria
- [ ] Add trigger meaningfulness evaluation prompt

### Medium Priority - GUI Feature Completion
- [x] Implement animated loading indicator (throbber) in chat GUI
- [x] Implement streaming responses - FULLY IMPLEMENTED
- [x] Implement session loading - switch to previous conversation
- [x] Implement task status change and delete in GUI
- [x] Implement task create dialog in GUI
- [x] Implement client daemon to handle all GUI interactions
- [ ] Update server tools to send requests to client daemon
- [ ] End-to-end testing
- [ ] Add message history scrolling and pagination

### Low Priority
- [ ] Add auto-scroll to latest message
- [ ] Add configuration UI for preferences
- [ ] Add trigger management UI
- [ ] Add message search functionality
- [ ] Add themes/dark mode toggle

## Phase 8: Polish & Testing (Remaining)
- [x] Fix all clippy lints with strict warnings
- [x] Implement graceful shutdown with signal handling
- [ ] Write integration tests for trigger execution
- [ ] Test memory compaction and rotation
- [ ] Test custom trigger creation and execution
- [ ] Test cron scheduling
- [ ] Write documentation and usage examples

## Bug Fixes
- [x] Fixed query_memory tool - use correct column names
- [x] Enhanced AI context - always provide current time and task summary
- [ ] Fix trigger execution - no user notification/interaction when triggers fire

## Technical Debt
- [ ] Consider switching from postcard to a more maintainable protocol
- [ ] Evaluate replacing tokio::select_all with better pattern
- [ ] Review database schema for optimization opportunities
- [ ] Consider lazy loading for large conversation histories

---

**Last Updated**: 2026-02-07
**Version**: 0.3
