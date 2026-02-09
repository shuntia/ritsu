# Ritsu Server Tools Reference

This file documents the dynamic tool registry and the built-in tools implemented in ritsu-server/src/tools.rs. It mirrors the metadata (name, description, tags, parameters) and summarizes runtime behavior, side effects, and example usages.

---

## Overview

- Tools are registered into a ToolRegistry and executed by name with a HashMap<String, String> of parameters.
- Each tool returns a ritsu_common::ToolResult (success flag, output string, optional error).
- Tool handlers are asynchronous and run inside Box::pin(async move { ... }).
- The ToolRegistry logs usage to a SQLite table (`tool_usage`) when a database connection is configured.

## Core types

- Tool: { name, description, tags, parameters: Vec<ToolParameter>, handler: ToolFunction }
- ToolParameter: { name, description, required: bool, param_type: String }
- ToolFunction: Arc<dyn Fn(HashMap<String, String>) -> ToolFuture + Send + Sync>
- ToolFuture: Pin<Box<dyn Future<Output = ToolResult> + Send>>

All parameters are passed as strings in the args HashMap; tools are responsible for parsing / validating values (e.g., integers, floats, booleans).

---

## Registered tools

The register_all_tools function registers the following built-in tools (11 total):

- notify_client
- create_note
- query_memory
- create_trigger
- analyze_now
- open_chat
- create_task
- update_task
- list_tasks
- set_preference
- wait

Each tool is documented below with parameters, behavior, return values, and examples.

---

### notify_client

- Description: Send a notification to the user's desktop/client (preferred: client daemon). Falls back to broadcasting a push notification.
- Tags: notification, ui
- Parameters:
  - title (string, required): Notification title.
  - message (string, required): Notification message body.
  - urgency (string, optional): one of `low`, `normal`, `urgent` (or `critical`). Defaults to `normal`.
- Behavior:
  - Validates required parameters `title` and `message`.
  - Maps urgency to NotificationUrgency enum (Low, Normal, Critical).
  - Sends a ServerToClientRequest::NotifyUser to the client daemon via state.send_to_client_daemon.
  - On failure to reach the client daemon, falls back to state.broadcast_push(ServerPush::Notification) and reports success with a fallback message.
- Return: ToolResult::success("Notification sent: <title>") on success; ToolResult::error on missing params or unrecoverable errors.
- Example args: { "title": "Reminder", "message": "Stand up break", "urgency": "low" }

---

### create_note

- Description: Create a persistent note for future reference (stored via MemoryManager).
- Tags: memory, note
- Parameters:
  - content (string, required): Note content.
  - tags (string, optional): Comma-separated tags (e.g., "work,meeting").
- Behavior:
  - Validates `content`.
  - Parses `tags` into Vec<String> by splitting on commas and trimming.
  - Calls memory.create_note(&content, &tags).await to persist the note.
- Return: ToolResult::success("Note created successfully with N tags") or ToolResult::error on failure.
- Example args: { "content": "Met with Alice about roadmap", "tags": "meeting,roadmap" }

---

### query_memory

- Description: Query past conversations, daily/monthly summaries, or notes from MemoryManager.
- Tags: memory, search
- Parameters:
  - type (string, required): One of `conversations`, `daily`, `monthly`, `notes`.
  - query (string, optional): Substring filter applied to content/summaries.
  - limit (string, optional): Maximum number of results (parseable to integer; default 10).
- Behavior:
  - Dispatches to appropriate MemoryManager methods depending on `type`:
    - `conversations`: memory.get_recent_conversations_days(30) and filters by query.
    - `daily`: memory.get_summaries("daily", 30).
    - `monthly`: memory.get_summaries("monthly", 12).
    - `notes`: memory.query_notes(limit).
  - Formats a readable textual response listing matched items.
- Return: ToolResult::success(formatted_text) or ToolResult::error("Unknown query type") / other errors.
- Example args: { "type": "notes", "query": "roadmap", "limit": "5" }

---

### create_trigger

- Description: Create custom reminder/notification triggers. Intended for meaningful, time-specific reminders.
- Tags: automation, trigger, reminder
- Parameters:
  - name (string, required): Unique trigger name (e.g., `morning_pr_review`).
  - schedule (string, required): Schedule format - can be `HH:MM` for daily, a number of seconds (interval), or a cron expression.
  - type (string, optional): `time` (daily HH:MM), `interval` (seconds), `cron` (cron expression), or `dynamic` (one-time). Defaults to `time`.
  - note (string, optional): Instructional note describing what AI should do when the trigger fires (used for custom triggers).
  - open_chat (string, optional): `'true'` to open chat window when triggered, otherwise notification-only.
  - urgency (string, optional): Notification urgency (`low`, `normal`, `critical`).
  - tag (string, optional): Tag for categorization.
  - description (string, optional): Short description of the trigger.
- Behavior:
  - Validates `name` and `schedule`.
  - Builds JSON metadata from optional parameters (note, open_chat, urgency, tag, description).
  - Inserts a row into the `triggers` database table as `created_by = 'ai'` and then reloads triggers via trigger_registry.load_from_database().await.
  - Notifies the trigger loop to reschedule via trigger_registry.notify_changed().
- Return: ToolResult::success on success or ToolResult::error on DB/migration errors.
- Example args: { "name": "standup_reminder", "schedule": "09:00", "type": "time", "note": "Send notification and open chat if no response" }

---

### analyze_now

- Description: Schedule an immediate analysis/compaction task (dynamic trigger) such as conversation analysis or tool-effectiveness analysis.
- Tags: analysis, immediate
- Parameters:
  - type (string, required): Analysis type: `conversation`, `pattern`, `reflection`, `tool_effectiveness` (default: `conversation`).
- Behavior:
  - Creates a dynamic trigger name using the current timestamp and calls trigger_registry.add_dynamic_trigger(...) with metadata { analysis_type }.
  - Returns success once the dynamic trigger is scheduled.
- Return: ToolResult::success("Analysis scheduled: <type>") or ToolResult::error on failure.
- Example args: { "type": "conversation" }

---

### open_chat

- Description: Request that the client open the chat GUI and optionally display an initial message.
- Tags: ui, interaction
- Parameters:
  - message (string, optional): Initial message to display in the chat.
  - urgency (string, optional): Urgency level (unused by send_to_client_daemon path but included for parity).
- Behavior:
  - Sends ServerToClientRequest::OpenChat to the client daemon via state.send_to_client_daemon.
  - On failure, falls back to broadcasting ServerPush::OpenChat via state.broadcast_push.
- Return: ToolResult::success("Chat window opened") or ToolResult::success("Chat window opened (fallback)") on fallback; ToolResult::error for other failures.
- Example args: { "message": "Time to review PRs" }

---

### create_task

- Description: Create a new task via TaskManager.
- Tags: task, productivity
- Parameters:
  - title (string, required): Task title.
  - priority (string, optional): `low`, `medium`, `high`, `urgent` (default: `medium`).
  - due_date (string, optional): Due date in `YYYY-MM-DD` format.
  - description (string, optional): Task description.
- Behavior:
  - Validates `title` and converts `priority` into ritsu_common::TaskPriority.
  - Calls task_manager.create_task(...) and returns the created task ID in the success message.
- Return: ToolResult::success("Task created with ID <id>: <title>") or ToolResult::error on failure.
- Example args: { "title": "Write release notes", "priority": "high", "due_date": "2026-02-10" }

---

### update_task

- Description: Update an existing task's status or priority via TaskManager.
- Tags: task, productivity
- Parameters:
  - id (string, required): Task ID (string number parsable to integer).
  - status (string, optional): `pending`, `in_progress`, `completed`, `cancelled`.
  - priority (string, optional): `low`, `medium`, `high`, `urgent`.
- Behavior:
  - Parses `id` as i64 and applies updates:
    - status: maps to ritsu_common::TaskStatus and calls task_manager.update_task_status.
    - priority: maps to ritsu_common::TaskPriority and calls task_manager.update_task_priority.
  - If neither status nor priority provided, returns an error.
- Return: ToolResult::success("Task <id> updated: <updates>") or ToolResult::error on parse/validation/manager errors.
- Example args: { "id": "42", "status": "completed" }

---

### list_tasks

- Description: List tasks with optional status/priority filters.
- Tags: task, productivity
- Parameters:
  - status (string, optional): Filter by status.
  - priority (string, optional): Filter by priority.
- Behavior:
  - Calls task_manager.list_tasks(status, priority) and returns a human-readable summary for each found task.
- Return: ToolResult::success("Found N tasks:\n<list>") or ToolResult::success("No tasks found matching the filters") or ToolResult::error on failure.
- Example args: { "status": "pending" }

---

### set_preference

- Description: Persist a user preference using PreferencesManager.
- Tags: preferences, memory
- Parameters:
  - category (string, required): Preference category (e.g., `schedule`, `communication`).
  - key (string, required): Preference key (e.g., `wake_time`).
  - value (string, required): Preference value.
- Behavior:
  - Calls preferences.set_preference(&category, &key, &value, 1.0, Some("user")).
- Return: ToolResult::success("Preference saved: category / key = value") or ToolResult::error on failure.
- Example args: { "category": "schedule", "key": "wake_time", "value": "07:00" }

---

### wait

- Description: Pause asynchronously for a specified duration (seconds).
- Tags: utility, time
- Parameters:
  - seconds (string, required): Float or integer number of seconds to wait (e.g., "0.5", "2").
- Behavior:
  - Parses `seconds` as f64, constructs a Duration, performs tokio::time::sleep(dur).await, then returns success.
- Return: ToolResult::success("Waited X seconds") or ToolResult::error on invalid parameter.
- Example args: { "seconds": "1.5" }

---

## Notes and implementation details

- All tools accept parameters as strings. Where numeric/boolean values are needed, the handler parses them and returns clear error messages if parsing fails.
- Handlers frequently attempt a best-effort strategy and provide fallbacks (e.g., notify_client and open_chat fall back to broadcasting push messages if the client daemon path fails).
- ToolRegistry.execute logs execution metadata (argument keys and lengths, execution time) using tracing::debug and records usage to the `tool_usage` DB table when a database connection is configured.
- Tool authors should ensure handlers avoid logging sensitive parameter values directly; registry.execute summarizes args by key and length for that reason.

---

If you need example client code (JSON payloads or helper wrappers) for invoking these tools from the AI prompt layer or tests, I can add a small snippet per tool.