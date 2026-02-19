# ZeroAI (中文版)

[English](README.md) | 中文

一个基于 Rust 的 AI 模型代理和客户端库，支持多种 AI 提供商和 OAuth 认证。

## 项目概述

ZeroAI 是一个统一的 AI 模型接口库，提供了对多种 AI 提供商的支持，包括 OpenAI、Anthropic、Google Gemini、Qwen 等。它包含两个主要组件：

- **zeroai**: 核心库，提供统一的 AI 模型接口
- **zeroai-proxy**: HTTP 代理服务器和 TUI 配置工具

## 功能特性

### 支持的提供商

- **OpenAI**: GPT-4o, GPT-4o-mini, o1, o3-mini
- **OpenAI Codex**: GPT-5.2, GPT-5.2-codex, GPT-5.3-codex (OAuth)
- **Anthropic**: Claude 3.5 Sonnet 等 (API key 和 Setup Token)
- **Google Gemini**: Gemini 2.5 Pro 等
- **Qwen**: 通义千问 (API key 和 OAuth)
- **DeepSeek**: DeepSeek V3, DeepSeek R1
- **Xai**: Grok 3, Grok 3 Mini
- **Moonshot**: Kimi K2.5
- **Minimax**: MiniMax M2.1, M2.5
- **Xiaomi**: MiMo V2 Flash
- **OpenRouter**: 支持多种模型
- **Ollama**: 本地模型
- **vLLM**: 本地模型
- **HuggingFace**: HF 模型
- **GitHub Copilot**: GitHub Copilot
- **Amazon Bedrock**: AWS Bedrock
- **Cloudflare AI Gateway**: Cloudflare 网关
- **自定义 OpenAI 兼容端点**

### 认证方式

- **API Key**: 环境变量或配置文件
- **OAuth**: 设备授权流程 (Qwen Portal, OpenAI Codex, Anthropic Setup Token)
- **Setup Token**: Anthropic Claude Code 专用
- **环境变量嗅探**: 自动检测现有配置
- **配置文件管理**: `~/.zeroai/config.json`

### 模型管理

- **动态模型获取**: 支持 OpenAI 兼容的 `/v1/models` 端点
- **静态模型列表**: 为不支持动态获取的提供商提供预定义模型
- **模型映射**: 统一的模型 ID 格式 `<provider>/<model>`
- **模型元数据**: 上下文窗口、最大 token 数、推理支持等

### 思考/推理支持

- **Anthropic**: 支持 interleaved thinking 和 setup-token
- **Google/Gemini**: 支持 thinking budget
- **OpenAI**: 支持 o1, o3-mini 等推理模型
- **流式响应**: 支持思考内容的流式传输

### 工具调用支持

- **Anthropic**: Claude Code 工具映射
- **OpenAI**: 函数调用
- **Google**: 工具使用
- **统一接口**: 跨提供商的工具定义和调用

## 安装

### 从源码构建

```bash
# 克隆仓库
git clone https://github.com/hushhenry/zeroai.git
cd zeroai

# 构建
cargo build --release

# 或者直接运行
cargo run --bin zeroai-proxy -- config
```

### 运行代理服务器

```bash
# 启动 HTTP 代理服务器
cargo run --bin zeroai-proxy -- serve --port 8787

# 或使用编译后的二进制文件
./target/release/zeroai-proxy serve --port 8787
```

## CLI 命令

`zeroai-proxy` 二进制文件提供以下子命令：

### `serve` - 启动 HTTP 代理服务器

启动一个 OpenAI 兼容的 HTTP 代理服务器，将请求路由到配置的 AI 提供商。

**用法：**
```bash
zeroai-proxy serve [OPTIONS]

# 选项：
#   -p, --port <PORT>     监听端口 (默认: 8787)
#   --host <HOST>         绑定主机 (默认: 127.0.0.1)
```

**示例：**
```bash
# 启动服务器到默认端口 (8787)
zeroai-proxy serve

# 启动服务器到自定义端口
zeroai-proxy serve --port 9000

# 绑定到特定接口
zeroai-proxy serve --host 0.0.0.0 --port 8080
```

**API 端点：**
- `GET /v1/models` - 列出可用模型
- `POST /v1/chat/completions` - 聊天补全 (OpenAI 格式)
- `POST /v1/messages` - Anthropic Messages API 格式

**API 使用示例：**
```bash
# 列出模型
curl http://127.0.0.1:8787/v1/models

# 聊天补全
curl -X POST http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-api-key>" \
  -d '{
    "model": "openai/gpt-4o",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### `config` - 配置提供商 (TUI)

启动交互式 TUI 配置工具来设置提供商和认证。

**用法：**
```bash
zeroai-proxy config
```

**功能：**
- 浏览和选择 AI 提供商
- 配置认证方式：
  - API Key (环境变量或手动输入)
  - OAuth (设备授权流程)
  - Setup Token (Anthropic Claude Code)
- 管理每个提供商的启用模型
- 查看和编辑配置文件

**导航：**
- 使用方向键导航
- 按 `Enter` 选择
- 按 `a` 添加账户
- 按 `d` 删除账户
- 按 `q` 或 `Esc` 退出

### `auth-check` - 验证凭据

验证所有配置提供商的凭据，检查 API 连接性。

**用法：**
```bash
zeroai-proxy auth-check
```

**输出：**
- ✅ 提供商名称和模型数量
- ❌ 提供商名称和错误信息 (未授权/禁止访问)

**示例：**
```
Checking credentials for 3 provider(s)...

  ✅ openai (4 model(s))
  ✅ anthropic (2 model(s))
  ❌ qwen-portal: 401 Unauthorized / Forbidden
```

### `doctor` - 健康检查

对配置的模型运行健康检查以验证功能。

**用法：**
```bash
zeroai-proxy doctor [OPTIONS]

# 选项：
#   -m, --model <MODEL>   要检查的特定模型 (格式: <provider>/<model>)
```

**示例：**
```bash
# 检查所有启用的模型 (每个提供商一个)
zeroai-proxy doctor

# 检查特定模型
zeroai-proxy doctor --model openai/gpt-4o
```

**功能：**
1. 使用简单的聊天补全测试每个模型
2. 验证工具调用能力 (使用 `get_current_time` 工具)
3. 检查流式响应
4. 验证工具结果处理

**输出：**
```
📋 Checking openai/gpt-4o...
  Stream:     ✅ 128 tokens, stop=length
  Tool call:  ✅ Received
  Tool result: ✅ Processed
```

## 使用方法

### 1. 配置提供商

```bash
# 启动 TUI 配置工具
zeroai-proxy config
```

在 TUI 中：
- 选择提供商
- 选择认证方式 (API key / OAuth / Setup Token)
- 按照提示完成认证

### 2. 启动代理服务器

```bash
# 启动 HTTP 代理服务器
zeroai-proxy serve

# 或使用自定义设置
zeroai-proxy serve --host 0.0.0.0 --port 8080
```

### 3. 使用代理服务器

代理服务器提供 OpenAI 兼容的 API 端点：

```bash
# 代理服务器运行在 http://127.0.0.1:8787
# 使用方式与 OpenAI API 相同

# 示例：使用 curl
curl -X POST http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-api-key>" \
  -d '{
    "model": "openai/gpt-4o",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### 4. 验证配置

```bash
# 检查所有提供商的凭据
zeroai-proxy auth-check

# 对模型运行健康检查
zeroai-proxy doctor
```

### 5. 作为库使用

```rust
use zeroai::{AiClientBuilder, ProviderAuthInfo};

// 创建客户端
let client = AiClientBuilder::new()
    .with_provider("openai", "sk-...")
    .build()?;

// 聊天完成
let response = client.chat_completion(
    "openai/gpt-4o",
    vec![Message::user("Hello!")],
    None,
).await?;

println!("Response: {:?}", response.content);
```

## 项目结构

```
zeroai/
├── Cargo.toml              # 工作区配置
├── zeroai/                 # 核心库
│   ├── Cargo.toml
│   ├── src/
│   │   ├── auth/           # 认证管理
│   │   ├── client.rs       # AI 客户端
│   │   ├── mapper.rs       # 模型映射
│   │   ├── models/         # 模型管理
│   │   ├── oauth/          # OAuth 实现
│   │   ├── providers/      # 提供商实现
│   │   ├── types.rs        # 类型定义
│   │   └── lib.rs
│   └── tests/              # 单元测试
├── zeroai-proxy/           # 代理服务器和 TUI
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs         # CLI 入口
│   │   ├── server.rs       # HTTP 服务器
│   │   ├── config_tui.rs   # TUI 配置工具
│   │   └── doctor.rs       # 健康检查
│   └── tests/              # 集成测试
├── run_agent2.sh           # 示例脚本
├── REVIEW.md               # 与 rust-genai 的对比
└── README.md               # 本文档
```

## 配置文件

配置文件位于 `~/.zeroai/config.json`：

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

## 环境变量

支持的环境变量：

- `ANTHROPIC_API_KEY`: Anthropic API key
- `OPENAI_API_KEY`: OpenAI API key
- `DASHSCOPE_API_KEY`: 阿里云 DashScope API key
- `GOOGLE_API_KEY`: Google AI API key
- `DEEPSEEK_API_KEY`: DeepSeek API key
- `XAI_API_KEY`: Xai API key
- `MOONSHOT_API_KEY`: Moonshot API key
- `MINIMAX_API_KEY`: Minimax API key
- `XIAOMI_API_KEY`: Xiaomi MiMo API key
- `OPENROUTER_API_KEY`: OpenRouter API key

## 开发

### 运行测试

```bash
cargo test
```

### 格式化代码

```bash
cargo fmt
```

### 检查代码质量

```bash
cargo clippy
```

### 构建文档

```bash
cargo doc --open
```

## 贡献

欢迎贡献！请遵循以下步骤：

1. Fork 仓库
2. 创建功能分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'Add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 创建 Pull Request

## 许可证

本项目采用 MIT 许可证 - 详见 [LICENSE](LICENSE) 文件

## 相关项目

- [rust-genai](https://github.com/hushhenry/rust-genai) - 另一个 Rust AI 库，专注于思考/推理和工具调用
- [OpenClaw](https://github.com/openclaw/openclaw) - 个人 AI 助手平台

## 联系方式

- GitHub: [@hushhenry](https://github.com/hushhenry)
- Email: hush.henry@zohomail.com

---

**注意**: 本项目仍在开发中，API 可能会发生变化。请查看 [CHANGELOG](CHANGELOG.md) 获取最新更新。