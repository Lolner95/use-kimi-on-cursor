use super::responses_adapter::{adapt_cursor_responses_request, repair_tool_call_ids, seed_probe_message_if_needed};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};

// Placeholder for assistant tool-call history when Cursor drops reasoning_content.
const REASONING_PLACEHOLDER: &str = " ";
const MAX_FUNCTION_NAME_LEN: usize = 64;
const MIN_FUNCTION_NAME_LEN: usize = 3;
const MAX_SCHEMA_DEPTH: usize = 9; // root at depth 0 → total nesting ≤ 10 (Moonshot limit)

/// Moonshot function names must match `^[a-zA-Z_][a-zA-Z0-9-_]{2,63}$`
/// (3–64 chars, start with a letter/underscore).
pub fn sanitize_function_name(raw: &str) -> String {
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    while name.contains("__") {
        name = name.replace("__", "_");
    }
    name = name.trim_matches('_').to_string();

    if name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        name = format!("fn_{name}");
    }

    if name.len() > MAX_FUNCTION_NAME_LEN {
        name.truncate(MAX_FUNCTION_NAME_LEN);
        name = name.trim_end_matches('_').to_string();
    }

    // Pad short names to the 3-char minimum.
    while name.len() < MIN_FUNCTION_NAME_LEN {
        name.push('_');
    }

    if name.is_empty() {
        "fn_tool".to_string()
    } else {
        name
    }
}

fn unique_function_name(raw: &str, registry: &mut HashMap<String, String>) -> String {
    if let Some(existing) = registry.get(raw) {
        return existing.clone();
    }

    let base = sanitize_function_name(raw);
    let mut candidate = base.clone();
    let mut suffix = 2u32;
    while registry.values().any(|v| v == &candidate) {
        let stem = base.chars().take(58).collect::<String>();
        candidate = format!("{stem}_{suffix}");
        suffix += 1;
    }

    registry.insert(raw.to_string(), candidate.clone());
    candidate
}

fn remap_name_field(value: &mut Value, registry: &HashMap<String, String>) {
    if let Some(raw) = value.as_str() {
        if let Some(mapped) = registry.get(raw) {
            *value = Value::String(mapped.clone());
        } else {
            *value = Value::String(sanitize_function_name(raw));
        }
    }
}

#[derive(Debug, Clone)]
pub struct SanitizerConfig {
    pub real_model: String,
    pub force_non_streaming: bool,
    pub thinking_disabled: bool,
    pub sanitize_tools: bool,
    pub max_tokens_default: u32,
    pub max_tokens_cap: u32,
    pub inject_reasoning_placeholder: bool,
}

pub fn map_model_alias(alias: &str, real_model: &str) -> String {
    match alias {
        "gpt-4-turbo"
        | "gpt-4-turbo-preview"
        | "gpt-4o"
        | "gpt-4o-mini"
        | "gpt-4"
        | "gpt-3.5-turbo"
        | "gpt-5"
        | "gpt-5-high"
        | "gpt-5-high-max"
        | "gpt-5.5"
        | "gpt-5.5-high"
        | "o1"
        | "o1-preview"
        | "o3"
        | "o3-mini"
        | "feex-kimi-max" => real_model.to_string(),
        other if other.starts_with("kimi-") => other.to_string(),
        _ => real_model.to_string(),
    }
}

pub fn sanitize_request(mut body: Value, config: &SanitizerConfig) -> Value {
    let Some(obj) = body.as_object_mut() else {
        return body;
    };

    let requested_model = obj
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(&config.real_model)
        .to_string();
    obj.insert(
        "model".into(),
        Value::String(map_model_alias(&requested_model, &config.real_model)),
    );

    if config.thinking_disabled {
        obj.insert("thinking".into(), json!({ "type": "disabled" }));
    } else {
        obj.insert("thinking".into(), json!({ "type": "enabled" }));
    }

    if config.force_non_streaming {
        obj.insert("stream".into(), Value::Bool(false));
        obj.remove("stream_options");
    }

    if let Some(max_completion) = obj.remove("max_completion_tokens") {
        if !obj.contains_key("max_tokens") {
            obj.insert("max_tokens".into(), max_completion);
        }
    }

    normalize_max_tokens(obj, config.max_tokens_default, config.max_tokens_cap);
    strip_unsupported_params(obj);

    // Cursor Agent sends Responses API `input` instead of Chat `messages`.
    let _adapt_stats = adapt_cursor_responses_request(obj);

    // Second-line-of-defense: fix any tool_call_id mismatches that the adapter
    // could not resolve (e.g. IDs that change between Cursor turns).
    let _repaired = repair_tool_call_ids(obj);

    let mut tool_name_map: HashMap<String, String> = HashMap::new();

    if config.sanitize_tools {
        normalize_and_sanitize_tools(obj, &mut tool_name_map);
        sanitize_tool_choice(obj, &tool_name_map);
    }

    seed_probe_message_if_needed(obj);

    if let Some(messages) = obj.get_mut("messages").and_then(|v| v.as_array_mut()) {
        repair_tool_call_pairing(messages);
        for message in messages.iter_mut() {
            sanitize_message(message, config.inject_reasoning_placeholder, &tool_name_map);
        }
    }

    body
}

/// Moonshot rejects a conversation when a `tool` message's `tool_call_id` does
/// not match a tool call declared by the immediately preceding assistant
/// message, when an assistant tool call never receives a tool result, or when
/// tool call IDs are empty/duplicated. Cursor produces all of these in practice
/// (trimmed history, dropped outputs, stale references), which surfaces as
/// HTTP 400 "tool_call_id ... not found". This pass rebuilds a consistent
/// pairing so the request succeeds instead of erroring.
pub fn repair_tool_call_pairing(messages: &mut Vec<Value>) {
    let original = std::mem::take(messages);
    let mut result: Vec<Value> = Vec::with_capacity(original.len());

    // Every tool call ID seen so far (for global uniqueness).
    let mut used_ids: HashSet<String> = HashSet::new();
    // IDs declared by the active assistant block, not yet answered.
    let mut open: Vec<String> = Vec::new();
    // Per-block rewrites (empty/duplicate IDs) so following tool results
    // carrying the old ID still pair up. A queue handles repeated old IDs.
    let mut renames: HashMap<String, VecDeque<String>> = HashMap::new();
    // Index in `result` of the assistant message owning the active tool block.
    let mut block_idx: Option<usize> = None;
    let mut id_counter = 0usize;

    for mut msg in original {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        let has_tool_calls = msg
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());

        if role == "assistant" && has_tool_calls {
            close_open_tool_calls(&mut result, &mut open);
            renames.clear();
            if let Some(calls) = msg.get_mut("tool_calls").and_then(|v| v.as_array_mut()) {
                for call in calls.iter_mut() {
                    let raw_id = call
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let final_id = if !raw_id.is_empty() && used_ids.insert(raw_id.clone()) {
                        raw_id
                    } else {
                        let new_id = next_recovered_id(&mut used_ids, &mut id_counter);
                        renames.entry(raw_id).or_default().push_back(new_id.clone());
                        if let Some(call_obj) = call.as_object_mut() {
                            call_obj.insert("id".into(), Value::String(new_id.clone()));
                        }
                        new_id
                    };
                    open.push(final_id);
                }
            }
            result.push(msg);
            block_idx = Some(result.len() - 1);
        } else if role == "tool" {
            let raw_id = msg
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mapped_id = renames
                .get_mut(&raw_id)
                .and_then(|queue| queue.pop_front())
                .unwrap_or(raw_id);

            if let Some(pos) = open.iter().position(|id| *id == mapped_id) {
                open.remove(pos);
                if let Some(obj) = msg.as_object_mut() {
                    obj.insert("tool_call_id".into(), Value::String(mapped_id));
                }
                result.push(msg);
            } else {
                // Orphaned tool result: synthesize a matching tool call instead
                // of letting Moonshot reject the whole request.
                let final_id = if !mapped_id.is_empty() && used_ids.insert(mapped_id.clone()) {
                    mapped_id
                } else {
                    next_recovered_id(&mut used_ids, &mut id_counter)
                };
                let synthetic_call = json!({
                    "id": final_id.clone(),
                    "type": "function",
                    "function": { "name": "recovered_tool_call", "arguments": "{}" }
                });
                match block_idx {
                    // Extend the active assistant block (keeps ordering valid
                    // even when other tool results in the block are pending).
                    Some(idx) => {
                        if let Some(calls) = result[idx]
                            .get_mut("tool_calls")
                            .and_then(|v| v.as_array_mut())
                        {
                            calls.push(synthetic_call);
                        }
                    }
                    // No active block: insert a synthetic assistant message.
                    None => {
                        result.push(json!({
                            "role": "assistant",
                            "content": Value::Null,
                            "tool_calls": [synthetic_call]
                        }));
                        block_idx = Some(result.len() - 1);
                    }
                }
                if let Some(obj) = msg.as_object_mut() {
                    obj.insert("tool_call_id".into(), Value::String(final_id));
                }
                result.push(msg);
            }
        } else {
            close_open_tool_calls(&mut result, &mut open);
            renames.clear();
            block_idx = None;
            result.push(msg);
        }
    }

    close_open_tool_calls(&mut result, &mut open);
    *messages = result;
}

/// Answer every still-open tool call with a synthetic result so Moonshot never
/// sees an assistant tool call without a matching tool message.
fn close_open_tool_calls(result: &mut Vec<Value>, open: &mut Vec<String>) {
    for id in open.drain(..) {
        result.push(json!({
            "role": "tool",
            "tool_call_id": id,
            "content": "Tool call did not return a result."
        }));
    }
}

fn next_recovered_id(used: &mut HashSet<String>, counter: &mut usize) -> String {
    loop {
        *counter += 1;
        let id = format!("call_recovered_{}", *counter);
        if used.insert(id.clone()) {
            return id;
        }
    }
}

fn strip_unsupported_params(obj: &mut Map<String, Value>) {
    for key in [
        "temperature",
        "top_p",
        "n",
        "presence_penalty",
        "frequency_penalty",
        "logprobs",
        "top_logprobs",
        "seed",
        "reasoning_effort",
        "reasoning",
        "parallel_tool_calls",
        "store",
        "metadata",
        "service_tier",
        "user",
        "prediction",
        "modalities",
        "audio",
        "web_search_options",
    ] {
        obj.remove(key);
    }
    // response_format is preserved - Moonshot supports JSON mode.
    // Only strip if it's not a supported format (e.g. complex JSON schemas).
    if let Some(rf) = obj.get("response_format") {
        if let Some(typ) = rf.get("type").and_then(|t| t.as_str()) {
            if typ != "json_object" && typ != "text" {
                obj.remove("response_format");
            }
        } else if rf.is_object() && rf.get("type").is_none() {
            obj.remove("response_format");
        }
    }
}

fn normalize_max_tokens(obj: &mut Map<String, Value>, default: u32, cap: u32) {
    let requested = obj
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(default);
    let clamped = requested.clamp(1, cap);
    obj.insert("max_tokens".into(), Value::Number(clamped.into()));
}

fn sanitize_tool_choice(obj: &mut Map<String, Value>, registry: &HashMap<String, String>) {
    let Some(choice) = obj.remove("tool_choice") else {
        return;
    };

    let normalized = match choice {
        Value::String(ref s) if s == "auto" || s == "none" || s == "required" => choice,
        Value::Object(mut o) if o.get("type").and_then(|t| t.as_str()) == Some("function") => {
            if let Some(name) = o.remove("name") {
                o.insert("function".into(), json!({ "name": name }));
            }
            if let Some(func) = o.get_mut("function").and_then(|f| f.as_object_mut()) {
                if let Some(name) = func.get_mut("name") {
                    remap_name_field(name, registry);
                }
            }
            Value::Object(o)
        }
        _ => Value::String("auto".into()),
    };

    obj.insert("tool_choice".into(), normalized);
}

fn normalize_and_sanitize_tools(obj: &mut Map<String, Value>, registry: &mut HashMap<String, String>) {
    let Some(tools) = obj.get_mut("tools").and_then(|v| v.as_array_mut()) else {
        return;
    };

    let mut kept: Vec<Value> = Vec::new();
    for tool in tools.iter_mut() {
        if normalize_cursor_tool(tool) {
            sanitize_tool(tool, registry);
            kept.push(tool.clone());
        }
    }
    *tools = kept;
}

/// Cursor Agent may send flat tools (`name` at top level) or `type: custom` entries.
fn normalize_cursor_tool(tool: &mut Value) -> bool {
    let Some(obj) = tool.as_object_mut() else {
        return false;
    };

    let tool_type = obj.get("type").and_then(|t| t.as_str()).unwrap_or("function");
    match tool_type {
        "function" => {
            if !obj.contains_key("function") {
                let name = obj.remove("name");
                let description = obj.remove("description");
                let parameters = obj
                    .remove("parameters")
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                let strict = obj.remove("strict");
                let mut function = Map::new();
                if let Some(n) = name {
                    function.insert("name".into(), n);
                }
                if let Some(d) = description {
                    function.insert("description".into(), d);
                }
                function.insert("parameters".into(), parameters);
                if let Some(s) = strict {
                    function.insert("strict".into(), s);
                }
                obj.insert("function".into(), Value::Object(function));
            }
            true
        }
        "custom" => {
            let name = obj
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("custom_tool")
                .to_string();
            let description = obj
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("Custom Cursor tool")
                .to_string();
            *obj = Map::from_iter([
                ("type".into(), Value::String("function".into())),
                (
                    "function".into(),
                    json!({
                        "name": name,
                        "description": description,
                        "parameters": { "type": "object", "properties": {} }
                    }),
                ),
            ]);
            true
        }
        _ => false,
    }
}

fn sanitize_message(
    message: &mut Value,
    inject_reasoning: bool,
    registry: &HashMap<String, String>,
) {
    let Some(obj) = message.as_object_mut() else {
        return;
    };

    if obj.get("role").and_then(|r| r.as_str()) == Some("developer") {
        obj.insert("role".into(), Value::String("system".into()));
    }

    obj.remove("name");
    obj.remove("cache_control");

    let has_tool_calls = obj
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());

    if has_tool_calls {
        if inject_reasoning {
            let rc = obj
                .get("reasoning_content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if rc.trim().is_empty() {
                obj.insert(
                    "reasoning_content".into(),
                    Value::String(REASONING_PLACEHOLDER.into()),
                );
            }
        }
        if !obj.contains_key("content")
            || obj.get("content").map(|c| c.is_null()).unwrap_or(false)
        {
            obj.insert("content".into(), Value::Null);
        }
        if let Some(tool_calls) = obj.get_mut("tool_calls").and_then(|v| v.as_array_mut()) {
            for tc in tool_calls.iter_mut() {
                sanitize_tool_call(tc, registry);
            }
        }
    } else if obj.get("role").and_then(|r| r.as_str()) == Some("assistant") {
        obj.remove("reasoning_content");
    }

    if obj.get("role").and_then(|r| r.as_str()) == Some("tool") {
        if let Some(name) = obj.get_mut("name") {
            remap_name_field(name, registry);
        }
        if let Some(content) = obj.get("content") {
            if content.is_object() || content.is_array() {
                if let Ok(serialized) = serde_json::to_string(content) {
                    obj.insert("content".into(), Value::String(serialized));
                }
            }
        }
        // Moonshot rejects tool messages with null/missing content.
        let content_missing = obj.get("content").map(|c| c.is_null()).unwrap_or(true);
        if content_missing {
            obj.insert("content".into(), Value::String(" ".into()));
        }
        // tool_call_id pairing is guaranteed by repair_tool_call_pairing,
        // which runs before this per-message pass.
    }

    if let Some(content) = obj.get_mut("content") {
        sanitize_message_content(content);
    }
}

fn sanitize_tool_call(tool_call: &mut Value, registry: &HashMap<String, String>) {
    let Some(obj) = tool_call.as_object_mut() else {
        return;
    };
    if !obj.contains_key("type") {
        obj.insert("type".into(), Value::String("function".into()));
    }
    if let Some(function) = obj.get_mut("function").and_then(|f| f.as_object_mut()) {
        if let Some(name) = function.get_mut("name") {
            remap_name_field(name, registry);
        }
        if let Some(args) = function.get("arguments") {
            if args.is_object() || args.is_array() {
                if let Ok(serialized) = serde_json::to_string(args) {
                    function.insert("arguments".into(), Value::String(serialized));
                }
            }
        }
    }
}

fn sanitize_message_content(content: &mut Value) {
    let Some(parts) = content.as_array_mut() else {
        if let Some(s) = content.as_str() {
            if s.is_empty() {
                *content = Value::String(" ".into());
            }
        }
        return;
    };

    let mut kept_parts: Vec<Value> = Vec::new();
    for part in parts.iter_mut() {
        let Some(part_obj) = part.as_object_mut() else {
            continue;
        };
        part_obj.remove("cache_control");
        let part_type = part_obj
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("text");
        match part_type {
            // Preserve text parts as-is.
            "text" => kept_parts.push(part.clone()),
            // Preserve image_url parts for Kimi vision models.
            "image_url" => {
                // Kimi expects `image_url` with the same shape as OpenAI,
                // but rejects the `detail` field (OpenAI-specific). Strip it.
                let mut normalized = part_obj.clone();
                if let Some(url_val) = normalized.get("image_url").cloned() {
                    if url_val.is_string() {
                        normalized.insert(
                            "image_url".into(),
                            json!({ "url": url_val }),
                        );
                    } else if let Some(obj) = url_val.as_object() {
                        let mut cleaned = obj.clone();
                        cleaned.remove("detail");
                        normalized.insert("image_url".into(), Value::Object(cleaned));
                    }
                }
                kept_parts.push(Value::Object(normalized));
            }
            // Kimi also supports inline base64 images via `image_url`.
            // Keep any other part that has a text fallback, otherwise drop.
            _ => {
                if let Some(text) = part_obj.get("text").and_then(|t| t.as_str()) {
                    kept_parts.push(json!({ "type": "text", "text": text }));
                }
            }
        }
    }

    if kept_parts.is_empty() {
        *content = Value::String(" ".into());
    } else if kept_parts.len() == 1 {
        // If the single part is text, simplify to a plain string for
        // compatibility with text-only consumers.
        if let Some(text) = kept_parts[0].get("text").and_then(|t| t.as_str()) {
            *content = Value::String(text.to_string());
        } else {
            *content = kept_parts.remove(0);
        }
    } else {
        *content = Value::Array(kept_parts);
    }
}

fn sanitize_tool(tool: &mut Value, registry: &mut HashMap<String, String>) {
    let Some(obj) = tool.as_object_mut() else {
        return;
    };
    if obj.get("type").and_then(|t| t.as_str()) != Some("function") {
        return;
    }
    let Some(function) = obj.get_mut("function").and_then(|f| f.as_object_mut()) else {
        return;
    };
    if let Some(name) = function.get("name").and_then(|n| n.as_str()) {
        let mapped = unique_function_name(name, registry);
        function.insert("name".into(), Value::String(mapped));
    } else {
        function.insert(
            "name".into(),
            Value::String(unique_function_name("unnamed_tool", registry)),
        );
    }
    function.remove("strict");
    if !function.contains_key("parameters") {
        function.insert(
            "parameters".into(),
            json!({ "type": "object", "properties": {} }),
        );
    }
    if let Some(params) = function.get_mut("parameters") {
        sanitize_schema(params);
    }
}

/// Public entry: normalize a tool `parameters` schema into Moonshot Flavored JSON
/// Schema (MFJS). The root must be an object schema.
pub fn sanitize_schema(value: &mut Value) {
    normalize_schema_node(value, 0);
    // The root of `parameters` must be an object.
    if let Some(obj) = value.as_object_mut() {
        if !obj.contains_key("type") && !has_combinator(obj) {
            obj.insert("type".into(), Value::String("object".into()));
        }
    }
}

fn has_combinator(obj: &Map<String, Value>) -> bool {
    obj.contains_key("anyOf") || obj.contains_key("oneOf") || obj.contains_key("allOf")
}

fn infer_type_from_value(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "boolean",
        Value::Number(n) => {
            if n.is_f64() {
                "number"
            } else {
                "integer"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Null => "string",
    }
}

/// Ensure a schema node declares a `type` (MFJS requires it for every property).
/// Inference: from `enum`/`const` values when possible, else default to `string`.
fn ensure_type(obj: &mut Map<String, Value>) {
    if obj.contains_key("type") || has_combinator(obj) || obj.contains_key("$ref") {
        return;
    }

    let inferred = if let Some(Value::Array(values)) = obj.get("enum") {
        values.first().map(infer_type_from_value).unwrap_or("string")
    } else if let Some(c) = obj.get("const") {
        infer_type_from_value(c)
    } else if obj.contains_key("properties") {
        "object"
    } else if obj.contains_key("items") {
        "array"
    } else {
        "string"
    };

    obj.insert("type".into(), Value::String(inferred.into()));
}

/// Collapse an over-deep schema node into a shallow valid leaf so the whole
/// request stays within Moonshot's depth limit of 10.
fn collapse_deep_node(obj: &mut Map<String, Value>) {
    let collapsed_type = obj
        .get("type")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if obj.contains_key("properties") || has_combinator(obj) {
                "object".to_string()
            } else if obj.contains_key("items") {
                "array".to_string()
            } else {
                "string".to_string()
            }
        });
    let description = obj
        .get("description")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());

    obj.clear();
    obj.insert("type".into(), Value::String(collapsed_type));
    if let Some(d) = description {
        obj.insert("description".into(), Value::String(d));
    }
}

fn normalize_schema_node(value: &mut Value, depth: usize) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };

    // definitions -> $defs (draft compatibility)
    if let Some(definitions) = obj.remove("definitions") {
        let defs = obj
            .entry("$defs".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let (Value::Object(defs_map), Value::Object(definitions_map)) = (defs, definitions) {
            for (k, v) in definitions_map {
                defs_map.entry(k).or_insert(v);
            }
        }
    }

    // Rewrite $ref paths. Moonshot only accepts refs that start with #/$defs/.
    // Rewrite the legacy #/definitions/ prefix; strip everything else so
    // ensure_type can infer a safe fallback type (avoids HTTP 400 from Moonshot).
    if let Some(Value::String(ref_path)) = obj.get("$ref").cloned() {
        let fixed = ref_path.replace("#/definitions/", "#/$defs/");
        if fixed.starts_with("#/$defs/") {
            obj.insert("$ref".into(), Value::String(fixed));
        } else {
            // Any other $ref format (e.g. bare name, #/properties/X) is not
            // supported by Moonshot - remove it and fall through to ensure_type.
            obj.remove("$ref");
        }
    }

    // Strip keywords Moonshot rejects or ignores.
    obj.remove("$schema");
    obj.remove("strict");
    obj.remove("additionalProperties");
    obj.remove("unevaluatedProperties");

    if depth >= MAX_SCHEMA_DEPTH {
        collapse_deep_node(obj);
        return;
    }

    // MFJS: when using combinators, `type` must live in the children, not the parent.
    if has_combinator(obj) {
        obj.remove("type");
        for key in ["anyOf", "oneOf", "allOf"] {
            if let Some(Value::Array(items)) = obj.get_mut(key) {
                for item in items.iter_mut() {
                    normalize_schema_node(item, depth + 1);
                    if let Some(child) = item.as_object_mut() {
                        ensure_type(child);
                    }
                }
            }
        }
    } else {
        ensure_type(obj);

        if let Some(Value::Object(props)) = obj.get_mut("properties") {
            for (_, prop) in props.iter_mut() {
                normalize_schema_node(prop, depth + 1);
                if let Some(child) = prop.as_object_mut() {
                    ensure_type(child);
                }
            }
        }

        match obj.get_mut("items") {
            Some(Value::Object(_)) => {
                if let Some(items) = obj.get_mut("items") {
                    normalize_schema_node(items, depth + 1);
                    if let Some(child) = items.as_object_mut() {
                        ensure_type(child);
                    }
                }
            }
            Some(Value::Array(arr)) => {
                for item in arr.iter_mut() {
                    normalize_schema_node(item, depth + 1);
                    if let Some(child) = item.as_object_mut() {
                        ensure_type(child);
                    }
                }
            }
            _ => {}
        }

        if let Some(Value::Object(defs)) = obj.get_mut("$defs") {
            for (_, def) in defs.iter_mut() {
                normalize_schema_node(def, depth + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> SanitizerConfig {
        SanitizerConfig {
            real_model: "kimi-k2.7".to_string(),
            force_non_streaming: true,
            thinking_disabled: true,
            sanitize_tools: true,
            max_tokens_default: 32_768,
            max_tokens_cap: 256 * 1024,
            inject_reasoning_placeholder: true,
        }
    }

    #[test]
    fn maps_gpt_alias_to_real_model() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let result = sanitize_request(body, &default_config());
        assert_eq!(result["model"], "kimi-k2.7");
    }

    #[test]
    fn maps_premium_alias_to_real_model() {
        let body = json!({
            "model": "gpt-5-high-max",
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let result = sanitize_request(body, &default_config());
        assert_eq!(result["model"], "kimi-k2.7");
    }

    #[test]
    fn disables_thinking_and_streaming() {
        let body = json!({
            "model": "gpt-4o",
            "stream": true,
            "stream_options": { "include_usage": true },
            "messages": []
        });
        let result = sanitize_request(body, &default_config());
        assert_eq!(result["thinking"], json!({ "type": "disabled" }));
        assert_eq!(result["stream"], false);
        assert!(result.get("stream_options").is_none());
    }

    #[test]
    fn strips_temperature_and_penalties() {
        let body = json!({
            "model": "gpt-4-turbo",
            "temperature": 0,
            "top_p": 0.9,
            "presence_penalty": 0.1,
            "frequency_penalty": 0.2,
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let result = sanitize_request(body, &default_config());
        assert!(result.get("temperature").is_none());
        assert!(result.get("top_p").is_none());
        assert!(result.get("presence_penalty").is_none());
        assert!(result.get("frequency_penalty").is_none());
    }

    #[test]
    fn maps_max_completion_tokens_and_applies_default() {
        let body = json!({
            "model": "gpt-4-turbo",
            "max_completion_tokens": 500,
            "messages": []
        });
        let result = sanitize_request(body, &default_config());
        assert_eq!(result["max_tokens"], 500);

        let no_max = json!({
            "model": "gpt-4-turbo",
            "messages": []
        });
        let result2 = sanitize_request(no_max, &default_config());
        assert_eq!(result2["max_tokens"], 32_768);
    }

    #[test]
    fn injects_reasoning_content_for_tool_call_assistant() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [{
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "read", "arguments": "{}" }
                }]
            }]
        });
        let result = sanitize_request(body, &default_config());
        assert_eq!(result["messages"][0]["reasoning_content"], " ");
    }

    #[test]
    fn maps_developer_role_to_system() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                { "role": "developer", "content": "rules" },
                { "role": "user", "content": "hi" }
            ]
        });
        let result = sanitize_request(body, &default_config());
        assert_eq!(result["messages"][0]["role"], "system");
    }

    #[test]
    fn converts_definitions_to_defs() {
        let mut schema = json!({
            "type": "object",
            "definitions": { "Foo": { "type": "string" } },
            "properties": { "bar": { "$ref": "#/definitions/Foo" } },
            "$schema": "http://json-schema.org/draft-07/schema#",
            "strict": true
        });
        sanitize_schema(&mut schema);
        assert!(schema.get("definitions").is_none());
        assert_eq!(schema["$defs"]["Foo"]["type"], "string");
        assert_eq!(schema["properties"]["bar"]["$ref"], "#/$defs/Foo");
    }

    #[test]
    fn stringifies_tool_message_object_content() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                { "role": "tool", "content": { "result": "ok" }, "tool_call_id": "1" }
            ]
        });
        let result = sanitize_request(body, &default_config());
        // The orphaned tool message gets a synthetic assistant inserted before it.
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["content"], "{\"result\":\"ok\"}");
    }

    #[test]
    fn sanitizes_tool_parameters() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "test",
                    "strict": true,
                    "parameters": {
                        "definitions": { "X": { "type": "number" } },
                        "properties": { "a": { "$ref": "#/definitions/X" } }
                    }
                }
            }]
        });
        let result = sanitize_request(body, &default_config());
        let params = &result["tools"][0]["function"]["parameters"];
        assert!(params.get("definitions").is_none());
        assert_eq!(params["properties"]["a"]["$ref"], "#/$defs/X");
    }

    #[test]
    fn sanitizes_invalid_function_names_from_cursor() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "mcp.filesystem.read_file",
                        "parameters": { "type": "object", "properties": {} }
                    }
                },
                {
                    "type": "custom",
                    "name": "apply_patch",
                    "description": "Apply a patch"
                }
            ]
        });
        let result = sanitize_request(body, &default_config());
        let names: Vec<String> = result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.iter().all(|n| {
            n.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }));
        assert!(names[0].contains("mcp"));
        assert!(!names[0].contains('.'));
    }

    #[test]
    fn seeds_user_message_when_messages_empty() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [],
            "tools": [{
                "type": "function",
                "function": { "name": "probe", "parameters": { "type": "object", "properties": {} } }
            }]
        });
        let result = sanitize_request(body, &default_config());
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn converts_cursor_input_field_in_full_sanitize_pipeline() {
        let body = json!({
            "model": "gpt-4-turbo",
            "instructions": "You are helpful.",
            "input": [
                { "role": "user", "content": "Build a todo app" },
                { "role": "assistant", "content": "Starting scaffold." },
                { "role": "user", "content": "Continue the build" }
            ],
            "tools": [{
                "type": "function",
                "name": "read_file",
                "parameters": { "type": "object", "properties": { "path": { "type": "string" } } }
            }]
        });
        let result = sanitize_request(body, &default_config());
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "instructions + 3 input items");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[3]["content"], "Continue the build");
        assert!(result.get("input").is_none());
    }

    #[test]
    fn normalizes_flat_cursor_tool_format() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [],
            "tools": [{
                "type": "function",
                "name": "server.tool/name",
                "description": "test",
                "parameters": { "type": "object", "properties": {} }
            }]
        });
        let result = sanitize_request(body, &default_config());
        assert!(result["tools"][0]["function"]["name"].is_string());
        assert!(result["tools"][0].get("name").is_none());
    }

    #[test]
    fn remaps_tool_call_names_in_message_history() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [{
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "mcp.foo.bar", "arguments": "{}" }
                }]
            }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "mcp.foo.bar",
                    "parameters": { "type": "object", "properties": {} }
                }
            }]
        });
        let result = sanitize_request(body, &default_config());
        let tool_name = result["tools"][0]["function"]["name"].as_str().unwrap();
        let call_name = result["messages"][0]["tool_calls"][0]["function"]["name"]
            .as_str()
            .unwrap();
        assert_eq!(tool_name, call_name);
        assert!(!tool_name.contains('.'));
    }

    #[test]
    fn mfjs_adds_missing_type_to_enum_property() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "set_mode",
                    "parameters": {
                        "type": "object",
                        "properties": { "mode": { "enum": ["a", "b"] } }
                    }
                }
            }]
        });
        let result = sanitize_request(body, &default_config());
        let mode = &result["tools"][0]["function"]["parameters"]["properties"]["mode"];
        assert_eq!(mode["type"], "string");
    }

    #[test]
    fn mfjs_adds_missing_type_to_plain_property() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "do_thing",
                    "parameters": {
                        "type": "object",
                        "properties": { "x": { "description": "a value" } }
                    }
                }
            }]
        });
        let result = sanitize_request(body, &default_config());
        let x = &result["tools"][0]["function"]["parameters"]["properties"]["x"];
        assert_eq!(x["type"], "string");
    }

    #[test]
    fn mfjs_moves_type_out_of_anyof_parent() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "pick",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "val": {
                                "type": "string",
                                "anyOf": [{ "type": "string" }, { "type": "number" }]
                            }
                        }
                    }
                }
            }]
        });
        let result = sanitize_request(body, &default_config());
        let val = &result["tools"][0]["function"]["parameters"]["properties"]["val"];
        assert!(val.get("type").is_none(), "parent of anyOf must not have type");
        assert_eq!(val["anyOf"][0]["type"], "string");
        assert_eq!(val["anyOf"][1]["type"], "number");
    }

    #[test]
    fn mfjs_caps_schema_depth_at_ten() {
        // Build a schema nested 15 levels deep.
        let mut deep = json!({ "type": "string" });
        for _ in 0..15 {
            deep = json!({ "type": "object", "properties": { "child": deep } });
        }
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{
                "type": "function",
                "function": { "name": "deep_tool", "parameters": deep }
            }]
        });
        let result = sanitize_request(body, &default_config());
        // Walk down and ensure depth never exceeds the limit (no `properties` past depth 10).
        let mut node = &result["tools"][0]["function"]["parameters"];
        let mut depth = 0;
        while let Some(child) = node.get("properties").and_then(|p| p.get("child")) {
            depth += 1;
            node = child;
            assert!(depth <= 10, "schema depth exceeded 10");
        }
    }

    #[test]
    fn repairs_orphan_tool_message_without_matching_call() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                { "role": "user", "content": "hi" },
                { "role": "tool", "tool_call_id": "call_lost", "content": "result" }
            ]
        });
        let result = sanitize_request(body, &default_config());
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_lost");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_lost");
    }

    #[test]
    fn synthesizes_tool_result_for_unanswered_call() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{}" }
                    }]
                },
                { "role": "user", "content": "what happened?" }
            ]
        });
        let result = sanitize_request(body, &default_config());
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
        assert_eq!(messages[2]["role"], "user");
    }

    #[test]
    fn attaches_orphan_tool_result_to_active_block() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_a",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "call_a", "content": "ok" },
                { "role": "tool", "tool_call_id": "call_b", "content": "stale" }
            ]
        });
        let result = sanitize_request(body, &default_config());
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        let calls = messages[0]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2, "orphan id appended to active block");
        assert_eq!(calls[1]["id"], "call_b");
        assert_eq!(messages[2]["tool_call_id"], "call_b");
    }

    #[test]
    fn deduplicates_tool_call_ids_across_blocks() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "dup",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "dup", "content": "first" },
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "dup",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "dup", "content": "second" }
            ]
        });
        let result = sanitize_request(body, &default_config());
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);
        let first_id = messages[0]["tool_calls"][0]["id"].as_str().unwrap();
        let second_id = messages[2]["tool_calls"][0]["id"].as_str().unwrap();
        assert_ne!(first_id, second_id, "duplicate call IDs must be rewritten");
        assert_eq!(messages[1]["tool_call_id"].as_str().unwrap(), first_id);
        assert_eq!(messages[3]["tool_call_id"].as_str().unwrap(), second_id);
    }

    #[test]
    fn rewrites_empty_tool_call_id_and_pairs_result() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "", "content": "ok" }
            ]
        });
        let result = sanitize_request(body, &default_config());
        let messages = result["messages"].as_array().unwrap();
        let call_id = messages[0]["tool_calls"][0]["id"].as_str().unwrap();
        assert!(!call_id.is_empty());
        assert_eq!(messages[1]["tool_call_id"].as_str().unwrap(), call_id);
    }

    #[test]
    fn closes_open_calls_before_next_assistant_block() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{}" }
                    }]
                },
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_2",
                        "type": "function",
                        "function": { "name": "write", "arguments": "{}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "call_2", "content": "ok" }
            ]
        });
        let result = sanitize_request(body, &default_config());
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
        assert_eq!(messages[3]["tool_call_id"], "call_2");
    }

    #[test]
    fn fills_null_tool_message_content() {
        let body = json!({
            "model": "gpt-4-turbo",
            "messages": [
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "call_1", "content": null }
            ]
        });
        let result = sanitize_request(body, &default_config());
        assert_eq!(result["messages"][1]["content"], " ");
    }

    #[test]
    fn mfjs_pads_short_function_names() {
        assert!(sanitize_function_name("id").len() >= 3);
        assert!(sanitize_function_name("a").len() >= 3);
        assert!(sanitize_function_name("x").chars().next().unwrap().is_ascii_alphabetic());
    }
}