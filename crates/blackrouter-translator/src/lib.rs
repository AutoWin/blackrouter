use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub mod stream;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireFormat {
    OpenAiChat,
    OpenAiResponses,
    ClaudeMessages,
    Gemini,
    GeminiCli,
    Kiro,
    Antigravity,
    CommandCode,
    Cursor,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TranslationError {
    #[error("unsupported translation: {from:?} -> {to:?}")]
    Unsupported { from: WireFormat, to: WireFormat },
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("invalid format: {0}")]
    InvalidFormat(String),
    #[error("translation failed: {0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, TranslationError>;

/// Check if translation is a passthrough (same format)
pub fn is_passthrough(source: WireFormat, target: WireFormat) -> bool {
    source == target
}

/// Translate a request body from source format to target format
pub fn translate_request(body: &Value, from: WireFormat, to: WireFormat) -> Result<Value> {
    if is_passthrough(from, to) {
        return Ok(body.clone());
    }

    match (from, to) {
        // OpenAI Chat -> Claude Messages
        (WireFormat::OpenAiChat, WireFormat::ClaudeMessages) => openai_to_claude(body),
        // OpenAI Chat -> Gemini
        (WireFormat::OpenAiChat, WireFormat::Gemini) => openai_to_gemini(body),
        // OpenAI Chat -> Gemini CLI
        (WireFormat::OpenAiChat, WireFormat::GeminiCli) => openai_to_gemini_cli(body),
        // Claude Messages -> OpenAI Chat
        (WireFormat::ClaudeMessages, WireFormat::OpenAiChat) => claude_to_openai(body),
        // Gemini -> OpenAI Chat
        (WireFormat::Gemini, WireFormat::OpenAiChat) => gemini_to_openai(body),
        // OpenAI Chat -> CommandCode
        (WireFormat::OpenAiChat, WireFormat::CommandCode) => openai_to_commandcode(body),
        // OpenAI Chat -> Cursor
        (WireFormat::OpenAiChat, WireFormat::Cursor) => openai_to_cursor(body),
        // OpenAI Chat -> Kiro
        (WireFormat::OpenAiChat, WireFormat::Kiro) => openai_to_kiro(body),
        // OpenAI Chat -> Antigravity
        (WireFormat::OpenAiChat, WireFormat::Antigravity) => openai_to_antigravity(body),
        _ => Err(TranslationError::Unsupported { from, to }),
    }
}

/// Translate a response body from source format back to target format
pub fn translate_response(body: &Value, from: WireFormat, to: WireFormat) -> Result<Value> {
    if is_passthrough(from, to) {
        return Ok(body.clone());
    }

    match (from, to) {
        // Claude -> OpenAI response
        (WireFormat::ClaudeMessages, WireFormat::OpenAiChat) => claude_response_to_openai(body),
        // Gemini -> OpenAI response
        (WireFormat::Gemini, WireFormat::OpenAiChat) => gemini_response_to_openai(body),
        // CommandCode -> OpenAI response
        (WireFormat::CommandCode, WireFormat::OpenAiChat) => commandcode_response_to_openai(body),
        // Cursor -> OpenAI response
        (WireFormat::Cursor, WireFormat::OpenAiChat) => cursor_response_to_openai(body),
        // Kiro -> OpenAI response
        (WireFormat::Kiro, WireFormat::OpenAiChat) => kiro_response_to_openai(body),
        // Antigravity -> OpenAI response
        (WireFormat::Antigravity, WireFormat::OpenAiChat) => antigravity_response_to_openai(body),
        // OpenAI Chat -> Claude response (for /v1/messages endpoint)
        (WireFormat::OpenAiChat, WireFormat::ClaudeMessages) => openai_response_to_claude(body),
        // Passthrough for same format or unsupported reverse
        _ => Ok(body.clone()),
    }
}

// ============================================================
// OpenAI Chat -> Other Formats
// ============================================================

fn openai_to_claude(body: &Value) -> Result<Value> {
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslationError::MissingField("messages".into()))?;

    let max_tokens = body
        .get("max_tokens")
        .or_else(|| body.get("max_completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(4096);

    let system = extract_system_message(messages);
    let claude_messages = convert_openai_messages_to_claude(messages)?;

    let mut result = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": claude_messages,
    });

    if let Some(system) = system {
        result
            .as_object_mut()
            .unwrap()
            .insert("system".to_string(), Value::String(system));
    }

    // Copy temperature, top_p, etc.
    if let Some(temp) = body.get("temperature") {
        result
            .as_object_mut()
            .unwrap()
            .insert("temperature".to_string(), temp.clone());
    }
    if let Some(top_p) = body.get("top_p") {
        result
            .as_object_mut()
            .unwrap()
            .insert("top_p".to_string(), top_p.clone());
    }
    if let Some(stop) = body.get("stop") {
        result
            .as_object_mut()
            .unwrap()
            .insert("stop_sequences".to_string(), stop.clone());
    }

    // Translate tools (function calling)
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let claude_tools: Vec<Value> = tools
            .iter()
            .filter_map(|tool| {
                let function = tool.get("function")?;
                Some(serde_json::json!({
                    "name": function.get("name").cloned().unwrap_or(Value::Null),
                    "description": function.get("description").cloned().unwrap_or(Value::Null),
                    "input_schema": function.get("parameters").cloned().unwrap_or(serde_json::json!({"type": "object"}))
                }))
            })
            .collect();
        if !claude_tools.is_empty() {
            result
                .as_object_mut()
                .unwrap()
                .insert("tools".to_string(), Value::Array(claude_tools));
        }
    }

    // Translate tool_choice
    if let Some(tool_choice) = body.get("tool_choice") {
        let claude_choice = match tool_choice {
            Value::String(s) => match s.as_str() {
                "auto" => serde_json::json!({"type": "auto"}),
                "none" => serde_json::json!({"type": "none"}),
                "required" => serde_json::json!({"type": "any"}),
                _ => serde_json::json!({"type": "auto"}),
            },
            Value::Object(obj) => {
                if let Some(name) = obj
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                {
                    serde_json::json!({"type": "tool", "name": name})
                } else {
                    serde_json::json!({"type": "auto"})
                }
            }
            _ => serde_json::json!({"type": "auto"}),
        };
        result
            .as_object_mut()
            .unwrap()
            .insert("tool_choice".to_string(), claude_choice);
    }

    // parallel_tool_calls → disable_parallel_tool_use (inverted)
    if let Some(parallel) = body.get("parallel_tool_calls").and_then(Value::as_bool) {
        result.as_object_mut().unwrap().insert(
            "disable_parallel_tool_use".to_string(),
            Value::Bool(!parallel),
        );
    }

    Ok(result)
}

fn openai_to_gemini(body: &Value) -> Result<Value> {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslationError::MissingField("messages".into()))?;

    let contents = convert_openai_messages_to_gemini(messages)?;

    let mut result = serde_json::json!({
        "contents": contents,
    });

    // Generation config
    let mut generation_config = serde_json::json!({});
    if let Some(temp) = body.get("temperature") {
        generation_config
            .as_object_mut()
            .unwrap()
            .insert("temperature".to_string(), temp.clone());
    }
    if let Some(top_p) = body.get("top_p") {
        generation_config
            .as_object_mut()
            .unwrap()
            .insert("topP".to_string(), top_p.clone());
    }
    if let Some(max_tokens) = body
        .get("max_tokens")
        .or_else(|| body.get("max_completion_tokens"))
    {
        generation_config
            .as_object_mut()
            .unwrap()
            .insert("maxOutputTokens".to_string(), max_tokens.clone());
    }
    if let Some(stop) = body.get("stop") {
        generation_config
            .as_object_mut()
            .unwrap()
            .insert("stopSequences".to_string(), stop.clone());
    }
    result
        .as_object_mut()
        .unwrap()
        .insert("generationConfig".to_string(), generation_config);

    // System instruction
    let system = extract_system_message(messages);
    if let Some(system) = system {
        result.as_object_mut().unwrap().insert(
            "systemInstruction".to_string(),
            serde_json::json!({
                "parts": [{"text": system}]
            }),
        );
    }

    // Translate tools (function calling) → Gemini function_declarations
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let func_decls: Vec<Value> = tools
            .iter()
            .filter_map(|tool| {
                let function = tool.get("function")?;
                Some(serde_json::json!({
                    "name": function.get("name").cloned().unwrap_or(Value::Null),
                    "description": function.get("description").cloned().unwrap_or(Value::Null),
                    "parameters": function.get("parameters").cloned().unwrap_or(serde_json::json!({"type": "object"}))
                }))
            })
            .collect();
        if !func_decls.is_empty() {
            result.as_object_mut().unwrap().insert(
                "tools".to_string(),
                serde_json::json!([{"function_declarations": func_decls}]),
            );
        }
    }

    // Translate tool_choice → Gemini tool_config
    if let Some(tool_choice) = body.get("tool_choice") {
        let mode = match tool_choice {
            Value::String(s) => match s.as_str() {
                "auto" => "AUTO",
                "none" => "NONE",
                "required" => "ANY",
                _ => "AUTO",
            },
            Value::Object(_) => "ANY",
            _ => "AUTO",
        };
        result.as_object_mut().unwrap().insert(
            "tool_config".to_string(),
            serde_json::json!({"function_calling_config": {"mode": mode}}),
        );
    }

    Ok(result)
}

fn openai_to_gemini_cli(body: &Value) -> Result<Value> {
    // Gemini CLI uses a simplified format
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslationError::MissingField("messages".into()))?;

    let mut prompt_parts = Vec::new();
    for msg in messages {
        if let Some(content) = msg.get("content").and_then(Value::as_str) {
            prompt_parts.push(serde_json::json!({"text": content}));
        }
    }

    Ok(serde_json::json!({
        "contents": [{
            "parts": prompt_parts
        }]
    }))
}

fn openai_to_commandcode(body: &Value) -> Result<Value> {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslationError::MissingField("messages".into()))?;

    let model = body.get("model").and_then(Value::as_str).unwrap_or("");

    // CommandCode uses a different message format with role/content
    let cc_messages: Vec<Value> = messages
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role").and_then(Value::as_str)?;
            let content = msg.get("content")?;
            Some(serde_json::json!({
                "role": role,
                "content": content
            }))
        })
        .collect();

    let mut result = serde_json::json!({
        "model": model,
        "messages": cc_messages,
    });

    if let Some(max_tokens) = body
        .get("max_tokens")
        .or_else(|| body.get("max_completion_tokens"))
    {
        result
            .as_object_mut()
            .unwrap()
            .insert("max_tokens".to_string(), max_tokens.clone());
    }

    Ok(result)
}

fn openai_to_cursor(body: &Value) -> Result<Value> {
    // Cursor uses OpenAI-compatible format with some extensions
    let mut result = body.clone();
    if let Some(obj) = result.as_object_mut() {
        // Cursor-specific fields
        obj.insert("stream".to_string(), Value::Bool(true));
    }
    Ok(result)
}

fn openai_to_kiro(body: &Value) -> Result<Value> {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslationError::MissingField("messages".into()))?;

    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("default");

    // Kiro uses assistantMessage format
    let conversation_state = serde_json::json!({
        "conversationState": {
            "currentMessage": {
                "content": extract_last_user_message(messages).unwrap_or_default(),
                "userInputMessage": {
                    "content": extract_last_user_message(messages).unwrap_or_default()
                }
            }
        }
    });

    Ok(serde_json::json!({
        "assistantId": model,
        "conversationState": conversation_state.get("conversationState"),
    }))
}

fn openai_to_antigravity(body: &Value) -> Result<Value> {
    let request = openai_to_gemini(body)?;
    Ok(serde_json::json!({
        "request": request,
        "requestType": "agent",
        "userAgent": "antigravity",
    }))
}

// ============================================================
// Other Formats -> OpenAI Chat (for responses)
// ============================================================

fn claude_to_openai(body: &Value) -> Result<Value> {
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslationError::MissingField("messages".into()))?;

    let system = body.get("system").and_then(Value::as_str);
    let max_tokens = body.get("max_tokens").and_then(Value::as_u64);

    let mut openai_messages = Vec::new();
    if let Some(system) = system {
        openai_messages.push(serde_json::json!({
            "role": "system",
            "content": system
        }));
    }

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
        let content = msg.get("content");
        openai_messages.push(serde_json::json!({
            "role": role,
            "content": content
        }));
    }

    let mut result = serde_json::json!({
        "model": model,
        "messages": openai_messages,
    });

    if let Some(max_tokens) = max_tokens {
        result
            .as_object_mut()
            .unwrap()
            .insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
    }

    if let Some(temp) = body.get("temperature") {
        result
            .as_object_mut()
            .unwrap()
            .insert("temperature".to_string(), temp.clone());
    }

    Ok(result)
}

fn gemini_to_openai(body: &Value) -> Result<Value> {
    let contents = body
        .get("contents")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslationError::MissingField("contents".into()))?;

    let system_instruction = body.get("systemInstruction");
    let generation_config = body.get("generationConfig");

    let mut openai_messages = Vec::new();

    // Add system instruction if present
    if let Some(system) = system_instruction
        .and_then(|v| v.get("parts"))
        .and_then(Value::as_array)
        .and_then(|parts| parts.first())
        .and_then(|p| p.get("text"))
        .and_then(Value::as_str)
    {
        openai_messages.push(serde_json::json!({
            "role": "system",
            "content": system
        }));
    }

    // Convert contents to messages
    for content in contents {
        let role = content
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let openai_role = if role == "model" { "assistant" } else { role };

        if let Some(parts) = content.get("parts").and_then(Value::as_array) {
            let text_parts: Vec<String> = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str).map(String::from))
                .collect();
            if !text_parts.is_empty() {
                openai_messages.push(serde_json::json!({
                    "role": openai_role,
                    "content": text_parts.join("\n")
                }));
            }
        }
    }

    let mut result = serde_json::json!({
        "messages": openai_messages,
    });

    // Map generation config
    if let Some(config) = generation_config {
        if let Some(temp) = config.get("temperature") {
            result
                .as_object_mut()
                .unwrap()
                .insert("temperature".to_string(), temp.clone());
        }
        if let Some(top_p) = config.get("topP") {
            result
                .as_object_mut()
                .unwrap()
                .insert("top_p".to_string(), top_p.clone());
        }
        if let Some(max_tokens) = config.get("maxOutputTokens") {
            result
                .as_object_mut()
                .unwrap()
                .insert("max_tokens".to_string(), max_tokens.clone());
        }
    }

    Ok(result)
}

// ============================================================
// Response Translations
// ============================================================

fn claude_response_to_openai(body: &Value) -> Result<Value> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("msg_claude");
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude");
    let content = body.get("content").and_then(Value::as_array);
    let stop_reason = body.get("stop_reason").and_then(Value::as_str);
    let usage = body.get("usage");

    // Extract text content and tool_use blocks
    let (text_content, tool_calls): (String, Vec<Value>) = content
        .map(|blocks| {
            let mut text_parts = Vec::new();
            let mut tools = Vec::new();
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            text_parts.push(text.to_string());
                        }
                    }
                    Some("tool_use") => {
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("tool_call");
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                        let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                        let arguments = serde_json::to_string(&input).unwrap_or_default();
                        tools.push(serde_json::json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": arguments
                            }
                        }));
                    }
                    _ => {}
                }
            }
            (text_parts.join(""), tools)
        })
        .unwrap_or_default();

    let has_tool_calls = !tool_calls.is_empty();

    let finish_reason = match stop_reason {
        Some("end_turn") | Some("stop_sequence") => "stop",
        Some("max_tokens") => "length",
        Some("tool_use") => "tool_calls",
        _ => {
            if has_tool_calls {
                "tool_calls"
            } else {
                "stop"
            }
        }
    };

    // Build message with optional tool_calls
    let message_content = if has_tool_calls && text_content.is_empty() {
        Value::Null
    } else {
        Value::String(text_content)
    };

    let mut message = serde_json::json!({
        "role": "assistant",
        "content": message_content
    });

    if has_tool_calls {
        message
            .as_object_mut()
            .unwrap()
            .insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    let mut result = serde_json::json!({
        "id": format!("chatcmpl-{}", id),
        "object": "chat.completion",
        "created": blackrouter_common::unix_timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason
        }]
    });

    if let Some(usage) = usage {
        let prompt_tokens = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion_tokens = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        result.as_object_mut().unwrap().insert(
            "usage".to_string(),
            serde_json::json!({
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens
            }),
        );
    }

    Ok(result)
}

fn gemini_response_to_openai(body: &Value) -> Result<Value> {
    let candidates = body.get("candidates").and_then(Value::as_array);

    // Extract text and functionCall parts
    let (text, tool_calls): (String, Vec<Value>) = candidates
        .and_then(|c| c.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| {
            let mut text_parts = Vec::new();
            let mut tools = Vec::new();
            for (i, p) in parts.iter().enumerate() {
                if let Some(text) = p.get("text").and_then(Value::as_str) {
                    text_parts.push(text.to_string());
                }
                if let Some(fc) = p.get("functionCall") {
                    let name = fc.get("name").and_then(Value::as_str).unwrap_or("");
                    let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
                    let arguments = serde_json::to_string(&args).unwrap_or_default();
                    tools.push(serde_json::json!({
                        "id": format!("call_gemini_{}", i),
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments
                        }
                    }));
                }
            }
            (text_parts.join(""), tools)
        })
        .unwrap_or_default();

    let has_tool_calls = !tool_calls.is_empty();

    let finish_reason = candidates
        .and_then(|c| c.first())
        .and_then(|c| c.get("finishReason"))
        .and_then(Value::as_str)
        .map(|reason| match reason {
            "STOP" => {
                if has_tool_calls {
                    "tool_calls"
                } else {
                    "stop"
                }
            }
            "MAX_TOKENS" => "length",
            _ => "stop",
        })
        .unwrap_or("stop");

    let usage_metadata = body.get("usageMetadata");

    // Build message with optional tool_calls
    let message_content = if has_tool_calls && text.is_empty() {
        Value::Null
    } else {
        Value::String(text)
    };

    let mut message = serde_json::json!({
        "role": "assistant",
        "content": message_content
    });

    if has_tool_calls {
        message
            .as_object_mut()
            .unwrap()
            .insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    let mut result = serde_json::json!({
        "id": format!("chatcmpl-gemini-{}", blackrouter_common::unix_timestamp()),
        "object": "chat.completion",
        "created": blackrouter_common::unix_timestamp(),
        "model": "gemini",
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason
        }]
    });

    if let Some(metadata) = usage_metadata {
        let prompt_tokens = metadata
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion_tokens = metadata
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        result.as_object_mut().unwrap().insert(
            "usage".to_string(),
            serde_json::json!({
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens
            }),
        );
    }

    Ok(result)
}

fn commandcode_response_to_openai(body: &Value) -> Result<Value> {
    // CommandCode returns OpenAI-compatible format with some extra fields
    let mut result = body.clone();
    if let Some(obj) = result.as_object_mut() {
        // Ensure required OpenAI fields exist
        if !obj.contains_key("object") {
            obj.insert(
                "object".to_string(),
                Value::String("chat.completion".to_string()),
            );
        }
        if !obj.contains_key("created") {
            obj.insert(
                "created".to_string(),
                Value::Number(blackrouter_common::unix_timestamp().into()),
            );
        }
    }
    Ok(result)
}

fn cursor_response_to_openai(body: &Value) -> Result<Value> {
    // Cursor returns SSE stream or OpenAI-compatible format
    // For non-streaming, it's already OpenAI-compatible
    let mut result = body.clone();
    if let Some(obj) = result.as_object_mut() {
        if !obj.contains_key("object") {
            obj.insert(
                "object".to_string(),
                Value::String("chat.completion".to_string()),
            );
        }
    }
    Ok(result)
}

fn kiro_response_to_openai(body: &Value) -> Result<Value> {
    // Kiro returns a different response structure
    let content = body
        .get("assistantResponse")
        .or_else(|| body.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");

    Ok(serde_json::json!({
        "id": format!("chatcmpl-kiro-{}", blackrouter_common::unix_timestamp()),
        "object": "chat.completion",
        "created": blackrouter_common::unix_timestamp(),
        "model": "kiro",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }]
    }))
}

fn antigravity_response_to_openai(body: &Value) -> Result<Value> {
    let response = body.get("response").unwrap_or(body);
    gemini_response_to_openai(response)
}

// ============================================================
// OpenAI Chat -> Claude Messages response (for /v1/messages endpoint)
// ============================================================

fn openai_response_to_claude(body: &Value) -> Result<Value> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("msg_openai");
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("openai");

    let choice = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first());

    let message = choice.and_then(|c| c.get("message"));
    let text = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool_calls = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array);
    let finish_reason = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(Value::as_str);

    let mut content_blocks = Vec::new();

    if !text.is_empty() {
        content_blocks.push(serde_json::json!({
            "type": "text",
            "text": text
        }));
    }

    // Convert OpenAI tool_calls → Claude tool_use blocks
    if let Some(tool_calls) = tool_calls {
        for tc in tool_calls {
            let id = tc.get("id").and_then(Value::as_str).unwrap_or("tool_call");
            let function = tc.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = function
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input: Value = serde_json::from_str(arguments).unwrap_or(serde_json::json!({}));
            content_blocks.push(serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }

    let stop_reason = match finish_reason {
        Some("stop") | None => "end_turn",
        Some("length") => "max_tokens",
        Some("tool_calls") => "tool_use",
        _ => "end_turn",
    };

    let mut result = serde_json::json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content_blocks,
        "stop_reason": stop_reason
    });

    // Convert usage
    if let Some(usage) = body.get("usage") {
        let prompt_tokens = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion_tokens = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        result.as_object_mut().unwrap().insert(
            "usage".to_string(),
            serde_json::json!({
                "input_tokens": prompt_tokens,
                "output_tokens": completion_tokens
            }),
        );
    }

    Ok(result)
}

// ============================================================
// OpenAI Responses API conversion (for /v1/responses endpoint)
// ============================================================

/// Convert OpenAI Responses API request → Chat Completions request
pub fn responses_request_to_chat(body: &Value) -> Result<Value> {
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");

    // `input` can be a string or an array of messages
    let input = body.get("input");
    let messages = match input {
        Some(Value::String(s)) => vec![serde_json::json!({
            "role": "user",
            "content": s
        })],
        Some(Value::Array(arr)) => {
            // Responses API input items → chat messages
            arr.iter()
                .filter_map(|item| {
                    let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                    let content = item.get("content").or_else(|| item.get("text"))?;
                    Some(serde_json::json!({
                        "role": role,
                        "content": content
                    }))
                })
                .collect()
        }
        _ => vec![],
    };

    let mut chat_body = serde_json::json!({
        "model": model,
        "messages": messages
    });

    // Copy common parameters
    if let Some(temp) = body.get("temperature") {
        chat_body
            .as_object_mut()
            .unwrap()
            .insert("temperature".to_string(), temp.clone());
    }
    if let Some(max_tokens) = body.get("max_output_tokens") {
        chat_body
            .as_object_mut()
            .unwrap()
            .insert("max_tokens".to_string(), max_tokens.clone());
    }
    if let Some(top_p) = body.get("top_p") {
        chat_body
            .as_object_mut()
            .unwrap()
            .insert("top_p".to_string(), top_p.clone());
    }
    if let Some(stream) = body.get("stream") {
        chat_body
            .as_object_mut()
            .unwrap()
            .insert("stream".to_string(), stream.clone());
    }

    // Copy tools if present (Responses API uses same tool format)
    if let Some(tools) = body.get("tools") {
        chat_body
            .as_object_mut()
            .unwrap()
            .insert("tools".to_string(), tools.clone());
    }
    if let Some(tool_choice) = body.get("tool_choice") {
        chat_body
            .as_object_mut()
            .unwrap()
            .insert("tool_choice".to_string(), tool_choice.clone());
    }

    Ok(chat_body)
}

/// Convert Chat Completions response → OpenAI Responses API response
pub fn chat_response_to_responses(body: &Value, model: &str) -> Result<Value> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("resp_openai");

    let choice = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first());

    let message = choice.and_then(|c| c.get("message"));
    let text = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool_calls = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array);
    let finish_reason = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(Value::as_str);

    let mut output = Vec::new();

    // Add message output item
    let mut output_item = serde_json::json!({
        "type": "message",
        "role": "assistant",
        "content": []
    });

    let mut content_parts = Vec::new();
    if !text.is_empty() {
        content_parts.push(serde_json::json!({
            "type": "output_text",
            "text": text
        }));
    }

    // Add tool calls as function_call output items
    if let Some(tool_calls) = tool_calls {
        for tc in tool_calls {
            let id = tc.get("id").and_then(Value::as_str).unwrap_or("call");
            let function = tc.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = function
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("{}");
            output.push(serde_json::json!({
                "type": "function_call",
                "id": id,
                "call_id": id,
                "name": name,
                "arguments": arguments
            }));
        }
    }

    output_item
        .as_object_mut()
        .unwrap()
        .insert("content".to_string(), Value::Array(content_parts));
    output.insert(0, output_item);

    let status = match finish_reason {
        Some("stop") | None => "completed",
        Some("length") => "incomplete",
        Some("tool_calls") => "completed",
        _ => "completed",
    };

    let mut result = serde_json::json!({
        "id": id,
        "object": "response",
        "created_at": blackrouter_common::unix_timestamp(),
        "model": model,
        "status": status,
        "output": output
    });

    // Convert usage
    if let Some(usage) = body.get("usage") {
        let prompt_tokens = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion_tokens = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        result.as_object_mut().unwrap().insert(
            "usage".to_string(),
            serde_json::json!({
                "input_tokens": prompt_tokens,
                "output_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens
            }),
        );
    }

    Ok(result)
}

// ============================================================
// Helper Functions
// ============================================================

fn extract_system_message(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .find(|msg| msg.get("role").and_then(Value::as_str) == Some("system"))
        .and_then(|msg| msg.get("content"))
        .and_then(Value::as_str)
        .map(String::from)
}

fn extract_last_user_message(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|msg| msg.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|msg| msg.get("content"))
        .and_then(Value::as_str)
        .map(String::from)
}

fn convert_openai_messages_to_claude(messages: &[Value]) -> Result<Vec<Value>> {
    let mut claude_messages = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");

        match role {
            "system" => continue, // Handled separately via system field
            "user" => {
                let content = msg.get("content");
                claude_messages.push(serde_json::json!({
                    "role": "user",
                    "content": content
                }));
            }
            "assistant" => {
                // Check for tool_calls in assistant message
                if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    let mut content_blocks = Vec::new();

                    // Add text content if present
                    if let Some(text) = msg.get("content").and_then(Value::as_str) {
                        if !text.is_empty() {
                            content_blocks.push(serde_json::json!({
                                "type": "text",
                                "text": text
                            }));
                        }
                    }

                    // Add tool_use blocks
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(Value::as_str).unwrap_or("tool_call");
                        let function = tc.get("function");
                        let name = function
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let arguments = function
                            .and_then(|f| f.get("arguments"))
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let input: Value =
                            serde_json::from_str(arguments).unwrap_or(serde_json::json!({}));
                        content_blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input
                        }));
                    }

                    claude_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content_blocks
                    }));
                } else {
                    let content = msg.get("content");
                    claude_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content
                    }));
                }
            }
            "tool" => {
                // Tool result → Claude user message with tool_result content block
                let tool_call_id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_call");
                let content = msg.get("content").unwrap_or(&Value::Null);
                claude_messages.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": content
                    }]
                }));
            }
            "function" => {
                // Legacy function role → user message with context
                if let Some(content) = msg.get("content").and_then(Value::as_str) {
                    claude_messages.push(serde_json::json!({
                        "role": "user",
                        "content": format!("[Function Result]: {}", content)
                    }));
                }
            }
            _ => {
                let content = msg.get("content");
                claude_messages.push(serde_json::json!({
                    "role": "user",
                    "content": content
                }));
            }
        }
    }

    // Merge consecutive same-role messages
    // Claude requires alternating roles, but tool_result (user) after tool_use (assistant) is fine
    let mut normalized: Vec<Value> = Vec::new();
    let mut last_role = String::new();

    for msg in claude_messages {
        let role = msg
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        if role == last_role {
            if let Some(last) = normalized.last_mut() {
                // Merge two string contents
                if let (Some(lc), Some(cc)) = (
                    last.get("content").and_then(Value::as_str),
                    msg.get("content").and_then(Value::as_str),
                ) {
                    *last = serde_json::json!({
                        "role": role,
                        "content": format!("{}\n\n{}", lc, cc)
                    });
                    continue;
                }
                // Merge two array contents
                if let (Some(lc_arr), Some(cc_arr)) = (
                    last.get("content").and_then(Value::as_array),
                    msg.get("content").and_then(Value::as_array),
                ) {
                    let mut merged = lc_arr.clone();
                    merged.extend(cc_arr.iter().cloned());
                    *last = serde_json::json!({
                        "role": role,
                        "content": merged
                    });
                    continue;
                }
            }
        }
        normalized.push(msg);
        last_role = role;
    }

    Ok(normalized)
}

fn convert_openai_messages_to_gemini(messages: &[Value]) -> Result<Vec<Value>> {
    let mut gemini_contents = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");

        match role {
            "system" => continue, // Handled separately via systemInstruction
            "assistant" => {
                let gemini_role = "model";

                // Check for tool_calls
                if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    let mut parts = Vec::new();

                    // Add text content if present
                    if let Some(text) = msg.get("content").and_then(Value::as_str) {
                        if !text.is_empty() {
                            parts.push(serde_json::json!({"text": text}));
                        }
                    }

                    // Add functionCall parts
                    for tc in tool_calls {
                        let function = tc.get("function");
                        let name = function
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let arguments = function
                            .and_then(|f| f.get("arguments"))
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let args: Value =
                            serde_json::from_str(arguments).unwrap_or(serde_json::json!({}));
                        parts.push(serde_json::json!({
                            "functionCall": {
                                "name": name,
                                "args": args
                            }
                        }));
                    }

                    gemini_contents.push(serde_json::json!({
                        "role": gemini_role,
                        "parts": parts
                    }));
                } else {
                    let text = msg.get("content").and_then(Value::as_str).unwrap_or("");
                    if !text.is_empty() {
                        gemini_contents.push(serde_json::json!({
                            "role": gemini_role,
                            "parts": [{"text": text}]
                        }));
                    }
                }
            }
            "tool" => {
                // Tool result → Gemini functionResponse in user message
                let tool_call_id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let content = msg.get("content");
                let response = if let Some(s) = content.and_then(Value::as_str) {
                    serde_json::from_str(s).unwrap_or(serde_json::json!({"result": s}))
                } else {
                    content.cloned().unwrap_or(serde_json::json!({}))
                };
                gemini_contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": tool_call_id,
                            "response": response
                        }
                    }]
                }));
            }
            "user" | _ => {
                let content = msg.get("content");
                if let Some(text) = content.and_then(Value::as_str) {
                    gemini_contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{"text": text}]
                    }));
                } else if let Some(content) = content {
                    gemini_contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{"text": content.to_string()}]
                    }));
                }
            }
        }
    }

    Ok(gemini_contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_passthrough() {
        let body = json!({"model": "gpt-4", "messages": []});
        let result = translate_request(&body, WireFormat::OpenAiChat, WireFormat::OpenAiChat);
        assert_eq!(result.unwrap(), body);
    }

    #[test]
    fn test_openai_to_claude() {
        let body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello"}
            ],
            "temperature": 0.7,
            "max_tokens": 1000
        });

        let result =
            translate_request(&body, WireFormat::OpenAiChat, WireFormat::ClaudeMessages).unwrap();

        assert_eq!(result.get("model").unwrap(), "gpt-4");
        assert_eq!(result.get("system").unwrap(), "You are helpful");
        assert_eq!(result.get("temperature").unwrap(), 0.7);
        assert_eq!(result.get("max_tokens").unwrap(), 1000);

        let messages = result.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].get("role").unwrap(), "user");
    }

    #[test]
    fn test_openai_to_gemini() {
        let body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {"role": "user", "content": "How are you?"}
            ]
        });

        let result = translate_request(&body, WireFormat::OpenAiChat, WireFormat::Gemini).unwrap();

        let contents = result.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 3); // system merged into first user message

        let system_instruction = result.get("systemInstruction").unwrap();
        assert!(system_instruction.get("parts").is_some());
    }

    #[test]
    fn test_claude_response_to_openai() {
        let body = json!({
            "id": "msg_123",
            "model": "claude-3-opus",
            "content": [
                {"type": "text", "text": "Hello world"}
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            }
        });

        let result =
            translate_response(&body, WireFormat::ClaudeMessages, WireFormat::OpenAiChat).unwrap();

        assert!(result
            .get("id")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("chatcmpl-"));
        assert_eq!(result.get("object").unwrap(), "chat.completion");

        let choices = result.get("choices").unwrap().as_array().unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(
            choices[0].get("message").unwrap().get("content").unwrap(),
            "Hello world"
        );

        let usage = result.get("usage").unwrap();
        assert_eq!(usage.get("prompt_tokens").unwrap(), 10);
        assert_eq!(usage.get("completion_tokens").unwrap(), 5);
    }

    #[test]
    fn test_gemini_response_to_openai() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello from Gemini"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 15,
                "candidatesTokenCount": 8
            }
        });

        let result = translate_response(&body, WireFormat::Gemini, WireFormat::OpenAiChat).unwrap();

        let choices = result.get("choices").unwrap().as_array().unwrap();
        assert_eq!(
            choices[0].get("message").unwrap().get("content").unwrap(),
            "Hello from Gemini"
        );
    }

    #[test]
    fn test_openai_to_antigravity_wraps_gemini_request() {
        let body = json!({
            "model": "black-gemini",
            "messages": [{"role": "user", "content": "Hello"}],
            "max_tokens": 16
        });

        let result =
            translate_request(&body, WireFormat::OpenAiChat, WireFormat::Antigravity).unwrap();

        assert!(result.get("request").is_some());
        assert!(result.get("request").unwrap().get("contents").is_some());
        assert!(result
            .get("request")
            .unwrap()
            .get("generationConfig")
            .is_some());
        assert!(result.get("contents").is_none());
    }

    #[test]
    fn test_antigravity_response_to_openai_unwraps_response() {
        let body = json!({
            "response": {
                "candidates": [{
                    "content": {
                        "parts": [{"text": "ok"}]
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 6,
                    "candidatesTokenCount": 1
                }
            },
            "traceId": "trace"
        });

        let result =
            translate_response(&body, WireFormat::Antigravity, WireFormat::OpenAiChat).unwrap();
        let choices = result.get("choices").unwrap().as_array().unwrap();
        assert_eq!(
            choices[0].get("message").unwrap().get("content").unwrap(),
            "ok"
        );
        assert_eq!(
            result.get("usage").unwrap().get("prompt_tokens").unwrap(),
            6
        );
        assert_eq!(
            result
                .get("usage")
                .unwrap()
                .get("completion_tokens")
                .unwrap(),
            1
        );
    }

    // ============================================================
    // Tool Call Translation Tests (Phase 1.5)
    // ============================================================

    #[test]
    fn test_openai_to_claude_with_tools() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "What's the weather?"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather for a city",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "city": {"type": "string"}
                        },
                        "required": ["city"]
                    }
                }
            }],
            "tool_choice": "auto",
            "max_tokens": 1024
        });

        let result =
            translate_request(&body, WireFormat::OpenAiChat, WireFormat::ClaudeMessages).unwrap();

        // Tools should be translated to Claude format
        let tools = result.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].get("name").unwrap(), "get_weather");
        assert!(tools[0].get("input_schema").is_some());
        assert!(tools[0].get("parameters").is_none()); // Should be input_schema, not parameters

        // tool_choice should be translated
        let tool_choice = result.get("tool_choice").unwrap();
        assert_eq!(tool_choice.get("type").unwrap(), "auto");
    }

    #[test]
    fn test_openai_to_claude_tool_choice_required() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}],
            "tool_choice": "required"
        });

        let result =
            translate_request(&body, WireFormat::OpenAiChat, WireFormat::ClaudeMessages).unwrap();

        let tool_choice = result.get("tool_choice").unwrap();
        assert_eq!(tool_choice.get("type").unwrap(), "any");
    }

    #[test]
    fn test_openai_to_claude_tool_choice_specific() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}],
            "tool_choice": {"type": "function", "function": {"name": "get_weather"}}
        });

        let result =
            translate_request(&body, WireFormat::OpenAiChat, WireFormat::ClaudeMessages).unwrap();

        let tool_choice = result.get("tool_choice").unwrap();
        assert_eq!(tool_choice.get("type").unwrap(), "tool");
        assert_eq!(tool_choice.get("name").unwrap(), "get_weather");
    }

    #[test]
    fn test_openai_to_claude_parallel_tool_calls() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}],
            "parallel_tool_calls": true
        });

        let result =
            translate_request(&body, WireFormat::OpenAiChat, WireFormat::ClaudeMessages).unwrap();

        // parallel_tool_calls=true → disable_parallel_tool_use=false
        assert_eq!(result.get("disable_parallel_tool_use").unwrap(), false);
    }

    #[test]
    fn test_openai_to_gemini_with_tools() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "What's the weather?"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {"type": "object", "properties": {}}
                }
            }],
            "tool_choice": "auto"
        });

        let result = translate_request(&body, WireFormat::OpenAiChat, WireFormat::Gemini).unwrap();

        // Tools should be translated to Gemini function_declarations
        let tools = result.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 1);
        let func_decls = tools[0]
            .get("function_declarations")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(func_decls.len(), 1);
        assert_eq!(func_decls[0].get("name").unwrap(), "get_weather");

        // tool_choice should be translated to tool_config
        let tool_config = result.get("tool_config").unwrap();
        assert_eq!(
            tool_config
                .get("function_calling_config")
                .unwrap()
                .get("mode")
                .unwrap(),
            "AUTO"
        );
    }

    #[test]
    fn test_claude_response_with_tool_use_to_openai() {
        let body = json!({
            "id": "msg_abc",
            "model": "claude-3-opus",
            "content": [
                {"type": "text", "text": "Let me check the weather."},
                {"type": "tool_use", "id": "call_123", "name": "get_weather", "input": {"city": "SF"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 20}
        });

        let result =
            translate_response(&body, WireFormat::ClaudeMessages, WireFormat::OpenAiChat).unwrap();

        let choices = result.get("choices").unwrap().as_array().unwrap();
        let message = &choices[0].get("message").unwrap();

        // Should have tool_calls
        let tool_calls = message.get("tool_calls").unwrap().as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].get("id").unwrap(), "call_123");
        assert_eq!(tool_calls[0].get("type").unwrap(), "function");
        assert_eq!(
            tool_calls[0].get("function").unwrap().get("name").unwrap(),
            "get_weather"
        );
        // arguments should be a JSON string
        let args = tool_calls[0]
            .get("function")
            .unwrap()
            .get("arguments")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(args.contains("SF"));

        // Text content should be preserved
        assert_eq!(
            message.get("content").unwrap().as_str().unwrap(),
            "Let me check the weather."
        );

        // finish_reason should be tool_calls
        assert_eq!(choices[0].get("finish_reason").unwrap(), "tool_calls");
    }

    #[test]
    fn test_gemini_response_with_function_call_to_openai() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "Checking weather..."},
                        {"functionCall": {"name": "get_weather", "args": {"city": "NYC"}}}
                    ]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 10}
        });

        let result = translate_response(&body, WireFormat::Gemini, WireFormat::OpenAiChat).unwrap();

        let choices = result.get("choices").unwrap().as_array().unwrap();
        let message = &choices[0].get("message").unwrap();

        // Should have tool_calls
        let tool_calls = message.get("tool_calls").unwrap().as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            tool_calls[0].get("function").unwrap().get("name").unwrap(),
            "get_weather"
        );
        let args = tool_calls[0]
            .get("function")
            .unwrap()
            .get("arguments")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(args.contains("NYC"));

        // finish_reason should be tool_calls (has tool calls despite STOP)
        assert_eq!(choices[0].get("finish_reason").unwrap(), "tool_calls");
    }

    #[test]
    fn test_openai_messages_with_tool_calls_to_claude() {
        let body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "What's the weather in SF?"},
                {"role": "assistant", "content": null, "tool_calls": [{
                    "id": "call_abc",
                    "type": "function",
                    "function": {"name": "get_weather", "arguments": "{\"city\": \"SF\"}"}
                }]},
                {"role": "tool", "tool_call_id": "call_abc", "content": "{\"temp\": 72}"}
            ],
            "max_tokens": 1024
        });

        let result =
            translate_request(&body, WireFormat::OpenAiChat, WireFormat::ClaudeMessages).unwrap();

        let messages = result.get("messages").unwrap().as_array().unwrap();
        // Should have 3 messages: user, assistant with tool_use, user with tool_result
        assert_eq!(messages.len(), 3);

        // First: user message
        assert_eq!(messages[0].get("role").unwrap(), "user");

        // Second: assistant with tool_use content block
        assert_eq!(messages[1].get("role").unwrap(), "assistant");
        let content = messages[1].get("content").unwrap().as_array().unwrap();
        let tool_use = content
            .iter()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
            .unwrap();
        assert_eq!(tool_use.get("id").unwrap(), "call_abc");
        assert_eq!(tool_use.get("name").unwrap(), "get_weather");
        assert_eq!(tool_use.get("input").unwrap().get("city").unwrap(), "SF");

        // Third: user with tool_result
        assert_eq!(messages[2].get("role").unwrap(), "user");
        let content = messages[2].get("content").unwrap().as_array().unwrap();
        let tool_result = content
            .iter()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .unwrap();
        assert_eq!(tool_result.get("tool_use_id").unwrap(), "call_abc");
    }

    #[test]
    fn test_openai_messages_with_tool_calls_to_gemini() {
        let body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {"role": "assistant", "content": null, "tool_calls": [{
                    "id": "call_xyz",
                    "type": "function",
                    "function": {"name": "get_weather", "arguments": "{\"city\": \"LA\"}"}
                }]},
                {"role": "tool", "tool_call_id": "call_xyz", "content": "{\"temp\": 80}"}
            ]
        });

        let result = translate_request(&body, WireFormat::OpenAiChat, WireFormat::Gemini).unwrap();

        let contents = result.get("contents").unwrap().as_array().unwrap();
        // Should have 3 contents: user, model with functionCall, user with functionResponse
        assert_eq!(contents.len(), 3);

        // Second: model with functionCall
        assert_eq!(contents[1].get("role").unwrap(), "model");
        let parts = contents[1].get("parts").unwrap().as_array().unwrap();
        let func_call = parts
            .iter()
            .find(|p| p.get("functionCall").is_some())
            .unwrap();
        assert_eq!(
            func_call.get("functionCall").unwrap().get("name").unwrap(),
            "get_weather"
        );

        // Third: user with functionResponse
        assert_eq!(contents[2].get("role").unwrap(), "user");
        let parts = contents[2].get("parts").unwrap().as_array().unwrap();
        assert!(parts.iter().any(|p| p.get("functionResponse").is_some()));
    }
}
