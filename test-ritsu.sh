#!/bin/bash
set -e

echo "=== Ritsu End-to-End Test ==="
echo

# Add target/debug to PATH so ritsu can find ritsu-server
export PATH="$(pwd)/target/debug:$PATH"

# Clean up any existing server
echo "1. Stopping any existing ritsu-server..."
./target/debug/ritsu stop 2>/dev/null || true
sleep 1

# Start server
echo "2. Starting ritsu-server..."
./target/debug/ritsu start
sleep 2

# Check status
echo "3. Checking server status..."
./target/debug/ritsu status

# Send a message
echo "4. Sending a test message..."
./target/debug/ritsu send "Hello, Ritsu! This is a test message."

# List tasks
echo "5. Listing tasks..."
./target/debug/ritsu task list

# Create a task
echo "6. Creating a task..."
./target/debug/ritsu task add "Test task" --priority high --due "2026-03-01"

# List tasks again
echo "7. Listing tasks after creation..."
./target/debug/ritsu task list

# Update task status
echo "8. Updating task status..."
./target/debug/ritsu task update 1 --status in_progress

# List triggers
echo "9. Listing triggers..."
./target/debug/ritsu trigger list

# Query memory
echo "10. Querying recent memory..."
./target/debug/ritsu memory --days 7

# Stop server
echo "11. Stopping server..."
./target/debug/ritsu stop

echo
echo "=== All tests passed! ==="
