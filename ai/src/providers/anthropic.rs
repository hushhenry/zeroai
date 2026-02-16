use super::{Provider, ProviderError};
use crate::types::*;
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

/// Anthropic Messages API provider.
pub struct AnthropicProvider {
    client: Client,
}

impl AnthropicProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Response types (SSE stream)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StreamEventData {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    content_block: Option<ContentBlockData>,
    #[serde(default)]
    delta: Option<DeltaData>,
    #[serde(default)]
    message: Option<MessageData>,
    #[serde(default)]
    usage: Option<UsageData>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ContentBlockData {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct DeltaData {
    #[serde(rename = "type", default)]
    delta_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct MessageData {
    #[serde(default)]
    usage: Option<UsageData>,
}

#[derive(Deserialize)]
struct UsageData {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn convert_messages(context: &ChatContext) -> Vec<AnthropicMessage> {
    let mut msgs = Vec::new();

    for msg in &context.messages {
        match msg {
            Message::User(u) => {
                let content = user_content_to_value(&u.content);
                msgs.push(AnthropicMessage {
                    role: "user".into(),
                    content,
                });
            }
            Message::Assistant(a) => {
                let mut blocks = Vec::new();
                for block in &a.content {
                    match block {
                        ContentBlock::Text(t) => {
                            blocks.push(json!({"type": "text", "text": t.text}));
                        }
                        ContentBlock::Thinking(th) => {
                            blocks.push(json!({"type": "thinking", "thinking": th.thinking}));
                        }
                        ContentBlock::ToolCall(tc) => {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": tc.arguments
                            }));
                        }
                        _ => {}
                    }
                }
                msgs.push(AnthropicMessage {
                    role: "assistant".into(),
                    content: json!(blocks),
                });
            }
            Message::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::Text(t) = b {
                            Some(t.text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                msgs.push(AnthropicMessage {
                    role: "user".into(),
                    content: json!([{
                        "type": "tool_result",
                        "tool_use_id": tr.tool_call_id,
                        "content": text,
                        "is_error": tr.is_error
                    }]),
                });
            }
        }
    }

    msgs
}

fn user_content_to_value(blocks: &[ContentBlock]) -> serde_json::Value {
    if blocks.len() == 1 {
        if let ContentBlock::Text(t) = &blocks[0] {
            return json!(t.text);
        }
    }

    let parts: Vec<serde_json::Value> = blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(json!({"type": "text", "text": t.text})),
            ContentBlock::Image(img) => Some(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": img.mime_type,
                    "data": img.data
                }
            })),
            _ => None,
        })
        .collect();

    json!(parts)
}

fn convert_tools(tools: &[ToolDef]) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|t| AnthropicTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.parameters.clone(),
        })
        .collect()
}


// ---------------------------------------------------------------------------
// Provider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for AnthropicProvider {
    fn stream(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &StreamOptions,
    ) -> BoxStream<'static, Result<StreamEvent, ProviderError>> {
        let api_key = match &options.api_key {
            Some(k) => k.clone(),
            None => {
                return Box::pin(stream::once(async {
                    Err(ProviderError::AuthRequired(
                        "API key required for Anthropic".into(),
                    ))
                }));
            }
        };

        let base_url = model.base_url.trim_end_matches('/').to_string();
        let url = format!("{}/messages", base_url);

        let messages = convert_messages(context);
        let tools = if context.tools.is_empty() {
            None
        } else {
            Some(convert_tools(&context.tools))
        };

        let body = MessagesRequest {
            model: model.id.clone(),
            messages,
            max_tokens: options.max_tokens.unwrap_or(model.max_tokens),
            system: context.system_prompt.clone(),
            temperature: options.temperature,
            stream: true,
            tools,
        };

        let mut headers_map = HashMap::new();
        if let Some(model_headers) = &model.headers {
            headers_map.extend(model_headers.clone());
        }
        if let Some(extra) = &options.extra_headers {
            headers_map.extend(extra.clone());
        }

        let client = self.client.clone();
        let model_id = model.id.clone();
        let provider_id = model.provider.clone();

        let s = async_stream::stream! {
            let mut req = client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json");

            for (k, v) in &headers_map {
                req = req.header(k.as_str(), v.as_str());
            }

            let resp = match req.json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    yield Err(ProviderError::Network(e));
                    return;
                }
            };

            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                yield Err(ProviderError::Http {
                    status: status.as_u16(),
                    body: body_text,
                });
                return;
            }

            yield Ok(StreamEvent::Start);

            let mut text_buf = String::new();
            let mut thinking_buf = String::new();
            let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
            let mut usage = Usage::default();
            let mut stop_reason = StopReason::Stop;

            let mut event_type_buf = String::new();
            let mut line_buf = String::new();

            let mut byte_stream = resp.bytes_stream();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        yield Err(ProviderError::Network(e));
                        return;
                    }
                };

                let chunk_str = String::from_utf8_lossy(&chunk_bytes);
                line_buf.push_str(&chunk_str);

                while let Some(newline_pos) = line_buf.find('\n') {
                    let line: String = line_buf.drain(..=newline_pos).collect();
                    let line = line.trim();

                    if line.is_empty() {
                        continue;
                    }

                    if let Some(evt) = line.strip_prefix("event: ") {
                        event_type_buf = evt.trim().to_string();
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        let evt: StreamEventData = match serde_json::from_str(data) {
                            Ok(e) => e,
                            Err(_) => continue,
                        };

                        match evt.event_type.as_str() {
                            "message_start" => {
                                if let Some(msg) = &evt.message {
                                    if let Some(u) = &msg.usage {
                                        usage.input_tokens = u.input_tokens;
                                        usage.cache_read_tokens = u.cache_read_input_tokens.unwrap_or(0);
                                        usage.cache_write_tokens = u.cache_creation_input_tokens.unwrap_or(0);
                                    }
                                }
                            }
                            "content_block_start" => {
                                if let Some(block) = &evt.content_block {
                                    match block.block_type.as_str() {
                                        "tool_use" => {
                                            let id = block.id.clone().unwrap_or_default();
                                            let name = block.name.clone().unwrap_or_default();
                                            let idx = tool_calls.len();
                                            tool_calls.push((id.clone(), name.clone(), String::new()));
                                            yield Ok(StreamEvent::ToolCallStart {
                                                index: idx,
                                                id,
                                                name,
                                            });
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Some(delta) = &evt.delta {
                                    match delta.delta_type.as_deref() {
                                        Some("text_delta") => {
                                            if let Some(text) = &delta.text {
                                                text_buf.push_str(text);
                                                yield Ok(StreamEvent::TextDelta(text.clone()));
                                            }
                                        }
                                        Some("thinking_delta") => {
                                            if let Some(thinking) = &delta.thinking {
                                                thinking_buf.push_str(thinking);
                                                yield Ok(StreamEvent::ThinkingDelta(thinking.clone()));
                                            }
                                        }
                                        Some("input_json_delta") => {
                                            if let Some(partial_json) = &delta.partial_json {
                                                if let Some(last) = tool_calls.last_mut() {
                                                    last.2.push_str(partial_json);
                                                    let idx = tool_calls.len() - 1;
                                                    yield Ok(StreamEvent::ToolCallDelta {
                                                        index: idx,
                                                        delta: partial_json.clone(),
                                                    });
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "content_block_stop" => {
                                // Emit tool call end if the last block was a tool call
                                if let Some(idx) = evt.index {
                                    if idx < tool_calls.len() {
                                        let (id, name, args_str) = &tool_calls[idx];
                                        let arguments: serde_json::Value =
                                            serde_json::from_str(args_str).unwrap_or(json!({}));
                                        yield Ok(StreamEvent::ToolCallEnd {
                                            index: idx,
                                            tool_call: ToolCall {
                                                id: id.clone(),
                                                name: name.clone(),
                                                arguments,
                                            },
                                        });
                                    }
                                }
                            }
                            "message_delta" => {
                                if let Some(delta) = &evt.delta {
                                    if let Some(reason) = &delta.stop_reason {
                                        stop_reason = match reason.as_str() {
                                            "end_turn" => StopReason::Stop,
                                            "max_tokens" => StopReason::Length,
                                            "tool_use" => StopReason::ToolUse,
                                            _ => StopReason::Stop,
                                        };
                                    }
                                }
                                if let Some(u) = &evt.usage {
                                    usage.output_tokens = u.output_tokens;
                                }
                            }
                            _ => {}
                        }

                        event_type_buf.clear();
                    }
                }
            }

            let mut content = Vec::new();
            if !thinking_buf.is_empty() {
                content.push(ContentBlock::Thinking(ThinkingContent {
                    thinking: thinking_buf,
                }));
            }
            if !text_buf.is_empty() {
                content.push(ContentBlock::Text(TextContent { text: text_buf }));
            }
            for (id, name, args_str) in tool_calls {
                let arguments: serde_json::Value =
                    serde_json::from_str(&args_str).unwrap_or(json!({}));
                content.push(ContentBlock::ToolCall(ToolCall {
                    id,
                    name,
                    arguments,
                }));
            }

            usage.total_tokens = usage.input_tokens + usage.output_tokens +
                usage.cache_read_tokens + usage.cache_write_tokens;

            let msg = AssistantMessage {
                content,
                model: model_id,
                provider: provider_id,
                usage: Some(usage),
                stop_reason,
            };

            yield Ok(StreamEvent::Done { message: msg });
        };

        Box::pin(s)
    }

    async fn list_models(&self, _api_key: &str) -> Result<Vec<ModelDef>, ProviderError> {
        Ok(static_anthropic_models())
    }
}

/// Static model list for Anthropic (they don't have a /models endpoint).
pub fn static_anthropic_models() -> Vec<ModelDef> {
    let base_url = "https://api.anthropic.com/v1";
    let provider = "anthropic";

    vec![
        ModelDef {
            id: "claude-opus-4-0-20250514".into(),
            name: "Claude Opus 4".into(),
            api: Api::AnthropicMessages,
            provider: provider.into(),
            base_url: base_url.into(),
            reasoning: true,
            input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 15.0, output: 75.0, cache_read: 1.5, cache_write: 18.75 },
            context_window: 200000,
            max_tokens: 32768,
            headers: None,
        },
        ModelDef {
            id: "claude-4-6-opus-20260610".into(),
            name: "Claude Opus 4.6".into(),
            api: Api::AnthropicMessages,
            provider: provider.into(),
            base_url: base_url.into(),
            reasoning: true,
            input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 15.0, output: 75.0, cache_read: 1.5, cache_write: 18.75 },
            context_window: 200000,
            max_tokens: 32768,
            headers: None,
        },
        ModelDef {
            id: "claude-sonnet-4-0-20250514".into(),
            name: "Claude Sonnet 4".into(),
            api: Api::AnthropicMessages,
            provider: provider.into(),
            base_url: base_url.into(),
            reasoning: true,
            input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 3.0, output: 15.0, cache_read: 0.3, cache_write: 3.75 },
            context_window: 200000,
            max_tokens: 16384,
            headers: None,
        },
        ModelDef {
            id: "claude-sonnet-4-5-20250514".into(),
            name: "Claude Sonnet 4.5".into(),
            api: Api::AnthropicMessages,
            provider: provider.into(),
            base_url: base_url.into(),
            reasoning: true,
            input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 3.0, output: 15.0, cache_read: 0.3, cache_write: 3.75 },
            context_window: 200000,
            max_tokens: 16384,
            headers: None,
        },
        ModelDef {
            id: "claude-3-7-sonnet-20250219".into(),
            name: "Claude Sonnet 3.7".into(),
            api: Api::AnthropicMessages,
            provider: provider.into(),
            base_url: base_url.into(),
            reasoning: true,
            input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 3.0, output: 15.0, cache_read: 0.3, cache_write: 3.75 },
            context_window: 200000,
            max_tokens: 16384,
            headers: None,
        },
        ModelDef {
            id: "claude-3-5-sonnet-20241022".into(),
            name: "Claude Sonnet 3.5 v2".into(),
            api: Api::AnthropicMessages,
            provider: provider.into(),
            base_url: base_url.into(),
            reasoning: false,
            input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 3.0, output: 15.0, cache_read: 0.3, cache_write: 3.75 },
            context_window: 200000,
            max_tokens: 8192,
            headers: None,
        },
        ModelDef {
            id: "claude-3-5-haiku-20241022".into(),
            name: "Claude Haiku 3.5".into(),
            api: Api::AnthropicMessages,
            provider: provider.into(),
            base_url: base_url.into(),
            reasoning: false,
            input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 0.8, output: 4.0, cache_read: 0.08, cache_write: 1.0 },
            context_window: 200000,
            max_tokens: 8192,
            headers: None,
        },
        ModelDef {
            id: "claude-3-haiku-20240307".into(),
            name: "Claude Haiku 3".into(),
            api: Api::AnthropicMessages,
            provider: provider.into(),
            base_url: base_url.into(),
            reasoning: false,
            input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 0.25, output: 1.25, cache_read: 0.03, cache_write: 0.3 },
            context_window: 200000,
            max_tokens: 4096,
            headers: None,
        },
    ]
}
