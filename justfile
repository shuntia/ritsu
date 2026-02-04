# Justfile for ritsu development
# Install just: https://github.com/casey/just

# Default recipe (list available recipes)
default:
    @just --list

# Build all packages
build:
    cargo build --all

# Build in release mode
build-release:
    cargo build --all --release

# Build just the server
build-server:
    cargo build --bin ritsu-server

# Run tests with nextest
test:
    cargo nextest run

# Run tests with output
test-verbose:
    cargo nextest run --no-capture

# Run tests in CI mode (no retries)
test-ci:
    cargo nextest run --profile ci

# Run integration tests
test-integration:
    cargo nextest run --profile integration

# Run standard cargo test
test-cargo:
    cargo test

# Check code without building
check:
    cargo check --all

# Run clippy lints
lint:
    cargo clippy --all -- -D warnings

# Format code
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Clean build artifacts
clean:
    cargo clean

# Start the server
run-server:
    cargo run --bin ritsu-server

# Start the server in release mode
run-server-release:
    cargo run --bin ritsu-server --release

# Run the CLI client
run-cli *ARGS:
    cargo run --bin ritsu -- {{ARGS}}

# Full check: format, lint, test
ci: fmt-check lint test-ci

# Development workflow: format, build, test
dev: fmt build test

# Watch for changes and run tests
watch-test:
    cargo watch -x 'nextest run'

# Watch for changes and run server
watch-server:
    cargo watch -x 'run --bin ritsu-server'
