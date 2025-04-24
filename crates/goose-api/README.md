# Goose API

An asynchronous REST API for interacting with Goose's AI agent capabilities.

## Overview

The goose-api crate provides an HTTP API interface to Goose's AI capabilities, enabling integration with other services and applications. It is designed as a daemon that can be run in the background, offering the same core functionality as the Goose CLI but accessible over HTTP.

## Installation

### Prerequisites

- Rust toolchain (cargo, rustc)
- Goose dependencies

### Building

```bash
# Navigate to the goose-api directory
cd crates/goose-api

# Build the project
cargo build

# For a production-optimized build
cargo build --release
```

## Configuration

Goose API supports configuration through both environment variables and a configuration file. The precedence order is:

1. Environment variables (highest priority)
2. Configuration file (lower priority)
3. Default values (lowest priority)

### Configuration File

Create a file named `config` (with no extension) in the directory where you run the goose-api. The format can be JSON, YAML, TOML, etc. (the `config` crate will detect the format automatically).

Example `config` file (YAML format):

```yaml
# API server configuration
host: 127.0.0.1
port: 8080
api_key: your_secure_api_key

# Provider configuration
provider: openai
model: gpt-4o
```

### Environment Variables

All configurations can be set using environment variables prefixed with `GOOSE_API_`.

```bash
# API server configuration
export GOOSE_API_HOST=0.0.0.0
export GOOSE_API_PORT=8080
export GOOSE_API_KEY=your_secure_api_key

# Provider configuration
export GOOSE_API_PROVIDER=openai
export GOOSE_API_MODEL=gpt-4o

# Provider-specific credentials (based on provider requirements)
export OPENAI_API_KEY=your_openai_api_key
export ANTHROPIC_API_KEY=your_anthropic_api_key
# etc.
```

## API Authentication

All API endpoints require authentication using an API key. The key should be provided in the `x-api-key` header.

Example:

```
x-api-key: your_secure_api_key
```

## Running the Server

```bash
# Run the server in development mode
cargo run

# Run the compiled binary directly
./target/debug/goose-api

# For production (with optimizations)
./target/release/goose-api
```

By default, the server runs on `127.0.0.1:8080`. You can modify this using configuration options.

## API Endpoints

### 1. Start a Session

**Endpoint**: `POST /session/start`

**Description**: Initiates a new session with Goose, providing an initial prompt.

**Request**:
- Headers:
  - Content-Type: application/json
  - x-api-key: [your-api-key]
- Body:
```json
{
  "prompt": "Your instruction to Goose"
}
```

**Response**:
```json
{
  "message": "Session started with prompt: Your instruction to Goose",
  "status": "success"
}
```

### 2. Reply to a Session

**Endpoint**: `POST /session/reply`

**Description**: Sends a follow-up message to an existing session.

**Request**:
- Headers:
  - Content-Type: application/json
  - x-api-key: [your-api-key]
- Body:
```json
{
  "prompt": "Your follow-up instruction"
}
```

**Response**:
```json
{
  "message": "Reply: Response from Goose",
  "status": "success"
}
```

### 3. List Extensions

**Endpoint**: `GET /extensions/list`

**Description**: Returns a list of available extensions.

**Request**:
- Headers:
  - x-api-key: [your-api-key]

**Response**:
```json
{
  "extensions": ["extension1", "extension2", "extension3"]
}
```

### 4. Get Provider Configuration

**Endpoint**: `GET /provider/config`

**Description**: Returns the current provider configuration.

**Request**:
- Headers:
  - x-api-key: [your-api-key]

**Response**:
```json
{
  "provider": "openai",
  "model": "gpt-4o"
}
```

## Examples

### Using cURL

```bash
# Start a session
curl -X POST http://localhost:8080/session/start \
  -H "Content-Type: application/json" \
  -H "x-api-key: your_secure_api_key" \
  -d '{"prompt": "Create a Python function to generate Fibonacci numbers"}'

# Reply to an ongoing session
curl -X POST http://localhost:8080/session/reply \
  -H "Content-Type: application/json" \
  -H "x-api-key: your_secure_api_key" \
  -d '{"prompt": "Add documentation to this function"}'

# List extensions
curl -X GET http://localhost:8080/extensions/list \
  -H "x-api-key: your_secure_api_key"

# Get provider configuration
curl -X GET http://localhost:8080/provider/config \
  -H "x-api-key: your_secure_api_key"
```

### Using Python

```python
import requests

API_URL = "http://localhost:8080"
API_KEY = "your_secure_api_key"
HEADERS = {
    "Content-Type": "application/json",
    "x-api-key": API_KEY
}

# Start a session
response = requests.post(
    f"{API_URL}/session/start", 
    headers=HEADERS, 
    json={"prompt": "Create a Python function to generate Fibonacci numbers"}
)
print(response.json())

# Reply to an ongoing session
response = requests.post(
    f"{API_URL}/session/reply", 
    headers=HEADERS, 
    json={"prompt": "Add documentation to this function"}
)
print(response.json())

# List extensions
response = requests.get(f"{API_URL}/extensions/list", headers=HEADERS)
print(response.json())

# Get provider configuration
response = requests.get(f"{API_URL}/provider/config", headers=HEADERS)
print(response.json())
```

## Troubleshooting

### Common Issues

1. **API Key Authentication Failure**:  
   Ensure the key in your request header matches the configured API key.

2. **Provider Configuration Issues**:  
   Make sure you've set the necessary environment variables for your chosen provider.

3. **Missing Required Keys**:  
   Check the server logs for messages about missing required provider configuration keys.

## Implementation Status (vs. Implementation Plan)

The current implementation includes the following features from the implementation plan:

✅ **Step 1-2**: Created goose-api crate with necessary dependencies  
✅ **Step 3-4**: Defined API endpoints with request/response structures  
✅ **Step 5**: Integration with goose core functionality  
✅ **Step 6**: Configuration via environment variables and config file  
✅ **Step 9**: API Key authentication  

🟡 **Step 7**: Extension loading mechanism (partial implementation)  
🟡 **Step 8**: MCP support (partial implementation)  
✅ **Step 10**: Documentation  
❌ **Step 11**: Tests (not yet implemented)  

## Future Work

- Extend session management capabilities
- Add more comprehensive error handling
- Implement unit and integration tests
- Complete MCP integration
- Add metrics and monitoring
- Add OpenAPI documentation generation