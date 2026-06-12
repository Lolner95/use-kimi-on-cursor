//! Cursor Agent sends OpenAI **Responses API** payloads (`input`, `instructions`,
//! typed items) to `/v1/chat/completions`. Moonshot only understands Chat
//! Completions (`messages`). This module bridges the two formats.

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Default)]
pub struct AdaptStats {
    pub had_input: bool,
    pub had_instructions: bool,
    pub input_item_count: usize,
    pub messages_before: usize,
    pub messages_produced: usize,
    pub seeded_probe: bool,
    pub had_previous_response_id: bool,
}

/// Convert Cursor's Responses-shaped body into Chat Completions `messages`.
/// Safe to call when the body already uses `messages` — it becomes a no-op.
pub fn adapt_cursor_responses_request(obj: &mut Map<String, Value>) -> AdaptStats {
    let mut stats = AdaptStats {
        messages_before: message_count(obj),
        had_input: obj.contains_key("input"),
        had_instructions: obj
            .get("instructions")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty()),
        had_previous_response_id: obj.contains_key("previous_response_id"),
        ..Default::default()
    };

    stats.input_item_count = match obj.get("input") {
        Some(Value::Array(arr)) => arr.len(),
        Some(Value::String(_)) => 1,
        _ => 0,
    };

    if !messages_empty_or_missing(obj) {
        obj.remove("input");
        obj.remove("instructions");
        strip_responses_only_fields(obj);
        stats.messages_produced = stats.messages_before;
        return stats;
    }

    let instructions = obj
        .get("instructions")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let input = obj.remove("input");
    obj.remove("instructions");

    if let Some(input_val) = input {
        let messages = convert_input_to_messages(&input_val, instructions.as_deref());
        stats.messages_produced = messages.len();
        if !messages.is_empty() {
            obj.insert("messages".into(), Value::Array(messages));
        }
    }

    strip_responses_only_fields(obj);
    stats
}

fn message_count(obj: &Map<String, Value>) -> usize {
    obj.get("messages")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

fn messages_empty_or_missing(obj: &Map<String, Value>) -> bool {
    match obj.get("messages") {
        Some(Value::Array(arr)) => arr.is_empty(),
        _ => true,
    }
}

fn strip_responses_only_fields(obj: &mut Map<String, Value>) {
    for key in [
        "previous_response_id",
        "conversation",
        "text",
        "prompt_cache_retention",
        "truncation",
        "include",
        "background",
    ] {
        obj.remove(key);
    }
}

fn convert_input_to_messages(input: &Value, instructions: Option<&str>) -> Vec<Value> {
    let mut messages = Vec::new();

    if let Some(instr) = instructions {
        if !instr.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": instr }));
        }
    }

    match input {
        Value::String(text) => {
            if !text.trim().is_empty() {
                messages.push(json!({ "role": "user", "content": text }));
            }
        }
        Value::Array(items) => {
            messages.extend(convert_input_items(items));
        }
        _ => {}
    }

    messages
}

fn convert_input_items(items: &[Value]) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut pending_tool_calls: Vec<Value> = Vec::new();
    // IDs of tool calls already emitted but not yet answered by an output item.
    let mut unanswered_call_ids: Vec<String> = Vec::new();
    let mut synth_id_counter = 0usize;

    let handle_output = |item: &Value,
                             messages: &mut Vec<Value>,
                             pending: &mut Vec<Value>,
                             unanswered: &mut Vec<String>| {
        flush_pending_tool_calls(messages, pending, unanswered);
        // Cursor occasionally omits call_id on outputs; pair such an output
        // with the oldest unanswered call instead of dropping it.
        let fallback = if extract_call_id(item).is_none() {
            unanswered.first().cloned()
        } else {
            None
        };
        if let Some(msg) = function_call_output_to_message(item, fallback.as_deref()) {
            if let Some(id) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
                if let Some(pos) = unanswered.iter().position(|x| x == id) {
                    unanswered.remove(pos);
                }
            }
            messages.push(msg);
        }
    };

    for item in items {
        let item_type = item.get("type").and_then(|t| t.as_str());

        match item_type {
            Some("function_call") => {
                pending_tool_calls.push(function_call_to_tool_call(item, &mut synth_id_counter));
            }
            Some("function_call_output") => {
                handle_output(
                    item,
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut unanswered_call_ids,
                );
            }
            Some("reasoning") | Some("item_reference") | Some("reasoning_item") => {
                // Kimi chat completions cannot round-trip these; skip.
            }
            Some("input_image") => {
                // Cursor sends standalone image items without a `role`.
                // Convert them into a user message with an image_url part.
                flush_pending_tool_calls(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut unanswered_call_ids,
                );
                let url = item
                    .get("image_url")
                    .and_then(|u| u.as_str())
                    .or_else(|| item.get("file_id").and_then(|f| f.as_str()))
                    .unwrap_or("");
                if !url.is_empty() {
                    messages.push(json!({
                        "role": "user",
                        "content": [
                            { "type": "image_url", "image_url": { "url": url } }
                        ]
                    }));
                }
            }
            Some("message") | None => {
                flush_pending_tool_calls(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut unanswered_call_ids,
                );
                if let Some(msg) = input_item_to_message(item) {
                    messages.push(msg);
                }
            }
            Some(other) if other.ends_with("_call_output") => {
                handle_output(
                    item,
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut unanswered_call_ids,
                );
            }
            _ => {
                flush_pending_tool_calls(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut unanswered_call_ids,
                );
                if let Some(msg) = input_item_to_message(item) {
                    messages.push(msg);
                }
            }
        }
    }

    flush_pending_tool_calls(&mut messages, &mut pending_tool_calls, &mut unanswered_call_ids);
    messages
}

fn flush_pending_tool_calls(
    messages: &mut Vec<Value>,
    pending: &mut Vec<Value>,
    unanswered: &mut Vec<String>,
) {
    if pending.is_empty() {
        return;
    }
    for call in pending.iter() {
        if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
            unanswered.push(id.to_string());
        }
    }
    messages.push(json!({
        "role": "assistant",
        "content": Value::Null,
        "tool_calls": pending.clone()
    }));
    pending.clear();
}

fn input_item_to_message(item: &Value) -> Option<Value> {
    let role = item.get("role").and_then(|r| r.as_str())?;
    let content = item
        .get("content")
        .cloned()
        .or_else(|| item.get("text").cloned())
        .unwrap_or(Value::Null);

    if matches!(content, Value::Null) {
        let has_tool_calls = item
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        if !has_tool_calls {
            return None;
        }
    }

    // Normalize Cursor Responses API content parts into OpenAI Chat format.
    let normalized_content = normalize_content_parts(&content);

    let mut msg = json!({ "role": role, "content": normalized_content });
    if let Some(tool_calls) = item.get("tool_calls") {
        if let Some(obj) = msg.as_object_mut() {
            obj.insert("tool_calls".into(), tool_calls.clone());
        }
    }
    Some(msg)
}

/// Convert Cursor Responses API content types (`input_text`, `input_image`,
/// `image_file`) into standard OpenAI Chat content parts (`text`, `image_url`).
fn normalize_content_parts(content: &Value) -> Value {
    let Some(parts) = content.as_array() else {
        // Not an array — leave strings and nulls as-is.
        return content.clone();
    };

    let mut normalized: Vec<Value> = Vec::new();
    for part in parts {
        let Some(obj) = part.as_object() else {
            normalized.push(part.clone());
            continue;
        };
        let part_type = obj
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("text");

        match part_type {
            "input_text" | "text" => {
                if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                    normalized.push(json!({ "type": "text", "text": text }));
                }
            }
            "input_image" | "image_file" => {
                // Cursor sends image data either as a URL string or as a file_id.
                let url = obj
                    .get("image_url")
                    .and_then(|u| u.as_str())
                    .or_else(|| obj.get("file_id").and_then(|f| f.as_str()))
                    .or_else(|| {
                        obj.get("image_url")
                            .and_then(|u| u.as_object())
                            .and_then(|o| o.get("url").and_then(|u| u.as_str()))
                    })
                    .unwrap_or("");
                if !url.is_empty() {
                    normalized.push(json!({
                        "type": "image_url",
                        "image_url": { "url": url }
                    }));
                }
            }
            "image_url" => {
                // Already in OpenAI format — pass through but normalize the inner structure
                // and strip the unsupported `detail` field (Moonshot rejects it).
                let mut cloned = obj.clone();
                if let Some(url_val) = cloned.get("image_url").cloned() {
                    if url_val.is_string() {
                        cloned.insert(
                            "image_url".into(),
                            json!({ "url": url_val }),
                        );
                    } else if let Some(inner) = url_val.as_object() {
                        let mut cleaned = inner.clone();
                        cleaned.remove("detail");
                        cloned.insert("image_url".into(), Value::Object(cleaned));
                    }
                }
                normalized.push(Value::Object(cloned));
            }
            _ => {
                // Unknown part type — keep as-is; sanitizer will handle it.
                normalized.push(part.clone());
            }
        }
    }

    if normalized.len() == 1 {
        if let Some(text) = normalized[0].get("text").and_then(|t| t.as_str()) {
            return Value::String(text.to_string());
        }
        return normalized.remove(0);
    }
    if normalized.is_empty() {
        content.clone()
    } else {
        Value::Array(normalized)
    }
}

/// Extract a non-empty call ID, trying `call_id` first then `id`.
/// Returns `None` if both are absent or empty — callers must handle this.
fn extract_call_id<'a>(item: &'a Value) -> Option<&'a str> {
    item.get("call_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            item.get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
}

fn function_call_to_tool_call(item: &Value, synth_id_counter: &mut usize) -> Value {
    // Prefer call_id > id; fall back to a unique synthetic value so we never
    // send an empty or duplicated id to Moonshot (both cause HTTP 400).
    let call_id = match extract_call_id(item) {
        Some(id) => id.to_string(),
        None => {
            *synth_id_counter += 1;
            format!("kimi_call_{}", *synth_id_counter)
        }
    };
    let name = item
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let arguments = item.get("arguments").map(|a| {
        if a.is_string() {
            a.clone()
        } else {
            Value::String(a.to_string())
        }
    }).unwrap_or_else(|| json!("{}"));

    json!({
        "id": call_id,
        "type": "function",
        "function": { "name": name, "arguments": arguments }
    })
}

fn function_call_output_to_message(item: &Value, fallback_id: Option<&str>) -> Option<Value> {
    // Prefer call_id > id, then the caller-provided fallback (oldest unanswered
    // call). Skip entirely if we cannot produce a non-empty id — an empty
    // tool_call_id always causes Moonshot HTTP 400.
    let call_id = extract_call_id(item)
        .or(fallback_id)
        .filter(|s| !s.is_empty())?;
    let output = item
        .get("output")
        .or_else(|| item.get("content"))
        .cloned()
        .unwrap_or(Value::String(String::new()));

    let content = if output.is_string() {
        output
    } else {
        Value::String(output.to_string())
    };

    Some(json!({
        "role": "tool",
        "tool_call_id": call_id,
        "content": content
    }))
}

/// Second-line-of-defense repair for tool_call_id mismatches that slip through
/// the adapter (e.g. when Cursor sends mismatched IDs across multiple turns).
/// Walks the messages array and forces every `tool` message's `tool_call_id`
/// to match the nearest preceding assistant `tool_calls` entry by sequential
/// pairing. Returns true if any repair was applied.
pub fn repair_tool_call_ids(body: &mut Map<String, Value>) -> bool {
    let messages = match body.get_mut("messages") {
        Some(Value::Array(arr)) => arr,
        _ => return false,
    };

    let mut repaired = false;
    let mut assistant_call_ids: Vec<String> = Vec::new();

    for msg in messages.iter_mut() {
        let Some(obj) = msg.as_object_mut() else { continue };
        let role = obj.get("role").and_then(|v| v.as_str()).unwrap_or("");

        if role == "assistant" {
            if let Some(calls) = obj.get("tool_calls").and_then(|v| v.as_array()) {
                assistant_call_ids = calls
                    .iter()
                    .filter_map(|c| c.get("id").and_then(|v| v.as_str()).map(String::from))
                    .collect();
            } else {
                assistant_call_ids.clear();
            }
        } else if role == "tool" {
            let current_id = obj
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !assistant_call_ids.is_empty()
                && !assistant_call_ids.iter().any(|id| id == &current_id)
            {
                obj.insert(
                    "tool_call_id".into(),
                    Value::String(assistant_call_ids[0].clone()),
                );
                repaired = true;
            }
            if !assistant_call_ids.is_empty() {
                assistant_call_ids.remove(0);
            }
        }
    }

    repaired
}


/// Seed a placeholder user message only for Cursor's tool-schema validation probe
/// (tools present, no conversation content).
pub fn seed_probe_message_if_needed(obj: &mut Map<String, Value>) -> bool {
    let has_tools = obj
        .get("tools")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());

    if !has_tools || !messages_empty_or_missing(obj) {
        return false;
    }

    obj.insert(
        "messages".into(),
        json!([{ "role": "user", "content": "Hi" }]),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_string_input_to_user_message() {
        let mut obj = Map::from_iter([(
            "input".into(),
            Value::String("Continue the build".into()),
        )]);
        let stats = adapt_cursor_responses_request(&mut obj);
        assert_eq!(stats.messages_produced, 1);
        assert_eq!(obj["messages"][0]["role"], "user");
        assert_eq!(obj["messages"][0]["content"], "Continue the build");
    }

    #[test]
    fn converts_instructions_and_role_messages() {
        let mut obj = Map::from_iter([
            (
                "instructions".into(),
                Value::String("You are a coding agent.".into()),
            ),
            (
                "input".into(),
                Value::Array(vec![
                    json!({ "role": "developer", "content": "Be concise." }),
                    json!({ "role": "user", "content": "Build a CLI todo app" }),
                    json!({ "role": "assistant", "content": "I'll create the project structure." }),
                    json!({ "role": "user", "content": "Continue the build" }),
                ]),
            ),
        ]);
        let stats = adapt_cursor_responses_request(&mut obj);
        assert_eq!(stats.messages_produced, 5);
        assert_eq!(obj["messages"][0]["role"], "system");
        assert_eq!(obj["messages"][4]["content"], "Continue the build");
    }

    #[test]
    fn converts_function_call_items_to_assistant_tool_calls() {
        let mut obj = Map::from_iter([(
            "input".into(),
            Value::Array(vec![
                json!({ "role": "user", "content": "read package.json" }),
                json!({
                    "type": "function_call",
                    "call_id": "call_abc",
                    "name": "read_file",
                    "arguments": "{\"path\":\"package.json\"}"
                }),
                json!({
                    "type": "function_call_output",
                    "call_id": "call_abc",
                    "output": "{\"name\":\"my-app\"}"
                }),
                json!({ "role": "user", "content": "Continue" }),
            ]),
        )]);
        adapt_cursor_responses_request(&mut obj);
        let msgs = obj["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[1]["role"], "assistant");
        assert!(msgs[1]["tool_calls"].is_array());
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["tool_call_id"], "call_abc");
        assert_eq!(msgs[3]["content"], "Continue");
    }

    #[test]
    fn pairs_output_without_call_id_to_oldest_unanswered_call() {
        let mut obj = Map::from_iter([(
            "input".into(),
            Value::Array(vec![
                json!({ "role": "user", "content": "run the tool" }),
                json!({
                    "type": "function_call",
                    "call_id": "call_xyz",
                    "name": "run_cmd",
                    "arguments": "{}"
                }),
                json!({
                    "type": "function_call_output",
                    "output": "done"
                }),
            ]),
        )]);
        adapt_cursor_responses_request(&mut obj);
        let msgs = obj["messages"].as_array().unwrap();
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["tool_call_id"], "call_xyz");
    }

    #[test]
    fn generates_unique_ids_for_calls_missing_ids() {
        let mut obj = Map::from_iter([(
            "input".into(),
            Value::Array(vec![
                json!({ "type": "function_call", "name": "a", "arguments": "{}" }),
                json!({ "type": "function_call", "name": "b", "arguments": "{}" }),
                json!({ "role": "user", "content": "continue" }),
            ]),
        )]);
        adapt_cursor_responses_request(&mut obj);
        let calls = obj["messages"][0]["tool_calls"].as_array().unwrap();
        let id_a = calls[0]["id"].as_str().unwrap();
        let id_b = calls[1]["id"].as_str().unwrap();
        assert!(!id_a.is_empty());
        assert!(!id_b.is_empty());
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn leaves_existing_messages_untouched() {
        let mut obj = Map::from_iter([
            (
                "messages".into(),
                Value::Array(vec![json!({ "role": "user", "content": "hello" })]),
            ),
            ("input".into(), Value::String("ignored".into())),
        ]);
        let stats = adapt_cursor_responses_request(&mut obj);
        assert_eq!(stats.messages_produced, 1);
        assert_eq!(obj["messages"][0]["content"], "hello");
        assert!(obj.get("input").is_none());
    }

    #[test]
    fn probe_seed_only_when_tools_and_no_messages() {
        let mut obj = Map::from_iter([(
            "tools".into(),
            Value::Array(vec![json!({"type":"function","function":{"name":"probe","parameters":{"type":"object","properties":{}}}})]),
        )]);
        assert!(seed_probe_message_if_needed(&mut obj));
        assert_eq!(obj["messages"][0]["content"], "Hi");
    }
}
