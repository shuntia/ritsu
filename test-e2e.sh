#!/bin/bash
# E2E Test Runner for Ritsu
#
# Usage:
#   ./test-e2e.sh              # Run all e2e tests
#   ./test-e2e.sh ping          # Run specific test
#   ./test-e2e.sh --build       # Build first, then test

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== Ritsu E2E Test Suite ===${NC}"

# Build binaries if requested
if [[ "$1" == "--build" ]]; then
    echo -e "${YELLOW}Building binaries...${NC}"
    cargo build --bin ritsu --bin ritsu-server
    shift
fi

# Check if binaries exist
if [[ ! -f "target/debug/ritsu" ]] || [[ ! -f "target/debug/ritsu-server" ]]; then
    echo -e "${RED}Error: Binaries not found. Run with --build flag or run 'cargo build' first${NC}"
    exit 1
fi

# Check if Ollama is running (optional but recommended)
if ! curl -s http://localhost:11434/api/tags >/dev/null 2>&1; then
    echo -e "${YELLOW}Warning: Ollama doesn't seem to be running. Some tests may fail.${NC}"
    echo "Start Ollama with: ollama serve"
    echo ""
fi

# Clean up any leftover processes and sockets
echo "Cleaning up previous test artifacts..."
pkill -f "ritsu-server" 2>/dev/null || true
pkill -f "ritsu.*start" 2>/dev/null || true
rm -f /tmp/ritsu*.sock /tmp/ritsu_test_* 2>/dev/null || true
sleep 1

# Run tests
if [[ -n "$1" ]]; then
    # Run specific test
    TEST_NAME="$1"
    echo -e "${GREEN}Running test: ${TEST_NAME}${NC}"
    cargo test --test e2e_test "test_${TEST_NAME}" -- --ignored --nocapture --test-threads=1
else
    # Run all tests sequentially
    echo -e "${GREEN}Running all E2E tests...${NC}"
    cargo test --test e2e_test -- --ignored --nocapture --test-threads=1
fi

EXIT_CODE=$?

# Cleanup after tests
echo ""
echo "Cleaning up..."
pkill -f "ritsu-server" 2>/dev/null || true
pkill -f "ritsu.*start" 2>/dev/null || true
rm -f /tmp/ritsu*.sock 2>/dev/null || true

if [[ $EXIT_CODE -eq 0 ]]; then
    echo -e "${GREEN}✓ All tests passed${NC}"
else
    echo -e "${RED}✗ Some tests failed${NC}"
fi

exit $EXIT_CODE
