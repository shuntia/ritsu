# Memory Compaction Instructions

## Purpose

Memory compaction transforms raw conversation data into structured, searchable summaries. This happens daily at 2 AM and reduces memory footprint while preserving important information.

## Daily Compaction Process (2 AM)

### Input
- All conversations from the previous day (from `daily_conversations` table)
- Previous day's summary (for context continuity)
- Task context (active tasks for the day)

### Your Task

1. **Read all conversations** for the day
2. **Extract key information**:
   - Main topics discussed
   - Decisions made
   - Action items identified
   - User preferences expressed or implied
   - Problems solved
   - Questions asked/answered
   - Emotional tone (if relevant)

3. **Create a comprehensive summary** that includes:
   - **Overview**: 1-2 sentence summary of the day
   - **Key Topics**: Bullet list of main conversation topics
   - **Action Items**: Things user needs to do (may become tasks)
   - **Preferences Noted**: Any user preferences discovered
   - **Context for Tomorrow**: Information relevant to continuing tomorrow

4. **Generate tags** for searchability (3-7 tags):
   - Topic-based: work, personal, technical, etc.
   - Activity-based: planning, problem-solving, brainstorming
   - Domain-based: coding, writing, research, etc.

### Output Format

```
# [Day of Week], [Date]

## Overview
[1-2 sentence high-level summary]

## Key Topics
- Topic 1: [brief description]
- Topic 2: [brief description]
- Topic 3: [brief description]

## Important Conversations
- [Significant exchange 1]
- [Significant exchange 2]

## Action Items & Follow-ups
- [Action item 1]
- [Follow-up question or task]

## Preferences & Patterns
- [Preference discovered]
- [Pattern observed]

## Context for Tomorrow
[Anything relevant for continuing conversations tomorrow]

Tags: [tag1, tag2, tag3, tag4, tag5]
```

### Guidelines

**What to Include:**
- Meaningful conversations (not "hello"/"goodbye")
- Decisions and their reasoning
- Problems and solutions discussed
- Tasks created or discussed
- User's goals and intentions
- Learning or insights gained

**What to Exclude:**
- Trivial small talk
- System status checks ("are you working?")
- Redundant information
- Overly detailed technical minutiae unless relevant

**Length:**
- Target: 300-500 words for a typical day
- Scale with activity: busy day = longer summary
- Be comprehensive but not exhaustive

**Tone:**
- Third-person perspective: "User discussed..." not "You discussed..."
- Factual and neutral
- Highlight user's voice and preferences

### Special Cases

**No Conversations:**
```
# [Day of Week], [Date]

## Overview
No conversations recorded for this day.

Tags: [no-activity]
```

**Very Busy Day (50+ exchanges):**
- Group similar topics together
- Focus on high-level themes
- Create sub-sections if needed
- May exceed 500 words if justified

**Sensitive Information:**
- Summarize without revealing sensitive details
- Use general terms: "personal matter" instead of specifics
- Respect user privacy even in private summaries

## Monthly Compaction Process (Last day of month, 5 AM)

### Input
- All daily summaries for the month
- Previous month's summary (for continuity)
- Task completion stats for the month

### Your Task

1. **Read all daily summaries** for the month
2. **Identify monthly themes**:
   - Recurring topics across multiple days
   - Progress on long-term projects
   - Behavioral patterns
   - Productivity trends
   - Goal progression

3. **Create monthly summary** that includes:
   - **Overview**: 2-3 sentence summary of the month
   - **Major Themes**: What dominated the month?
   - **Progress & Achievements**: Completed projects, milestones
   - **Patterns Observed**: Work habits, preferences, schedules
   - **Unresolved Items**: Things that need continued attention
   - **Month-to-Month Changes**: How this month differed from previous

4. **Generate high-level tags** (3-5 tags):
   - Theme-based: productivity, learning, planning
   - Outcome-based: growth, maintenance, exploration

### Output Format

```
# [Month] [Year]

## Overview
[2-3 sentence summary of the entire month]

## Major Themes
1. [Theme 1]: [Description and days where it appeared]
2. [Theme 2]: [Description and days where it appeared]

## Progress & Achievements
- [Achievement 1]
- [Project completed]
- [Goal reached]

## Patterns & Insights
- [Pattern 1]: [What it means]
- [Preference 2]: [Consistency observed]

## Challenges & Blockers
- [Challenge faced]
- [Area of difficulty]

## Looking Forward
[What to carry into next month, unresolved items]

## Statistics
- Days with activity: X/30
- Total conversations: ~XXX
- Tasks completed: XX
- Most active topic: [topic]

Tags: [tag1, tag2, tag3, tag4]
```

### Guidelines for Monthly Summaries

- **Be synthesizing**, not just concatenating daily summaries
- **Look for trends**, not just events
- **Identify growth** and changes over time
- **Note cyclical patterns** (weekly routines, monthly cycles)
- **Connect dots** between seemingly unrelated topics

## After Compaction

### Daily Compaction Follow-up Actions

Based on the day's summary, you may:

1. **Create tasks** for clear action items not already tracked
2. **Set preferences** for newly discovered user preferences
3. **Create notes** for important reference information
4. **Create triggers** for follow-ups or reminders

**Example:**
```
Summary revealed user wants to start morning exercise routine
→ create_trigger("morning_exercise_reminder", "06:30", type="time")
→ set_preference("routine", "morning_exercise", "yes")
→ create_task("Buy exercise mat", priority="medium")
```

### Monthly Compaction Follow-up Actions

Based on monthly patterns:

1. **Update system prompt** with long-term preferences
2. **Create recurring triggers** for established routines
3. **Note behavioral patterns** for better assistance
4. **Identify opportunities** for better tool usage

## Error Handling

If compaction fails or conversation data is corrupted:

1. **Don't panic** - log the error
2. **Attempt partial summary** with available data
3. **Note the issue** in the summary
4. **Create reminder** to investigate later

## Quality Checks

Before saving the summary:

- [ ] Is it searchable? (Good tags, clear sections)
- [ ] Is it comprehensive? (Covers all important topics)
- [ ] Is it concise? (Not unnecessarily verbose)
- [ ] Is it useful? (Helps recall the day/month accurately)
- [ ] Is it respectful? (Protects user privacy)

## Key Reminders

1. **Summaries are for future you** - Write for searchability and recall
2. **Quality over quantity** - Better to miss trivial details than include noise
3. **Preserve context** - Future conversations may reference past summaries
4. **Respect privacy** - Even in private summaries, be discreet
5. **Enable action** - Summaries should enable follow-up actions

Remember: Good compaction makes Ritsu smarter over time!
