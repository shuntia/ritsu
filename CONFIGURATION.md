# Ritsu Configuration Guide

Ritsu uses two configuration files to control server and client behavior.

## Server Configuration

**Location**: `~/.config/ritsu/config.toml`  
**Example**: `config.toml.example`

### LLM Configuration

```toml
[llm]
default_backend = "ollama"
disable_streaming = false
disable_tools = false

[[llm.backends]]
name = "ollama"
endpoint = "http://localhost:11434"
model = "llama3.2:3b"
```

- **default_backend**: Which LLM backend to use
- **disable_streaming**: Disable token-by-token streaming
- **disable_tools**: Disable tool calling
- **backends**: List of available LLM providers

### Server Configuration

```toml
[server]
socket_path = "/tmp/ritsu.sock"
database_path = "~/.local/share/ritsu/ritsu.db"
client_binary_path = "/usr/bin/ritsu"
```

- **socket_path**: Unix domain socket for IPC
- **database_path**: SQLite database location
- **client_binary_path**: Path to ritsu client binary (for launching GUI)

### Memory Configuration

```toml
[memory]
daily_rotation_days = 40
```

- **daily_rotation_days**: Days to keep detailed conversation history before compacting

### Timeout Configuration

```toml
[timeouts]
user_response_seconds = 300
http_request_seconds = 30
llm_request_seconds = 120
```

- **user_response_seconds**: How long to wait for user responses (5 minutes)
- **http_request_seconds**: HTTP request timeout (for future HTTP tools)
- **llm_request_seconds**: LLM API request timeout

---

## Client Configuration

**Location**: `~/.config/ritsu/client.toml`  
**Example**: `client.toml.example`

### Socket Configuration

```toml
[sockets]
client_socket = "/tmp/ritsu-client.sock"
server_socket = "/tmp/ritsu.sock"
```

- **client_socket**: Where client daemon listens for GUI/CLI connections
- **server_socket**: Where to connect to ritsu-server

**Environment Overrides**:
- `RITSU_CLIENT_SOCKET`
- `RITSU_SERVER_SOCKET`

### Timeout Configuration

```toml
[timeouts]
connect_seconds = 5
request_seconds = 30
streaming_seconds = 120
```

- **connect_seconds**: Socket connection timeout
- **request_seconds**: Non-streaming request timeout
- **streaming_seconds**: Initial streaming response timeout

**Environment Overrides**:
- `RITSU_CONNECT_TIMEOUT`
- `RITSU_REQUEST_TIMEOUT`
- `RITSU_STREAMING_TIMEOUT`

### GUI Configuration

```toml
[gui]
window_width = 800
window_height = 600
font_size = 14.0
notifications_enabled = true
max_messages = 500
```

- **window_width**: GUI window width in pixels
- **window_height**: GUI window height in pixels
- **font_size**: Font size in points
- **notifications_enabled**: Enable desktop notifications
- **max_messages**: Maximum messages to display in chat

### Path Configuration

```toml
[paths]
session_file = "~/.local/share/ritsu/session.txt"
cache_dir = "~/.cache/ritsu"
```

- **session_file**: Where to store current session ID
- **cache_dir**: Directory for chat history cache

### Retry Configuration

```toml
[retry]
max_retries = 3
retry_delay_ms = 1000
exponential_backoff = true
```

- **max_retries**: Number of connection retry attempts
- **retry_delay_ms**: Delay between retries in milliseconds
- **exponential_backoff**: Use exponential backoff algorithm

---

## Environment Variables

### Server Environment Variables

None currently. All server config is in `config.toml`.

### Client Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RITSU_CLIENT_SOCKET` | `/tmp/ritsu-client.sock` | Client daemon socket path |
| `RITSU_SERVER_SOCKET` | `/tmp/ritsu.sock` | Server socket path |
| `RITSU_CONNECT_TIMEOUT` | `5` | Connection timeout (seconds) |
| `RITSU_REQUEST_TIMEOUT` | `30` | Request timeout (seconds) |
| `RITSU_STREAMING_TIMEOUT` | `120` | Streaming timeout (seconds) |

Environment variables always override config file settings.

---

## Configuration Priority

**Server**: 
1. Command-line arguments (`--socket`, `--database`)
2. Config file (`~/.config/ritsu/config.toml`)
3. Built-in defaults

**Client**:
1. Environment variables
2. Config file (`~/.config/ritsu/client.toml`)
3. Built-in defaults

---

## Example Configurations

### Minimal Server Config

```toml
[llm]
default_backend = "ollama"

[[llm.backends]]
name = "ollama"
endpoint = "http://localhost:11434"
model = "llama3.2:3b"
```

### Minimal Client Config

```toml
# Uses all defaults - no config file needed!
# Default sockets: /tmp/ritsu.sock and /tmp/ritsu-client.sock
```

### Custom Socket Paths

**Server** (`config.toml`):
```toml
[server]
socket_path = "/var/run/ritsu/server.sock"
```

**Client** (`client.toml`):
```toml
[sockets]
server_socket = "/var/run/ritsu/server.sock"
client_socket = "/var/run/ritsu/client.sock"
```

**Or via environment**:
```bash
export RITSU_SERVER_SOCKET="/var/run/ritsu/server.sock"
export RITSU_CLIENT_SOCKET="/var/run/ritsu/client.sock"
ritsu chat
```

### Large Window GUI

```toml
[gui]
window_width = 1200
window_height = 900
font_size = 16.0
max_messages = 1000
```

### Aggressive Retry

```toml
[retry]
max_retries = 10
retry_delay_ms = 500
exponential_backoff = true
```

---

## Testing Configuration

For E2E tests, environment variables are the easiest way to override paths:

```bash
export RITSU_SERVER_SOCKET="/tmp/test_server.sock"
export RITSU_CLIENT_SOCKET="/tmp/test_client.sock"
./test-e2e.sh
```

See `TESTING.md` for more details on test configuration.
