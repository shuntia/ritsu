# Ritsu Implementation TODO

# Ritsu Implementation TODO

## Phase 1: Tool Execution Pipeline ⚡ COMPLETE ✅
**Status: All core infrastructure implemented and building successfully**

- [x] 1.1: Add llm crate with tool calling support (FunctionBuilder/ParamBuilder)
- [x] 1.2: Replace deprecated bincode with postcard
- [x] 1.3: Create .config/system_prompt.md for AI personality
- [x] 1.4: Wire ToolRegistry to LlmClient (all 9 tools registered)
- [x] 1.5: Implement conversation loop with tool results (generate_with_tool_execution)
- [x] 1.6: Fix all API compatibility issues
- [ ] 1.7: Test end-to-end tool calling with ollama

**Achievements:**
- Unified LLM interface across OpenAI/Anthropic/Ollama
- Automatic tool execution loop (max 5 iterations)
- Tool results fed back to LLM as user messages
- System prompt loaded from config file at runtime
- IPC SendMessage handler fully implements tool execution flow

## Phase 2: Implement Tool Handlers 🔧
- [ ] 2.1: create_note - wire to MemoryManager::create_note()
- [ ] 2.2: query_memory - wire to MemoryManager query methods
- [ ] 2.3: create_task - wire to TaskManager::create_task()
- [ ] 2.4: update_task - wire to TaskManager::update_status/priority()
- [ ] 2.5: list_tasks - wire to TaskManager::list_tasks()
- [ ] 2.6: create_trigger - wire to TriggerRegistry::create_trigger()
- [ ] 2.7: notify_client - implement IPC push notification
- [ ] 2.8: open_chat - implement GUI launch command
- [ ] 2.9: analyze_now - trigger immediate idle analysis

## Phase 3: Task Context in Triggers 📋
- [ ] 3.1: Add task summary methods to TaskManager
- [ ] 3.2: Include task stats in daily_compaction prompt
- [ ] 3.3: Include task stats in monthly_reflection prompt
- [ ] 3.4: Include task stats in pattern_recognition prompt
- [ ] 3.5: Include task stats in morning_briefing prompt

## Phase 4: Tool Usage Tracking 📊
- [ ] 4.1: Create tool_usage table in database
- [ ] 4.2: Log all tool calls (name, args, result, timestamp)
- [ ] 4.3: Add tool usage queries to MemoryManager
- [ ] 4.4: Wire tool effectiveness trigger to real data
- [ ] 4.5: Add tool success/failure metrics

## Phase 5: Inactivity Tracking ⏱️
- [ ] 5.1: Add last_activity timestamp to server state
- [ ] 5.2: Update timestamp on: chat messages, GUI opens, commands
- [ ] 5.3: Check inactivity in trigger loop
- [ ] 5.4: Trigger inactivity analysis after 30 min idle
- [ ] 5.5: Compact recent conversations before user returns

## Phase 6: Conversation State Management 💬
- [ ] 6.1: Create conversation session tracking
- [ ] 6.2: Store multi-turn conversations with tool results
- [ ] 6.3: Implement conversation history in chat
- [ ] 6.4: Add conversation context to SendMessage handler

## Phase 7: Advanced Features 🚀
- [ ] 7.1: User preferences extraction from notes
- [ ] 7.2: Trigger suggestions based on patterns
- [ ] 7.3: Smart notification urgency detection
- [ ] 7.4: Task priority recommendations
- [ ] 7.5: Inter-trigger learning (share insights)

## Priority Order
1. **Phase 1** - WITHOUT THIS, AI CAN'T DO ANYTHING
2. **Phase 2** - Makes tools actually work
3. **Phase 3** - Adds context to triggers
4. **Phase 6** - Better conversation UX
5. **Phase 4** - Analytics/metrics
6. **Phase 5** - Proactive behavior
7. **Phase 7** - Nice-to-haves

## Testing Checklist
- [ ] Basic tool call (create_note)
- [ ] Tool call with tool result in conversation
- [ ] Multiple tools in sequence
- [ ] Task CRUD operations via AI
- [ ] Trigger creation via AI
- [ ] Memory queries via AI
- [ ] Notification sending
- [ ] Inactivity detection
- [ ] Tool usage analytics
- [ ] Multi-turn conversation state

## Current Blockers
- **BLOCKER**: Tool execution pipeline doesn't exist
- **BLOCKER**: LLM doesn't know what tools are available
- **BLOCKER**: Tool calls never get executed even if returned
