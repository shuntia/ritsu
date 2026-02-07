# Tool Effectiveness Analysis

You are performing **bi-weekly tool effectiveness analysis** to evaluate how well the available tools are serving the user's needs.

## Your Task

Analyze the past 14 days of tool usage statistics and activity to determine:

1. **Tool Usage Frequency**: Which tools are most/least used
2. **Success Rates**: Which tools consistently succeed vs frequently fail
3. **User Interaction Patterns**: How the user engages with different tools
4. **Tool Relevance**: Whether current tools match user needs
5. **Performance Issues**: Any timeouts, errors, or reliability problems
6. **Missing Capabilities**: Gaps where tools could be helpful but don't exist

## Available Tools Reference

1. **notify_client** - Send system notifications
2. **create_note** - Store information in memory
3. **query_memory** - Retrieve notes and summaries
4. **create_trigger** - Schedule future actions
5. **analyze_now** - Trigger immediate analysis
6. **open_chat** - Launch chat interface
7. **create_task** - Create new tasks
8. **update_task** - Update task status
9. **list_tasks** - Query tasks
10. **set_preference** - Store user preferences

## Output Format

Provide a structured analysis covering:

**Usage Summary**: List tools by frequency (high/medium/low/unused)

**Performance Metrics**: Success rates, average response times, error patterns

**User Behavior**: How tools are being used effectively vs underutilized

**Effectiveness Assessment**: Which tools provide the most value

**Recommendations**: 2-3 specific suggestions:
- Tools to emphasize more
- Tools that need improvement
- New tools that could fill gaps
- Workflow optimizations

## Guidelines

- Base conclusions on actual usage data, not assumptions
- Highlight both successes and problems
- Consider user feedback and interaction context
- Identify opportunities for better tool integration
- Note any tools that are frequently attempted but fail
- Consider task management effectiveness if applicable
- Be honest about limitations

## Example Insights

- "create_note used 23 times with 100% success - highly effective for quick storage"
- "open_chat attempted 8 times but failed 5 times due to X11 focus issues"
- "notify_client underutilized - only 2 uses despite 15 opportunities for proactive notifications"
- "Tasks created but rarely updated - suggests task management workflow needs improvement"

Your analysis will inform future development priorities and may adjust how frequently tools are recommended.
