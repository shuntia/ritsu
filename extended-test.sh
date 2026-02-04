#!/bin/bash
set -e

echo "=== Ritsu Extended Feature Test ==="
echo

export PATH="$(pwd)/target/debug:$PATH"

# Stop any running server
ritsu stop 2>/dev/null || true
sleep 1

# Start server
echo "1. Starting server..."
ritsu start
sleep 2

# Test trigger operations
echo "2. Testing trigger creation..."
ritsu trigger add morning_alarm --time 07:00
ritsu trigger add evening_reminder --time 18:30

echo "3. Listing all triggers..."
ritsu trigger list

echo "4. Disabling a trigger..."
ritsu trigger disable morning_alarm
ritsu trigger list | grep -q "morning_alarm" && echo "✗ Disabled trigger still shown" || echo "✓ Disabled trigger hidden"

echo "5. Deleting a trigger..."
ritsu trigger delete evening_reminder
ritsu trigger list | grep -q "evening_reminder" && echo "✗ Deleted trigger still shown" || echo "✓ Deleted trigger removed"

# Test memory queries
echo "6. Sending multiple messages..."
ritsu send "First message"
ritsu send "Second message"
ritsu send "Third message"

echo "7. Querying recent conversations..."
ritsu memory --days 7 | grep -q "First message" && echo "✓ Memory query works"

# Test task operations
echo "8. Creating multiple tasks..."
ritsu task add "Buy groceries" --priority high --due "2026-03-01"
ritsu task add "Write report" --priority medium --due "2026-02-28"
ritsu task add "Call dentist" --priority urgent --due "2026-02-10"

echo "9. Listing and updating tasks..."
ritsu task list
ritsu task update 1 --status completed
ritsu task list | grep -q "Completed" && echo "✓ Task status update works"

# Stop server
echo "10. Stopping server..."
ritsu stop

echo
echo "=== Extended test passed! ==="
