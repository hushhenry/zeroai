use super::{Provider, ProviderError};
use crate::types::*;
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

pub struct AnthropicProvider {
    client: Client,
}

impl AnthropicProvider {
    pub fn new() -> Self {
        Self { client: Client::new() }
    }
}

impl Default for AnthropicProvider {
    fn default() -> Self { Self::new() }
}

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<serde_json::Value>,
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
    #[serde(rename = "input_schema")]
    parameters: serde_json::Value,
}

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
struct ContentBlockData {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
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
            None => return Box::pin(stream::once(async { Err(ProviderError::AuthRequired("API key required".into())) })),
        };

        // Identity Injection for setup-token (Claude Code mimic)
        // setup-tokens usually start with sk-ant-sid
        let is_setup_token = api_key.contains("sk-ant-sid");
        let mut headers = HashMap::new();
        headers.insert("x-api-key".to_string(), api_key.clone());
        headers.insert("anthropic-version".to_string(), "2023-06-01".to_string());
        
        let mut system_blocks = Vec::new();
        if is_setup_token {
            headers.insert("anthropic-beta".to_string(), "claude-code-20250219,interleaved-thinking-2025-05-14".to_string());
            system_blocks.push(json!({"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude."}));
        }
        if let Some(sys) = &context.system_prompt {
            system_blocks.push(json!({"type": "text", "text": sys}));
        }

        let system = if system_blocks.is_empty() { None } else { Some(json!(system_blocks)) };
        
        let req_body = MessagesRequest {
            model: model.id.clone(),
            messages: convert_messages(context),
            max_tokens: options.max_tokens.unwrap_or(model.max_tokens),
            system,
            temperature: options.temperature,
            stream: true,
            tools: if context.tools.is_empty() { None } else { Some(convert_tools(&context.tools)) },
        };

        let client = self.client.clone();
        let url = format!("{}/messages", model.base_url.trim_end_matches('/'));
        let model_id = model.id.clone();
        let provider_id = model.provider.clone();

        let s = async_stream::stream! {
            let mut req = client.post(&url);
            for (k, v) in &headers { req = req.header(k, v); }
            let resp = match req.json(&req_body).send().await {
                Ok(r) => r,
                Err(e) => { yield Err(ProviderError::Network(e)); return; }
            };
            let status = resp.status();
            if !status.is_success() {
                yield Err(ProviderError::Http { status: status.as_u16(), body: resp.text().await.unwrap_or_default() });
                return;
            }
            yield Ok(StreamEvent::Start);
            
            let mut text_buf = String::new();
            let mut thinking_buf = String::new();
            let mut tool_calls: Vec<(String, String, String)> = Vec::new();
            let mut usage = Usage::default();
            let mut stop_reason = StopReason::Stop;
            let mut line_buf = String::new();
            let mut byte_stream = resp.bytes_stream();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result { Ok(b) => b, Err(e) => { yield Err(ProviderError::Network(e)); return; } };
                line_buf.push_str(&String::from_utf8_lossy(&chunk_bytes));
                while let Some(newline_pos) = line_buf.find('\n') {
                    let line: String = line_buf.drain(..=newline_pos).collect();
                    let line = line.trim();
                    if line.is_empty() || !line.starts_with("data: ") { continue; }
                    let data = &line[6..];
                    let evt: StreamEventData = match serde_json::from_str(data) { Ok(e) => e, Err(_) => continue };
                    
                    match evt.event_type.as_str() {
                        "message_start" => { if let Some(m) = evt.message { if let Some(u) = m.usage { usage.input_tokens = u.input_tokens; } } }
                        "content_block_start" => {
                            if let Some(b) = evt.content_block {
                                if b.block_type == "tool_use" {
                                    let id = b.id.unwrap_or_default();
                                    let name = b.name.unwrap_or_default();
                                    tool_calls.push((id.clone(), name.clone(), String::new()));
                                    yield Ok(StreamEvent::ToolCallStart { index: tool_calls.len()-1, id, name });
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(d) = evt.delta {
                                if let Some(t) = d.text { text_buf.push_str(&t); yield Ok(StreamEvent::TextDelta(t)); }
                                if let Some(th) = d.thinking { thinking_buf.push_str(&th); yield Ok(StreamEvent::ThinkingDelta(th)); }
                                if let Some(pj) = d.partial_json {
                                    if let Some(last) = tool_calls.last_mut() {
                                        last.2.push_str(&pj);
                                        yield Ok(StreamEvent::ToolCallDelta { index: tool_calls.len()-1, delta: pj });
                                    }
                                }
                            }
                        }
                        "content_block_stop" => {
                            if let Some(idx) = evt.index {
                                if idx < tool_calls.len() {
                                    let (id, name, args) = &tool_calls[idx];
                                    yield Ok(StreamEvent::ToolCallEnd { index: idx, tool_call: ToolCall { id: id.clone(), name: name.clone(), arguments: serde_json::from_str(args).unwrap_or(json!({})) } });
                                }
                            }
                        }
                        "message_delta" => {
                            if let Some(d) = evt.delta { if let Some(sr) = d.stop_reason { stop_reason = match sr.as_str() { "end_turn" => StopReason::Stop, "tool_use" => StopReason::ToolUse, _ => StopReason::Stop }; } }
                            if let Some(u) = evt.usage { usage.output_tokens = u.output_tokens; }
                        }
                        _ => {}
                    }
                }
            }
            
            let mut content = Vec::new();
            if !thinking_buf.is_empty() { content.push(ContentBlock::Thinking(ThinkingContent { thinking: thinking_buf })); }
            if !text_buf.is_empty() { content.push(ContentBlock::Text(TextContent { text: text_buf })); }
            for (id, name, args) in tool_calls { content.push(ContentBlock::ToolCall(ToolCall { id, name, arguments: serde_json::from_str(&args).unwrap_or(json!({})) })); }
            
            usage.total_tokens = usage.input_tokens + usage.output_tokens;
            yield Ok(StreamEvent::Done { message: AssistantMessage { content, model: model_id, provider: provider_id, usage: Some(usage), stop_reason } });
        };
        Box::pin(s)
    }

    async fn list_models(&self, _api_key: &str) -> Result<Vec<ModelDef>, ProviderError> {
        Ok(static_anthropic_models())
    }
}

fn convert_messages(context: &ChatContext) -> Vec<AnthropicMessage> {
    context.messages.iter().map(|m| match m {
        Message::User(u) => AnthropicMessage { role: "user".into(), content: json!(u.content.iter().filter_map(|b| match b {
            ContentBlock::Text(t) => Some(json!({"type": "text", "text": t.text})),
            _ => None
        }).collect::<Vec<_>>()) },
        Message::Assistant(a) => AnthropicMessage { role: "assistant".into(), content: json!(a.content.iter().map(|b| match b {
            ContentBlock::Text(t) => json!({"type": "text", "text": t.text}),
            ContentBlock::ToolCall(tc) => json!({"type": "tool_use", "id": tc.id, "name": tc.name, "input": tc.arguments}),
            _ => json!({})
        }).collect::<Vec<_>>()) },
        Message::ToolResult(tr) => AnthropicMessage { role: "user".into(), content: json!([{"type": "tool_result", "tool_use_id": tr.tool_call_id, "content": "result", "is_error": tr.is_error}]) },
    }).collect()
}

fn convert_tools(tools: &[ToolDef]) -> Vec<AnthropicTool> {
    tools.iter().map(|t| AnthropicTool { name: t.name.clone(), description: t.description.clone(), parameters: t.parameters.clone() }).collect()
}

pub fn static_anthropic_models() -> Vec<ModelDef> {
    let p = "anthropic";
    let url = "https://api.anthropic.com/v1";
    vec![
        ModelDef { id: "claude-3-5-sonnet-20241022".into(), name: "Claude 3.5 Sonnet".into(), api: Api::AnthropicMessages, provider: p.into(), base_url: url.into(), reasoning: false, input: vec![InputModality::Text], cost: ModelCost::default(), context_window: 200000, max_tokens: 8192, headers: None },
    ]
}
