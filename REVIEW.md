# zeroai vs rust-genai: Comparison (Thinking/Reasoning & Tool Calls)

This document compares **zeroai** (this project) and **rust-genai** (`/home/hush/.openclaw/workspace/rust-genai`) with focus on (1) thinking/reasoning support and (2) tool/function-call support. File paths and code snippets are included for both projects.

---

## 1. Thinking / Reasoning Support

### 1.1 Type-level representation

**zeroai** (`zeroai/src/types.rs`):

- **Request:** `RequestOptions` has an optional `reasoning: Option<ThinkingLevel>` where `ThinkingLevel` is an enum:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
}
```

- **Response:** Assistant content can include a dedicated thinking block. `ContentBlock` has a `Thinking` variant and `ThinkingContent` holds the text and optional signature:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingContent {
    pub thinking: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text(TextContent),
    Thinking(ThinkingContent),
    Image(ImageContent),
    ToolCall(ToolCall),
}
```

- **Stream:** `StreamEvent` includes `ThinkingDelta(String)` for incremental thinking.

**rust-genai** (`rust-genai/src/chat/chat_options.rs`, `rust-genai/src/chat/content_part/common.rs`):

- **Request:** `ChatOptions` has richer reasoning controls:
  - `reasoning_effort: Option<ReasoningEffort>` with variants `None`, `Low`, `Medium`, `High`, `Budget(u32)`, `Minimal`
  - `verbosity: Option<Verbosity>` (Low/Medium/High) for e.g. OpenAI
  - `capture_reasoning_content: Option<bool>` for streaming
  - `normalize_reasoning_content: Option<bool>` for normalizing e.g. `<think>...</think>` blocks

```rust
// chat_options.rs
pub reasoning_effort: Option<ReasoningEffort>,
pub verbosity: Option<Verbosity>,
pub capture_reasoning_content: Option<bool>,
pub normalize_reasoning_content: Option<bool>,
```

- **Response:** Content uses `ContentPart`. There is no separate “thinking content” block; thinking is represented as `ThoughtSignature(String)` (a standalone part, not full thinking text in the normalized content):

```rust
// content_part/common.rs
pub enum ContentPart {
    Text(String),
    Binary(Binary),
    ToolCall(ToolCall),
    ToolResponse(ToolResponse),
    ThoughtSignature(String),
    Custom(CustomPart),
}
```

- **Stream:** Uses `InterStreamEvent::ReasoningChunk(thinking)` and `InterStreamEvent::ThoughtSignatureChunk(signature)` (see streamer below). `ChatResponse` can carry `reasoning_content: Option<String>` (concatenated thinking from non-streaming).

**Summary:** zeroai models thinking as a first-class `Thinking(ThinkingContent)` block with optional signature and exposes `ThinkingDelta` on the stream. rust-genai uses `ThoughtSignature` in content and separates “reasoning content” capture (stream/non-stream) and richer request-time knobs (effort, verbosity, budget).

---

### 1.2 Anthropic: request payload (thinking budget / effort)

**zeroai** (`zeroai/src/providers/anthropic.rs`):

- Uses beta header `interleaved-thinking-2025-05-14` when the key looks like a setup-token (`sk-ant-sid`). No explicit thinking budget or effort in the JSON body; thinking is enabled implicitly for compatible models via the beta header.

```rust
// anthropic.rs (stream path, setup-token case)
headers.insert(
    "anthropic-beta".to_string(),
    "claude-code-20250219,interleaved-thinking-2025-05-14".to_string(),
);
// No "thinking" or "output_config" in the request body
```

- Non-setup-token (API key) requests do not set these beta headers; thinking is still parsed from the response/stream when the API returns thinking blocks.

**rust-genai** (`rust-genai/src/adapter/adapters/anthropic/adapter_impl.rs`):

- Sets thinking **budget** and **effort** explicitly in the request:
  - `insert_anthropic_thinking_budget_value()` maps `ReasoningEffort` to a `thinking.budget_tokens` value (e.g. Low→1024, Medium→8000, High→24000).
  - For opus-4-5, also sets `output_config.effort` (low/medium/high).
  - Model name can carry a reasoning suffix (e.g. `-low`, `-medium`) to infer effort.

```rust
// adapter_impl.rs
fn insert_anthropic_thinking_budget_value(payload: &mut Value, effort: &ReasoningEffort) -> Result<()> {
    let thinking_budget = match effort {
        ReasoningEffort::None => None,
        ReasoningEffort::Budget(budget) => Some(*budget),
        ReasoningEffort::Low | ReasoningEffort::Minimal => Some(REASONING_LOW),
        ReasoningEffort::Medium => Some(REASONING_MEDIUM),
        ReasoningEffort::High => Some(REASONING_HIGH),
    };
    if let Some(thinking_budget) = thinking_budget {
        payload.x_insert("thinking", json!({
            "type": "enabled",
            "budget_tokens": thinking_budget
        }))?;
    }
    Ok(())
}
```

**Summary:** rust-genai drives Anthropic thinking via explicit `thinking.budget_tokens` and (for opus-4-5) `output_config.effort`. zeroai enables interleaved thinking only for setup-token flows via a beta header and does not set thinking budget or effort in the body.

---

### 1.3 Anthropic: streaming (thinking blocks)

**zeroai** (`zeroai/src/providers/anthropic.rs`):

- Parses SSE `content_block_delta` for `delta.thinking` and `delta.signature`, and emits `StreamEvent::ThinkingDelta(th)` and accumulates signature. On completion, builds a single `ContentBlock::Thinking(ThinkingContent { thinking: thinking_buf, signature: signature_buf })` and includes it in `StreamEvent::Done`.

```rust
// content_block_delta handling
if let Some(th) = d.thinking { thinking_buf.push_str(&th); yield Ok(StreamEvent::ThinkingDelta(th)); }
if let Some(sig) = d.signature {
    if signature_buf.is_none() { signature_buf = Some(String::new()); }
    signature_buf.as_mut().unwrap().push_str(&sig);
}
// ...
if !thinking_buf.is_empty() { content.push(ContentBlock::Thinking(ThinkingContent { thinking: thinking_buf, signature: signature_buf })); }
```

**rust-genai** (`rust-genai/src/adapter/adapters/anthropic/streamer.rs`):

- Uses an `InProgressBlock` state (`Text`, `ToolUse`, `Thinking`). On `content_block_start` with type `"thinking"` sets `InProgressBlock::Thinking`. On `content_block_delta` for that block:
  - If `delta.thinking` is present: optionally appends to `captured_data.reasoning_content` and yields `InterStreamEvent::ReasoningChunk(thinking)`.
  - If `delta.signature` is present: yields `InterStreamEvent::ThoughtSignatureChunk(signature)`.

```rust
// streamer.rs
InProgressBlock::Thinking => {
    if let Ok(thinking) = data.x_take::<String>("/delta/thinking") {
        if self.options.capture_reasoning_content {
            // ... append to captured_data.reasoning_content
        }
        return Poll::Ready(Some(Ok(InterStreamEvent::ReasoningChunk(thinking))));
    } else if let Ok(signature) = data.x_take::<String>("/delta/signature") {
        return Poll::Ready(Some(Ok(InterStreamEvent::ThoughtSignatureChunk(signature))));
    }
    // ...
}
```

**Summary:** Both map Anthropic thinking/signature deltas to stream events. zeroai uses a single `ThinkingDelta` and a single `Thinking` block at the end; rust-genai uses `ReasoningChunk` / `ThoughtSignatureChunk` and optional capture into `reasoning_content`.

---

### 1.4 Non-streaming Anthropic response (thinking in content)

**zeroai** (`zeroai/src/providers/anthropic.rs`):

- When parsing the final `content` array, blocks with `type: "thinking"` are turned into `ContentBlock::Thinking(ThinkingContent { ... })`.

```rust
// anthropic.rs (non-streaming response parsing)
"thinking" => {
    if let Some(thinking) = block.thinking {
        content.push(ContentBlock::Thinking(ThinkingContent {
            thinking,
            signature: None,
        }));
    }
}
```

**rust-genai** (`rust-genai/src/adapter/adapters/anthropic/adapter_impl.rs`):

- Collects all `"thinking"` items into a `reasoning_content: Vec<String>`, then joins them and sets `ChatResponse.reasoning_content = Some(joined)`. Text and tool_use remain in the main content; thinking is not mixed in as content parts.

```rust
// adapter_impl.rs to_chat_response
let mut reasoning_content: Vec<String> = Vec::new();
for mut item in json_content_items {
    let typ: String = item.x_take("type")?;
    match typ.as_ref() {
        "text" => { /* content.push(ContentPart::from_text(...)) */ }
        "thinking" => reasoning_content.push(item.x_take("thinking")?),
        "tool_use" => { /* content.push(ContentPart::ToolCall(...)) */ }
        // ...
    }
}
let reasoning_content = if !reasoning_content.is_empty() {
    Some(reasoning_content.join("\n"))
} else { None };
// ChatResponse { ..., reasoning_content, ... }
```

**Summary:** zeroai keeps thinking inside the same content list as a `Thinking` block. rust-genai keeps thinking in a separate `reasoning_content` field and does not add thinking as `ContentPart`s.

---

### 1.5 Google / Gemini (thinking)

**zeroai** (`zeroai/src/providers/google.rs`, `zeroai/src/providers/google_gemini_cli.rs`):

- For models with `reasoning: true`, if `RequestOptions.reasoning` is set, fills `thinking_config` with a token budget derived from `ThinkingLevel` (e.g. Minimal→1024, Low→2048, Medium→8192, High→16384). Streaming and non-streaming responses detect “thought” parts and merge them into `ContentBlock::Thinking(ThinkingContent { ... })`.

**rust-genai** (`rust-genai/src/adapter/adapters/gemini/`):

- Uses `reasoning_effort` and similar options; reasoning/reasoning_content is handled in adapter and streamer logic (capture_reasoning_content, etc.), consistent with the options described above.

---

## 2. Tool Call Support

### 2.1 Tool definitions (server-side)

**zeroai** (`zeroai/src/types.rs`):

- Single struct: `ToolDef` with `name`, `description`, and `parameters` (JSON Schema).

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
```

- `ChatContext` has `tools: Vec<ToolDef>`.

**rust-genai** (`rust-genai/src/chat/tool/tool_base.rs`, `tool_types.rs`):

- `Tool` has:
  - `name: ToolName` — either `ToolName::Custom(String)` or `ToolName::WebSearch` (built-in).
  - `description: Option<String>`
  - `schema: Option<Value>` (JSON Schema)
  - `config: Option<ToolConfig>` — e.g. `WebSearch(WebSearchConfig)` or `Custom(Value)` for provider-specific config.
- `ToolName` / `ToolConfig` use custom (de)serialization so custom tools are “bare” and built-ins like WebSearch are qualified (e.g. `{"WebSearch": null}`).

**Summary:** zeroai has a simple, flat tool definition. rust-genai adds a normalized tool name (including built-in WebSearch), optional schema, and optional provider-specific config.

---

### 2.2 Tool call (model output)

**zeroai** (`zeroai/src/types.rs`):

- `ToolCall` has `id`, `name`, `arguments` (JSON value). Represented in content as `ContentBlock::ToolCall(ToolCall)`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}
```

**rust-genai** (`rust-genai/src/chat/tool/tool_call.rs`):

- `ToolCall` has `call_id`, `fn_name`, `fn_arguments`, and optionally `thought_signatures: Option<Vec<String>>` for placing thought signatures before tool calls in the assistant turn (e.g. for continuation).

```rust
pub struct ToolCall {
    pub call_id: String,
    pub fn_name: String,
    pub fn_arguments: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signatures: Option<Vec<String>>,
}
```

**Summary:** Field names differ (id/call_id, name/fn_name, arguments/fn_arguments). rust-genai additionally supports `thought_signatures` for ordering thinking relative to tool calls.

---

### 2.3 Tool result (user/executor → model)

**zeroai** (`zeroai/src/types.rs`):

- `ToolResultMessage` is a message role with `tool_call_id`, `tool_name`, `content: Vec<ContentBlock>`, and `is_error`. It appears as `Message::ToolResult(ToolResultMessage)`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
}
```

**rust-genai** (`rust-genai/src/chat/tool/tool_response.rs`):

- `ToolResponse` is a simple pair: `call_id` and `content: String` (no structured content list, no explicit is_error in the type).

```rust
pub struct ToolResponse {
    pub call_id: String,
    pub content: String,
}
```

**Summary:** zeroai models tool results as full messages with multi-block content and an error flag. rust-genai uses a minimal call_id + string content; error semantics would be convention or encoded in content.

---

### 2.4 Anthropic: tool use in request/response

**zeroai** (`zeroai/src/providers/anthropic.rs`):

- Converts `context.tools` to Anthropic `tools` array with `name`, `description`, `input_schema` (from `parameters`). When using a setup-token (Claude Code), tool names are mapped to PascalCase official names (e.g. Read, Write, Bash) via `to_claude_code_name` / `from_claude_code_name`.
- Stream: `content_block_start` (type `tool_use`) → `ToolCallStart`; `content_block_delta` (partial_json) → `ToolCallDelta`; `content_block_stop` → `ToolCallEnd` with full `ToolCall`. Non-streaming response parses `content` items of type `tool_use` into `ContentBlock::ToolCall(ToolCall)`.

**rust-genai** (`rust-genai/src/adapter/adapters/anthropic/adapter_impl.rs`, `streamer.rs`):

- Builds Anthropic request with a `tools` array from the generic `Tool` list (name, description, input_schema). No Claude Code–specific name mapping.
- Stream: same idea — `content_block_start` for `tool_use` initializes `InProgressBlock::ToolUse { id, name, input }`; deltas append to `input`; `content_block_stop` produces a `ToolCall` and yields it. Tool calls can be captured in `StreamerCapturedData` when `capture_tool_calls` is set.

**Summary:** Both support Anthropic tool_use in stream and non-stream. zeroai adds Claude Code tool name mapping when using a setup-token; rust-genai adds optional capture of tool calls and thought signatures for tool-call ordering.

---

### 2.5 Stream events for tools

**zeroai** (`zeroai/src/types.rs`):

- Dedicated stream events for tool calls: `ToolCallStart { index, id, name }`, `ToolCallDelta { index, delta }`, `ToolCallEnd { index, tool_call }`.

**rust-genai**:

- Uses a generic `InterStreamEvent` (e.g. chunk, reasoning, thought signature, tool call). Tool calls are emitted when a full tool-use block is finished (e.g. in `content_block_stop`), and can be collected in `captured_tool_calls` when capture is enabled.

---

## 3. Summary Table

| Aspect | zeroai | rust-genai |
|--------|--------|------------|
| **Thinking in types** | `ThinkingLevel` (Minimal/Low/Medium/High), `ContentBlock::Thinking(ThinkingContent)` | `ReasoningEffort` (+ Budget, Minimal), `Verbosity`, `ContentPart::ThoughtSignature`, `reasoning_content` on response |
| **Anthropic thinking request** | Beta header for setup-token only; no body budget/effort | Explicit `thinking.budget_tokens` and (opus-4-5) `output_config.effort` |
| **Anthropic thinking stream** | `ThinkingDelta(String)`, single `Thinking` block in Done | `ReasoningChunk`, `ThoughtSignatureChunk`, optional capture |
| **Anthropic thinking non-stream** | Thinking as `ContentBlock::Thinking` in content | Thinking in `ChatResponse.reasoning_content` only |
| **Tool definition** | `ToolDef { name, description, parameters }` | `Tool { name: ToolName, description, schema, config }` with WebSearch/Custom |
| **Tool call** | `ToolCall { id, name, arguments }` | `ToolCall { call_id, fn_name, fn_arguments, thought_signatures }` |
| **Tool result** | `ToolResultMessage { tool_call_id, tool_name, content[], is_error }` | `ToolResponse { call_id, content }` |
| **Anthropic tools** | Claude Code name mapping (setup-token) | Generic tools; optional capture and thought ordering |

---

## 4. References (file paths)

**zeroai (this repo):**

- Types: `zeroai/src/types.rs`
- Anthropic provider: `zeroai/src/providers/anthropic.rs`
- Google/Gemini reasoning: `zeroai/src/providers/google.rs`, `zeroai/src/providers/google_gemini_cli.rs`

**rust-genai:**

- Chat options / reasoning: `rust-genai/src/chat/chat_options.rs`
- Content parts: `rust-genai/src/chat/content_part/common.rs`
- Tool types: `rust-genai/src/chat/tool/tool_types.rs`, `tool_base.rs`, `tool_call.rs`, `tool_response.rs`
- Anthropic adapter: `rust-genai/src/adapter/adapters/anthropic/adapter_impl.rs`
- Anthropic streamer: `rust-genai/src/adapter/adapters/anthropic/streamer.rs`
