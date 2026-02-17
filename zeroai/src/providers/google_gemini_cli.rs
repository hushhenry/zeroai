use super::sanitize;
use super::{Provider, ProviderError};
use crate::types::*;
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

/// Google Gemini CLI / Antigravity provider.
/// Uses the Cloud Code Assist API endpoint.
/// Shared implementation for both `gemini-cli` and `antigravity` providers.
pub struct GoogleGeminiCliProvider {
    client: Client,
    /// Whether this instance operates in Antigravity mode.
    is_antigravity: bool,
}

impl GoogleGeminiCliProvider {
    pub fn new_gemini_cli() -> Self {
        Self {
            client: Client::new(),
            is_antigravity: false,
        }
    }

    pub fn new_antigravity() -> Self {
        Self {
            client: Client::new(),
            is_antigravity: true,
        }
    }
}

const DEFAULT_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";
const ANTIGRAVITY_DAILY_ENDPOINT: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";
const DEFAULT_ANTIGRAVITY_VERSION: &str = "1.15.8";

fn gemini_cli_headers() -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("User-Agent".into(), "google-cloud-sdk vscode_cloudshelleditor/0.1".into());
    h.insert("X-Goog-Api-Client".into(), "gl-node/22.17.0".into());
    h.insert(
        "Client-Metadata".into(),
        serde_json::json!({
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI"
        })
        .to_string(),
    );
    h
}

fn antigravity_headers() -> HashMap<String, String> {
    let version = std::env::var("PI_AI_ANTIGRAVITY_VERSION")
        .unwrap_or_else(|_| DEFAULT_ANTIGRAVITY_VERSION.to_string());
    let mut h = HashMap::new();
    h.insert("User-Agent".into(), format!("antigravity/{} linux/x86_64", version));
    h.insert("X-Goog-Api-Client".into(), "google-cloud-sdk vscode_cloudshelleditor/0.1".into());
    h.insert(
        "Client-Metadata".into(),
        serde_json::json!({
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI"
        })
        .to_string(),
    );
    h
}

// ---------------------------------------------------------------------------
// Request types (Cloud Code Assist format)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CloudCodeAssistRequest {
    project: String,
    model: String,
    request: InnerRequest,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InnerRequest {
    contents: Vec<GContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GSystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GToolDeclaration>>,
}

#[derive(Serialize)]
struct GContent {
    role: String,
    parts: Vec<GPart>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GFunctionResponse>,
}

#[derive(Serialize)]
struct GFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize)]
struct GFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Serialize)]
struct GSystemInstruction {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GPart>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_config: Option<GThinkingConfig>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GThinkingConfig {
    include_thoughts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_level: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GToolDeclaration {
    function_declarations: Vec<GFunctionDeclaration>,
}

#[derive(Serialize)]
struct GFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ChunkEnvelope {
    response: Option<ResponseData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseData {
    candidates: Option<Vec<RCandidate>>,
    usage_metadata: Option<RUsageMetadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RCandidate {
    content: Option<RContent>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct RContent {
    parts: Option<Vec<RPart>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RPart {
    text: Option<String>,
    thought: Option<bool>,
    function_call: Option<RFunctionCall>,
}

#[derive(Deserialize)]
struct RFunctionCall {
    name: String,
    args: Option<serde_json::Value>,
    id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RUsageMetadata {
    prompt_token_count: Option<u64>,
    candidates_token_count: Option<u64>,
    thoughts_token_count: Option<u64>,
    total_token_count: Option<u64>,
    cached_content_token_count: Option<u64>,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn convert_messages(context: &ChatContext) -> Vec<GContent> {
    let mut contents = Vec::new();

    for msg in &context.messages {
        match msg {
            Message::User(u) => {
                let parts: Vec<GPart> = u
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(GPart {
                            text: Some(t.text.clone()),
                            function_call: None,
                            function_response: None,
                        }),
                        _ => None,
                    })
                    .collect();

                contents.push(GContent {
                    role: "user".into(),
                    parts,
                });
            }
            Message::Assistant(a) => {
                let mut parts = Vec::new();
                for block in &a.content {
                    match block {
                        ContentBlock::Text(t) => {
                            parts.push(GPart {
                                text: Some(t.text.clone()),
                                function_call: None,
                                function_response: None,
                            });
                        }
                        ContentBlock::ToolCall(tc) => {
                            parts.push(GPart {
                                text: None,
                                function_call: Some(GFunctionCall {
                                    name: tc.name.clone(),
                                    args: tc.arguments.clone(),
                                }),
                                function_response: None,
                            });
                        }
                        _ => {}
                    }
                }
                contents.push(GContent {
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

                contents.push(GContent {
                    role: "user".into(),
                    parts: vec![GPart {
                        text: None,
                        function_call: None,
                        function_response: Some(GFunctionResponse {
                            name: tr.tool_name.clone(),
                            response: json!({"result": text}),
                        }),
                    }],
                });
            }
        }
    }

    contents
}

fn convert_tools(tools: &[ToolDef]) -> Vec<GToolDeclaration> {
    vec![GToolDeclaration {
        function_declarations: tools
            .iter()
            .map(|t| GFunctionDeclaration {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect(),
    }]
}

/// Parse the JSON-encoded API key used by Cloud Code Assist.
/// Format: `{"token": "...", "projectId": "..."}`
fn parse_cloud_code_api_key(api_key: &str) -> Result<(String, String), ProviderError> {
    #[derive(Deserialize)]
    struct CloudKey {
        token: String,
        #[serde(rename = "projectId")]
        project_id: String,
    }

    let parsed: CloudKey = serde_json::from_str(api_key).map_err(|_| {
        ProviderError::AuthRequired(
            "Invalid Cloud Code Assist credentials. Expected JSON {token, projectId}.".into(),
        )
    })?;

    if parsed.token.is_empty() || parsed.project_id.is_empty() {
        return Err(ProviderError::AuthRequired(
            "Missing token or projectId in Cloud Code credentials.".into(),
        ));
    }

    Ok((parsed.token, parsed.project_id))
}

static TOOL_CALL_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Provider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for GoogleGeminiCliProvider {
    fn stream(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> BoxStream<'static, Result<StreamEvent, ProviderError>> {
        let api_key_raw = match &options.api_key {
            Some(k) => k.clone(),
            None => {
                return Box::pin(stream::once(async {
                    Err(ProviderError::AuthRequired(
                        "OAuth credentials required for Cloud Code Assist".into(),
                    ))
                }));
            }
        };

        let (access_token, project_id) = match parse_cloud_code_api_key(&api_key_raw) {
            Ok(v) => v,
            Err(e) => {
                return Box::pin(stream::once(async move { Err(e) }));
            }
        };

        let is_antigravity = self.is_antigravity;
        let base_url = if !model.base_url.is_empty() {
            model.base_url.trim_end_matches('/').to_string()
        } else if is_antigravity {
            ANTIGRAVITY_DAILY_ENDPOINT.to_string()
        } else {
            DEFAULT_ENDPOINT.to_string()
        };

        let url = format!("{}/v1internal:streamGenerateContent?alt=sse", base_url);

        let contents = convert_messages(context);

        let mut sys_parts = Vec::new();
        if is_antigravity {
            sys_parts.push(GPart {
                text: Some(
                    "You are Antigravity, a powerful agentic AI coding assistant designed by the Google Deepmind team."
                        .into(),
                ),
                function_call: None,
                function_response: None,
            });
        }
        if let Some(sp) = &context.system_prompt {
            sys_parts.push(GPart {
                text: Some(sp.clone()),
                function_call: None,
                function_response: None,
            });
        }

        let system_instruction = if sys_parts.is_empty() {
            None
        } else {
            Some(GSystemInstruction {
                role: if is_antigravity { Some("user".into()) } else { None },
                parts: sys_parts,
            })
        };

        let mut gen_config = GGenerationConfig {
            temperature: options.temperature,
            max_output_tokens: options.max_tokens,
            thinking_config: None,
        };

        if model.reasoning {
            if let Some(level) = &options.reasoning {
                let is_gemini3 = model.id.contains("3-pro") || model.id.contains("3-flash");
                if is_gemini3 {
                    let level_str = match level {
                        ThinkingLevel::Minimal => "MINIMAL",
                        ThinkingLevel::Low => "LOW",
                        ThinkingLevel::Medium => "MEDIUM",
                        ThinkingLevel::High => "HIGH",
                    };
                    gen_config.thinking_config = Some(GThinkingConfig {
                        include_thoughts: true,
                        thinking_budget: None,
                        thinking_level: Some(level_str.to_string()),
                    });
                } else {
                    let budget = match level {
                        ThinkingLevel::Minimal => 1024,
                        ThinkingLevel::Low => 2048,
                        ThinkingLevel::Medium => 8192,
                        ThinkingLevel::High => 16384,
                    };
                    gen_config.thinking_config = Some(GThinkingConfig {
                        include_thoughts: true,
                        thinking_budget: Some(budget),
                        thinking_level: None,
                    });
                }
            }
        }

        let tools = if context.tools.is_empty() {
            None
        } else {
            Some(convert_tools(&context.tools))
        };

        let request_body = CloudCodeAssistRequest {
            project: project_id,
            model: model.id.clone(),
            request: InnerRequest {
                contents,
                session_id: None,
                system_instruction,
                generation_config: Some(gen_config),
                tools,
            },
            request_type: if is_antigravity {
                Some("agent".into())
            } else {
                None
            },
            user_agent: Some(if is_antigravity {
                "antigravity"
            } else {
                "pi-coding-agent"
            }
            .into()),
            request_id: Some(format!(
                "{}-{}-{}",
                if is_antigravity { "agent" } else { "pi" },
                chrono::Utc::now().timestamp_millis(),
                uuid::Uuid::new_v4().to_string().get(..9).unwrap_or("000000000")
            )),
        };

        let extra_headers = if is_antigravity {
            antigravity_headers()
        } else {
            gemini_cli_headers()
        };

        let client = self.client.clone();
        let model_id = model.id.clone();
        let provider_id = model.provider.clone();
        let opt_extra_headers = options.extra_headers.clone();

        let s = async_stream::stream! {
            let mut req = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", access_token))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream");

            for (k, v) in &extra_headers {
                req = req.header(k.as_str(), v.as_str());
            }
            if let Some(mh) = &opt_extra_headers {
                for (k, v) in mh {
                    req = req.header(k.as_str(), v.as_str());
                }
            }

            let resp = match req.json(&request_body).send().await {
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

                    if line.is_empty() || !line.starts_with("data:") {
                        continue;
                    }

                    let data = line[5..].trim();
                    if data.is_empty() {
                        continue;
                    }

                    let chunk: ChunkEnvelope = match serde_json::from_str(data) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    let resp_data = match &chunk.response {
                        Some(r) => r,
                        None => continue,
                    };

                    if let Some(um) = &resp_data.usage_metadata {
                        let prompt = um.prompt_token_count.unwrap_or(0);
                        let cached = um.cached_content_token_count.unwrap_or(0);
                        usage.input_tokens = prompt.saturating_sub(cached);
                        usage.cache_read_tokens = cached;
                        usage.output_tokens = um.candidates_token_count.unwrap_or(0)
                            + um.thoughts_token_count.unwrap_or(0);
                        usage.total_tokens = um.total_token_count.unwrap_or(0);
                    }

                    if let Some(candidates) = &resp_data.candidates {
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
                                            let tc_id = fc.id.clone().unwrap_or_else(|| {
                                                format!("{}_{}", fc.name, counter)
                                            });
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
        let mut stream = self.stream(model, context, options);
        let mut full_msg = AssistantMessage {
            content: Vec::new(),
            model: model.id.clone(),
            provider: model.provider.clone(),
            usage: None,
            stop_reason: StopReason::Stop,
        };

        let mut text_buf = String::new();
        let mut thinking_buf = String::new();
        let mut tool_calls = Vec::new();

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(d) => text_buf.push_str(&d),
                StreamEvent::ThinkingDelta(d) => thinking_buf.push_str(&d),
                StreamEvent::ToolCallEnd { tool_call, .. } => tool_calls.push(tool_call),
                StreamEvent::Done { message } => {
                    full_msg.usage = message.usage;
                    full_msg.stop_reason = message.stop_reason;
                }
                _ => {}
            }
        }

        if !thinking_buf.is_empty() {
            full_msg.content.push(ContentBlock::Thinking(ThinkingContent {
                thinking: thinking_buf,
                signature: None,
            }));
        }
        if !text_buf.is_empty() {
            full_msg.content.push(ContentBlock::Text(TextContent {
                text: text_buf,
            }));
        }
        for tc in tool_calls {
            full_msg.content.push(ContentBlock::ToolCall(tc));
        }

        Ok(full_msg)
    }

    async fn list_models(&self, _api_key: &str) -> Result<Vec<ModelDef>, ProviderError> {
        if self.is_antigravity {
            Ok(static_antigravity_models())
        } else {
            Ok(static_gemini_cli_models())
        }
    }
}

/// Static model list for Gemini CLI provider.
pub fn static_gemini_cli_models() -> Vec<ModelDef> {
    let provider = "gemini-cli";
    let base_url = DEFAULT_ENDPOINT;
    let api = Api::GoogleGeminiCli;

    vec![
        model_def(provider, base_url, &api, "gemini-2.5-pro", "Gemini 2.5 Pro", true, 1048576, 65536),
        model_def(provider, base_url, &api, "gemini-2.5-flash", "Gemini 2.5 Flash", true, 1048576, 65536),
        model_def(provider, base_url, &api, "gemini-2.0-flash", "Gemini 2.0 Flash", false, 1048576, 8192),
        model_def(provider, base_url, &api, "gemini-3-pro-preview", "Gemini 3 Pro Preview", true, 1048576, 65536),
        model_def(provider, base_url, &api, "gemini-3-flash-preview", "Gemini 3 Flash Preview", true, 1048576, 65536),
    ]
}

/// Static model list for Antigravity provider.
pub fn static_antigravity_models() -> Vec<ModelDef> {
    let provider = "antigravity";
    let base_url = ANTIGRAVITY_DAILY_ENDPOINT;
    let api = Api::GoogleGeminiCli;

    vec![
        model_def(provider, base_url, &api, "gemini-2.5-pro", "Gemini 2.5 Pro", true, 1048576, 65536),
        model_def(provider, base_url, &api, "gemini-2.5-flash", "Gemini 2.5 Flash", true, 1048576, 65536),
        model_def(provider, base_url, &api, "gemini-2.0-flash", "Gemini 2.0 Flash", false, 1048576, 8192),
        model_def(provider, base_url, &api, "gemini-3-pro-preview", "Gemini 3 Pro Preview", true, 1048576, 65536),
        model_def(provider, base_url, &api, "gemini-3-flash-preview", "Gemini 3 Flash Preview", true, 1048576, 65536),
        model_def(provider, base_url, &api, "claude-sonnet-4-5-20250514", "Claude Sonnet 4.5", true, 200000, 16384),
        model_def(provider, base_url, &api, "claude-sonnet-4-0-20250514", "Claude Sonnet 4", true, 200000, 16384),
        model_def(provider, base_url, &api, "claude-3-5-sonnet-20241022", "Claude 3.5 Sonnet v2", false, 200000, 8192),
    ]
}

fn model_def(
    provider: &str,
    base_url: &str,
    api: &Api,
    id: &str,
    name: &str,
    reasoning: bool,
    context_window: u64,
    max_tokens: u64,
) -> ModelDef {
    ModelDef {
        id: id.into(),
        name: name.into(),
        api: api.clone(),
        provider: provider.into(),
        base_url: base_url.into(),
        reasoning,
        input: vec![InputModality::Text, InputModality::Image],
        cost: ModelCost::default(),
        context_window,
        max_tokens,
        headers: None,
    }
}
