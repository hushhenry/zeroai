use super::sanitize;
use super::{Provider, ProviderError};
use crate::types::*;
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Google Generative AI (Gemini API key) provider.
pub struct GoogleProvider {
    client: Client,
}

impl GoogleProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for GoogleProvider {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateContentRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<SystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDeclaration>>,
}

#[derive(Serialize)]
struct Content {
    role: String,
    parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<FunctionCallPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<FunctionResponsePart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inline_data: Option<InlineData>,
}

#[derive(Serialize)]
struct FunctionCallPart {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize)]
struct FunctionResponsePart {
    name: String,
    response: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: String,
    data: String,
}

#[derive(Serialize)]
struct SystemInstruction {
    parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_config: Option<ThinkingConfig>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThinkingConfig {
    include_thoughts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolDeclaration {
    function_declarations: Vec<FunctionDeclaration>,
}

#[derive(Serialize)]
struct FunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamChunk {
    candidates: Option<Vec<Candidate>>,
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    content: Option<CandidateContent>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct CandidateContent {
    parts: Option<Vec<ResponsePart>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponsePart {
    text: Option<String>,
    thought: Option<bool>,
    function_call: Option<FunctionCallResponse>,
}

#[derive(Deserialize)]
struct FunctionCallResponse {
    name: String,
    args: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    prompt_token_count: Option<u64>,
    candidates_token_count: Option<u64>,
    thoughts_token_count: Option<u64>,
    total_token_count: Option<u64>,
    cached_content_token_count: Option<u64>,
}

// ---------------------------------------------------------------------------
// Models list response
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ModelsListResponse {
    models: Option<Vec<ModelInfo>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelInfo {
    name: String,
    display_name: Option<String>,
    supported_generation_methods: Option<Vec<String>>,
    input_token_limit: Option<u64>,
    output_token_limit: Option<u64>,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn convert_messages(context: &ChatContext) -> Vec<Content> {
    let mut contents = Vec::new();

    for msg in &context.messages {
        match msg {
            Message::User(u) => {
                let parts = u
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(Part {
                            text: Some(t.text.clone()),
                            function_call: None,
                            function_response: None,
                            inline_data: None,
                        }),
                        ContentBlock::Image(img) => Some(Part {
                            text: None,
                            function_call: None,
                            function_response: None,
                            inline_data: Some(InlineData {
                                mime_type: img.mime_type.clone(),
                                data: img.data.clone(),
                            }),
                        }),
                        _ => None,
                    })
                    .collect();

                contents.push(Content {
                    role: "user".into(),
                    parts,
                });
            }
            Message::Assistant(a) => {
                let mut parts = Vec::new();
                for block in &a.content {
                    match block {
                        ContentBlock::Text(t) => {
                            parts.push(Part {
                                text: Some(t.text.clone()),
                                function_call: None,
                                function_response: None,
                                inline_data: None,
                            });
                        }
                        ContentBlock::ToolCall(tc) => {
                            parts.push(Part {
                                text: None,
                                function_call: Some(FunctionCallPart {
                                    name: tc.name.clone(),
                                    args: tc.arguments.clone(),
                                }),
                                function_response: None,
                                inline_data: None,
                            });
                        }
                        _ => {}
                    }
                }
                contents.push(Content {
                    role: "model".into(),
                    parts,
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

                contents.push(Content {
                    role: "user".into(),
                    parts: vec![Part {
                        text: None,
                        function_call: None,
                        function_response: Some(FunctionResponsePart {
                            name: tr.tool_name.clone(),
                            response: json!({"result": text}),
                        }),
                        inline_data: None,
                    }],
                });
            }
        }
    }

    contents
}

fn convert_tools(tools: &[ToolDef]) -> Vec<ToolDeclaration> {
    vec![ToolDeclaration {
        function_declarations: tools
            .iter()
            .map(|t| FunctionDeclaration {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect(),
    }]
}

// ---------------------------------------------------------------------------
// Provider impl
// ---------------------------------------------------------------------------

static TOOL_CALL_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateContentResponse {
    candidates: Vec<Candidate>,
    usage_metadata: Option<UsageMetadata>,
}

#[async_trait]
impl Provider for GoogleProvider {
    fn stream(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> BoxStream<'static, Result<StreamEvent, ProviderError>> {
        let api_key = match &options.api_key {
            Some(k) => k.clone(),
            None => {
                return Box::pin(stream::once(async {
                    Err(ProviderError::AuthRequired(
                        "API key required for Google".into(),
                    ))
                }));
            }
        };

        let base_url = model.base_url.trim_end_matches('/').to_string();
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            base_url, model.id, api_key
        );

        let contents = convert_messages(context);

        let system_instruction = context.system_prompt.as_ref().map(|sp| SystemInstruction {
            parts: vec![Part {
                text: Some(sp.clone()),
                function_call: None,
                function_response: None,
                inline_data: None,
            }],
        });

        let mut gen_config = GenerationConfig {
            temperature: options.temperature,
            max_output_tokens: options.max_tokens,
            thinking_config: None,
        };

        if model.reasoning {
            if let Some(level) = &options.reasoning {
                let budget = match level {
                    ThinkingLevel::Minimal => 1024,
                    ThinkingLevel::Low => 2048,
                    ThinkingLevel::Medium => 8192,
                    ThinkingLevel::High => 16384,
                };
                gen_config.thinking_config = Some(ThinkingConfig {
                    include_thoughts: true,
                    thinking_budget: Some(budget),
                });
            }
        }

        let tools = if context.tools.is_empty() {
            None
        } else {
            Some(convert_tools(&context.tools))
        };

        let body = GenerateContentRequest {
            contents,
            system_instruction,
            generation_config: Some(gen_config),
            tools,
        };

        let client = self.client.clone();
        let model_id = model.id.clone();
        let provider_id = model.provider.clone();

        let s = async_stream::stream! {
            let resp = match client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
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
                    body: sanitize::sanitize_api_error(&body_text),
                });
                return;
            }

            yield Ok(StreamEvent::Start);

            let mut text_buf = String::new();
            let mut thinking_buf = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut usage = Usage::default();
            let mut stop_reason = StopReason::Stop;
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

                    if !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..];
                    let chunk: StreamChunk = match serde_json::from_str(data) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    if let Some(um) = &chunk.usage_metadata {
                        let prompt = um.prompt_token_count.unwrap_or(0);
                        let cached = um.cached_content_token_count.unwrap_or(0);
                        usage.input_tokens = prompt.saturating_sub(cached);
                        usage.cache_read_tokens = cached;
                        usage.output_tokens = um.candidates_token_count.unwrap_or(0)
                            + um.thoughts_token_count.unwrap_or(0);
                        usage.total_tokens = um.total_token_count.unwrap_or(0);
                    }

                    if let Some(candidates) = &chunk.candidates {
                        for candidate in candidates {
                            if let Some(reason) = &candidate.finish_reason {
                                stop_reason = match reason.as_str() {
                                    "STOP" => StopReason::Stop,
                                    "MAX_TOKENS" => StopReason::Length,
                                    _ => StopReason::Stop,
                                };
                            }

                            if let Some(content) = &candidate.content {
                                if let Some(parts) = &content.parts {
                                    for part in parts {
                                        if let Some(text) = &part.text {
                                            let is_thinking = part.thought.unwrap_or(false);
                                            if is_thinking {
                                                thinking_buf.push_str(text);
                                                yield Ok(StreamEvent::ThinkingDelta(text.clone()));
                                            } else {
                                                text_buf.push_str(text);
                                                yield Ok(StreamEvent::TextDelta(text.clone()));
                                            }
                                        }

                                        if let Some(fc) = &part.function_call {
                                            let counter = TOOL_CALL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            let tc_id = format!("{}_{}", fc.name, counter);
                                            let args = fc.args.clone().unwrap_or(json!({}));
                                            let idx = tool_calls.len();

                                            let tc = ToolCall {
                                                id: tc_id.clone(),
                                                name: fc.name.clone(),
                                                arguments: args.clone(),
                                            };
                                            tool_calls.push(tc.clone());

                                            yield Ok(StreamEvent::ToolCallStart {
                                                index: idx,
                                                id: tc_id,
                                                name: fc.name.clone(),
                                            });
                                            yield Ok(StreamEvent::ToolCallDelta {
                                                index: idx,
                                                delta: args.to_string(),
                                            });
                                            yield Ok(StreamEvent::ToolCallEnd {
                                                index: idx,
                                                tool_call: tc,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !tool_calls.is_empty() {
                stop_reason = StopReason::ToolUse;
            }

            let mut content = Vec::new();
            if !thinking_buf.is_empty() {
                content.push(ContentBlock::Thinking(ThinkingContent { thinking: thinking_buf, signature: None }));
            }
            if !text_buf.is_empty() {
                content.push(ContentBlock::Text(TextContent { text: text_buf }));
            }
            for tc in tool_calls {
                content.push(ContentBlock::ToolCall(tc));
            }

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

    async fn chat(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> Result<AssistantMessage, ProviderError> {
        let api_key = match &options.api_key {
            Some(k) => k.clone(),
            None => {
                return Err(ProviderError::AuthRequired(
                    "API key required for Google".into(),
                ));
            }
        };

        let base_url = model.base_url.trim_end_matches('/').to_string();
        let url = format!("{}/models/{}:generateContent?key={}", base_url, model.id, api_key);

        let contents = convert_messages(context);

        let system_instruction = context.system_prompt.as_ref().map(|sp| SystemInstruction {
            parts: vec![Part {
                text: Some(sp.clone()),
                function_call: None,
                function_response: None,
                inline_data: None,
            }],
        });

        let mut gen_config = GenerationConfig {
            temperature: options.temperature,
            max_output_tokens: options.max_tokens,
            thinking_config: None,
        };

        if model.reasoning {
            if let Some(level) = &options.reasoning {
                let budget = match level {
                    ThinkingLevel::Minimal => 1024,
                    ThinkingLevel::Low => 2048,
                    ThinkingLevel::Medium => 8192,
                    ThinkingLevel::High => 16384,
                };
                gen_config.thinking_config = Some(ThinkingConfig {
                    include_thoughts: true,
                    thinking_budget: Some(budget),
                });
            }
        }

        let tools = if context.tools.is_empty() {
            None
        } else {
            Some(convert_tools(&context.tools))
        };

        let body = GenerateContentRequest {
            contents,
            system_instruction,
            generation_config: Some(gen_config),
            tools,
        };

        let resp = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: sanitize::sanitize_api_error(&body),
            });
        }

        let gen_resp: GenerateContentResponse = resp.json().await?;

        let mut text_buf = String::new();
        let mut thinking_buf = String::new();
        let mut tool_calls = Vec::new();
        let mut stop_reason = StopReason::Stop;
        let mut usage = Usage::default();

        if let Some(um) = gen_resp.usage_metadata {
            let prompt = um.prompt_token_count.unwrap_or(0);
            let cached = um.cached_content_token_count.unwrap_or(0);
            usage.input_tokens = prompt.saturating_sub(cached);
            usage.cache_read_tokens = cached;
            usage.output_tokens = um.candidates_token_count.unwrap_or(0) + um.thoughts_token_count.unwrap_or(0);
            usage.total_tokens = um.total_token_count.unwrap_or(0);
        }

        if let Some(candidate) = gen_resp.candidates.first() {
            if let Some(reason) = &candidate.finish_reason {
                stop_reason = match reason.as_str() {
                    "STOP" => StopReason::Stop,
                    "MAX_TOKENS" => StopReason::Length,
                    _ => StopReason::Stop,
                };
            }

            if let Some(content) = &candidate.content {
                if let Some(parts) = &content.parts {
                    for part in parts {
                        if let Some(text) = &part.text {
                            if part.thought.unwrap_or(false) {
                                thinking_buf.push_str(text);
                            } else {
                                text_buf.push_str(text);
                            }
                        }
                        if let Some(fc) = &part.function_call {
                            let counter = TOOL_CALL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            tool_calls.push(ToolCall {
                                id: format!("{}_{}", fc.name, counter),
                                name: fc.name.clone(),
                                arguments: fc.args.clone().unwrap_or(json!({})),
                            });
                        }
                    }
                }
            }
        }

        if !tool_calls.is_empty() {
            stop_reason = StopReason::ToolUse;
        }

        let mut content = Vec::new();
        if !thinking_buf.is_empty() {
            content.push(ContentBlock::Thinking(ThinkingContent { thinking: thinking_buf, signature: None }));
        }
        if !text_buf.is_empty() {
            content.push(ContentBlock::Text(TextContent { text: text_buf }));
        }
        for tc in tool_calls {
            content.push(ContentBlock::ToolCall(tc));
        }

        Ok(AssistantMessage {
            content,
            model: model.id.clone(),
            provider: model.provider.clone(),
            usage: Some(usage),
            stop_reason,
        })
    }

    async fn list_models(&self, api_key: &str) -> Result<Vec<ModelDef>, ProviderError> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models?key={}",
            api_key
        );

        let resp = self.client.get(&url).send().await?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status,
                body: sanitize::sanitize_api_error(&body),
            });
        }

        let list: ModelsListResponse = resp.json().await?;

        let models = list
            .models
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                m.supported_generation_methods
                    .as_ref()
                    .is_some_and(|methods| {
                        methods.contains(&"generateContent".to_string())
                            || methods.contains(&"streamGenerateContent".to_string())
                    })
            })
            .map(|m| {
                let id = m.name.strip_prefix("models/").unwrap_or(&m.name).to_string();
                let name = m.display_name.unwrap_or_else(|| id.clone());
                let reasoning = id.contains("thinking") || id.contains("2.5");

                ModelDef {
                    id,
                    name,
                    api: Api::GoogleGenerativeAi,
                    provider: "google".into(),
                    base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
                    reasoning,
                    input: vec![InputModality::Text, InputModality::Image],
                    cost: ModelCost::default(),
                    context_window: m.input_token_limit.unwrap_or(128000),
                    max_tokens: m.output_token_limit.unwrap_or(8192),
                    headers: None,
                }
            })
            .collect();

        Ok(models)
    }
}
