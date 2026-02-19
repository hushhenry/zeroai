# ZeroAI

[中文](README.zh.md) | English

A Rust-based AI model proxy and client library with support for multiple AI providers and OAuth authentication.

## Project Overview

ZeroAI is a unified AI model interface library that provides support for multiple AI providers, including OpenAI, Anthropic, Google Gemini, Qwen, and more. It consists of two main components:

- **zeroai**: Core library providing a unified AI model interface
- **zeroai-proxy**: HTTP proxy server and TUI configuration tool

## Features

### Supported Providers

- **OpenAI**: GPT-4o, GPT-4o-mini, o1, o3-mini
- **OpenAI Codex**: GPT-5.2, GPT-5.2-codex, GPT-5.3-codex (OAuth)
- **Anthropic**: Claude 3.5 Sonnet, etc. (API key and Setup Token)
- **Google Gemini**: Gemini 2.5 Pro, etc.
- **Qwen**: Tongyi Qianwen (API key and OAuth)
- **DeepSeek**: DeepSeek V3, DeepSeek R1
- **Xai**: Grok 3, Grok 3 Mini
- **Moonshot**: Kimi K2.5
- **Minimax**: MiniMax M2.1, M2.5
- **Xiaomi**: MiMo V2 Flash
- **OpenRouter**: Multiple models support
- **Ollama**: Local models
- **vLLM**: Local models
- **HuggingFace**: HF models
- **GitHub Copilot**: GitHub Copilot
- **Amazon Bedrock**: AWS Bedrock
- **Cloudflare AI Gateway**: Cloudflare gateway
- **Custom OpenAI-compatible endpoints**

### Authentication Methods

- **API Key**: Environment variables or configuration file
- **OAuth**: Device authorization flow (Qwen Portal, OpenAI Codex, Anthropic Setup Token)
- **Setup Token**: Anthropic Claude Code specific
- **Environment variable sniffing**: Automatic detection of existing configurations
- **Configuration file management**: `~/.zeroai/config.json`

### Model Management

- **Dynamic model fetching**: Supports OpenAI-compatible `/v1/models` endpoints
- **Static model lists**: Predefined models for providers that don't support dynamic fetching
- **Model mapping**: Unified model ID format `<provider>/<model>`
- **Model metadata**: Context window, max tokens, reasoning support, etc.

### Thinking/Reasoning Support

- **Anthropic**: Supports interleaved thinking and setup-token
- **Google/Gemini**: Supports thinking budget
- **OpenAI**: Supports reasoning models like o1, o3-mini
- **Streaming responses**: Supports streaming of thinking content

### Tool Calling Support

- **Anthropic**: Claude Code tool mapping
- **OpenAI**: Function calling
- **Google**: Tool use
- **Unified interface**: Cross-provider tool definitions and calls

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/hushhenry/zeroai.git
cd zeroai

# Build
cargo build --release

# Or run directly
cargo run --bin zeroai-proxy -- config
```

### Running the Proxy Server

```bash
# Start HTTP proxy server
cargo run --bin zeroai-proxy -- serve --port 8787

# Or use compiled binary
./target/release/zeroai-proxy serve --port 8787
```

## CLI Commands

The `zeroai-proxy` binary provides the following subcommands:

### `serve` - Start HTTP Proxy Server

Start an OpenAI-compatible HTTP proxy server that routes requests to configured AI providers.

**Usage:**
```bash
zeroai-proxy serve [OPTIONS]

# Options:
#   -p, --port <PORT>     Port to listen on (default: 8787)
#   --host <HOST>         Host to bind to (default: 127.0.0.1)
```

**Examples:**
```bash
# Start server on default port (8787)
zeroai-proxy serve

# Start server on custom port
zeroai-proxy serve --port 9000

# Bind to specific interface
zeroai-proxy serve --host 0.0.0.0 --port 8080
```

**API Endpoints:**
- `GET /v1/models` - List available models
- `POST /v1/chat/completions` - Chat completion (OpenAI format)
- `POST /v1/messages` - Anthropic Messages API format

**Example API Usage:**
```bash
# List models
curl http://127.0.0.1:8787/v1/models

# Chat completion
curl -X POST http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-api-key>" \
  -d '{
    "model": "openai/gpt-4o",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### `config` - Configure Providers (TUI)

Start an interactive TUI configuration tool to set up providers and authentication.

**Usage:**
```bash
zeroai-proxy config
```

**Features:**
- Browse and select AI providers
- Configure authentication methods:
  - API Key (environment variable or manual entry)
  - OAuth (device authorization flow)
  - Setup Token (Anthropic Claude Code)
- Manage enabled models for each provider
- View and edit configuration file

**Navigation:**
- Use arrow keys to navigate
- Press `Enter` to select
- Press `a` to add account
- Press `d` to delete account
- Press `q` or `Esc` to quit

### `auth-check` - Validate Credentials

Validate credentials for all configured providers by checking API connectivity.

**Usage:**
```bash
zeroai-proxy auth-check
```

**Output:**
- ✅ Provider name with number of models
- ❌ Provider name with error message (Unauthorized/Forbidden)

**Example:**
```
Checking credentials for 3 provider(s)...

  ✅ openai (4 model(s))
  ✅ anthropic (2 model(s))
  ❌ qwen-portal: 401 Unauthorized / Forbidden
```

### `doctor` - Health Check

Run health checks on configured models to verify functionality.

**Usage:**
```bash
zeroai-proxy doctor [OPTIONS]

# Options:
#   -m, --model <MODEL>   Specific model to check (format: <provider>/<model>)
```

**Examples:**
```bash
# Check all enabled models (one per provider)
zeroai-proxy doctor

# Check specific model
zeroai-proxy doctor --model openai/gpt-4o
```

**What it does:**
1. Tests each model with a simple chat completion
2. Verifies tool calling capability (uses `get_current_time` tool)
3. Checks streaming response
4. Validates tool result processing

**Output:**
```
📋 Checking openai/gpt-4o...
  Stream:     ✅ 128 tokens, stop=length
  Tool call:  ✅ Received
  Tool result: ✅ Processed
```

## Usage

### 1. Configure Providers

```bash
# Start TUI configuration tool
zeroai-proxy config
```

In the TUI:
- Select a provider
- Choose authentication method (API key / OAuth / Setup Token)
- Follow the prompts to complete authentication

### 2. Start Proxy Server

```bash
# Start HTTP proxy server
zeroai-proxy serve

# Or with custom settings
zeroai-proxy serve --host 0.0.0.0 --port 8080
```

### 3. Using the Proxy Server

The proxy server provides OpenAI-compatible API endpoints:

```bash
# Proxy server runs on http://127.0.0.1:8787
# Usage is the same as OpenAI API

# Example: Using curl
curl -X POST http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-api-key>" \
  -d '{
    "model": "openai/gpt-4o",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### 4. Validate Configuration

```bash
# Check credentials for all providers
zeroai-proxy auth-check

# Run health checks on models
zeroai-proxy doctor
```

### 5. Using as a Library

```rust
use zeroai::{AiClientBuilder, ProviderAuthInfo};

// Create client
let client = AiClientBuilder::new()
    .with_provider("openai", "sk-...")
    .build()?;

// Chat completion
let response = client.chat_completion(
    "openai/gpt-4o",
    vec![Message::user("Hello!")],
    None,
).await?;

println!("Response: {:?}", response.content);
```

## Project Structure

```
zeroai/
├── Cargo.toml              # Workspace configuration
├── zeroai/                 # Core library
│   ├── Cargo.toml
│   ├── src/
│   │   ├── auth/           # Authentication management
│   │   ├── client.rs       # AI client
│   │   ├── mapper.rs       # Model mapping
│   │   ├── models/         # Model management
│   │   ├── oauth/          # OAuth implementations
│   │   ├── providers/      # Provider implementations
│   │   ├── types.rs        # Type definitions
│   │   └── lib.rs
│   └── tests/              # Unit tests
├── zeroai-proxy/           # Proxy server and TUI
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs         # CLI entry point
│   │   ├── server.rs       # HTTP server
│   │   ├── config_tui.rs   # TUI configuration tool
│   │   └── doctor.rs       # Health checks
│   └── tests/              # Integration tests
├── run_agent2.sh           # Example script
├── REVIEW.md               # Comparison with rust-genai
└── README.md               # This document (English)
└── README.zh.md            # Chinese version
```

## Configuration

Configuration file is located at `~/.zeroai/config.json`:

```json
{
  "providers": {
    "openai": {
      "api_key": "sk-...",
      "enabled_models": ["gpt-4o", "gpt-4o-mini"]
    },
    "anthropic": {
      "api_key": "sk-ant-...",
      "enabled_models": ["claude-3-5-sonnet-20241022"]
    },
    "qwen-portal": {
      "access_token": "...",
      "refresh_token": "...",
      "expires": 1234567890
    }
  }
}
```

## Environment Variables

Supported environment variables:

- `ANTHROPIC_API_KEY`: Anthropic API key
- `OPENAI_API_KEY`: OpenAI API key
- `DASHSCOPE_API_KEY`: Alibaba Cloud DashScope API key
- `GOOGLE_API_KEY`: Google AI API key
- `DEEPSEEK_API_KEY`: DeepSeek API key
- `XAI_API_KEY`: Xai API key
- `MOONSHOT_API_KEY`: Moonshot API key
- `MINIMAX_API_KEY`: Minimax API key
- `XIAOMI_API_KEY`: Xiaomi MiMo API key
- `OPENROUTER_API_KEY`: OpenRouter API key

## Development

### Run Tests

```bash
cargo test
```

### Format Code

```bash
cargo fmt
```

### Check Code Quality

```bash
cargo clippy
```

### Build Documentation

```bash
cargo doc --open
```

## Contributing

Contributions are welcome! Please follow these steps:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Create a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Related Projects

- [rust-genai](https://github.com/hushhenry/rust-genai) - Another Rust AI library focused on thinking/reasoning and tool calls
- [OpenClaw](https://github.com/openclaw/openclaw) - Personal AI assistant platform

## Contact

- GitHub: [@hushhenry](https://github.com/hushhenry)
- Email: hush.henry@zohomail.com

---

**Note**: This project is under active development and the API may change. Please check [CHANGELOG](CHANGELOG.md) for the latest updates.