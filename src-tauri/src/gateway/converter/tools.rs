//! 工具（function calling）转换：工具名规范化、JSON Schema 清洗与长描述外移。

use super::*;

/// 规范化工具名称为 Kiro API 接受的格式
///
/// Kiro API 要求工具名称必须是纯 camelCase 格式（不能包含下划线或横杠）
/// 将分隔符（_、-、多下划线命名空间前缀）转换为 camelCase 边界
pub fn sanitize_tool_name(name: &str) -> String {
    // 按下划线和横杠分割
    let parts: Vec<&str> = name
        .split(|c| c == '_' || c == '-')
        .filter(|s| !s.is_empty())
        .collect();

    if parts.is_empty() {
        return "tool".to_string();
    }

    // 构建 camelCase：第一部分小写开头，其余部分首字母大写
    let mut result = String::new();
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            // 第一部分：首字母小写
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                result.push_str(&first.to_lowercase().to_string());
                result.push_str(chars.as_str());
            }
        } else {
            // 其余部分：首字母大写
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                result.push_str(&first.to_uppercase().to_string());
                result.push_str(chars.as_str());
            }
        }
    }

    if result.is_empty() {
        "tool".to_string()
    } else {
        result
    }
}

/// 缩短工具名称以符合 Kiro API 的 64 字符限制
///
/// 对于 MCP 工具（格式：mcp__server__tool），尝试缩短为 mcp__tool
/// 其他工具直接截断到 64 字符
pub fn shorten_tool_name(name: &str) -> String {
    if name.len() <= 64 {
        return name.to_string();
    }

    // MCP 工具：mcp__server__tool -> mcp__tool
    if name.starts_with("mcp__") {
        if let Some(last_idx) = name.rfind("__") {
            if last_idx > 5 {
                let shortened = format!("mcp__{}", &name[last_idx + 2..]);
                if shortened.len() <= 64 {
                    return shortened;
                }
            }
        }
    }

    // 直接截断到 64 字符
    name.chars().take(64).collect()
}

/// Anthropic 服务端工具（请求里只有 `type` + `name`，没有 description / input_schema）
/// 由 Anthropic 服务端执行；Kiro 没有对应能力，网关只能降级成普通 function 工具转发。
/// 由于 Kiro 拒绝空 description（见 `tool_description`），这里按 type 补一个准确描述。
fn server_side_tool_description(tool_type: &str) -> Option<&'static str> {
    let tool_type = tool_type.trim().to_ascii_lowercase();
    if tool_type.starts_with("web_search") {
        Some("Search the web for current information; put the search query in `query`.")
    } else if tool_type.starts_with("web_fetch") {
        Some("Fetch the contents of a URL; put the URL in `url`.")
    } else {
        None
    }
}

/// 转换 Anthropic 工具定义，返回 (Tool, Option<(sanitized_name, original_name)>)
pub fn convert_anthropic_tool(
    tool: &crate::gateway::models::AnthropicTool,
) -> (Tool, Option<(String, String)>) {
    let sanitized = shorten_tool_name(&sanitize_tool_name(&tool.name));

    // 截断超长描述（和 Kiro-Go 保持一致）
    let description = tool
        .description
        .as_ref()
        .map(|desc| {
            if desc.len() > TOOL_DESCRIPTION_MAX_LENGTH {
                // 按字节上限截断，但回退到最近的字符边界，避免在多字节字符（中文/emoji）中间切断导致 panic
                let mut end = TOOL_DESCRIPTION_MAX_LENGTH;
                while end > 0 && !desc.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &desc[..end])
            } else {
                desc.clone()
            }
        })
        // 服务端工具没有 description，按 type 补一个可读描述
        .or_else(|| {
            tool.r#type
                .as_deref()
                .and_then(server_side_tool_description)
                .map(str::to_string)
        });

    let converted_tool = Tool {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: sanitized.clone(),
            description,
            parameters: Some(normalize_json_schema(tool.input_schema.clone())),
        },
        cache_control: tool.cache_control.clone(),
    };

    // 如果工具名被修改，记录映射关系
    let mapping = if sanitized != tool.name {
        Some((sanitized, tool.name.clone()))
    } else {
        None
    };

    (converted_tool, mapping)
}

pub fn extract_tool_uses(message: &NormalizedMessage) -> Option<Vec<KiroToolUse>> {
    let tool_calls = message.tool_calls.as_ref()?;
    let tool_uses: Vec<KiroToolUse> = tool_calls
        .iter()
        .map(|tool_call| KiroToolUse {
            // 与 toolSpecification 保持一致：历史里的 tool_use 名字也需 sanitize，否则 Kiro 拒绝（名字不匹配）
            name: shorten_tool_name(&sanitize_tool_name(&tool_call.function.name)),
            input: serde_json::from_str(&tool_call.function.arguments)
                .unwrap_or_else(|_| json!({})),
            tool_use_id: tool_call.id.clone(),
        })
        .collect();

    if tool_uses.is_empty() {
        None
    } else {
        Some(tool_uses)
    }
}

pub fn normalize_tool_choice(
    tool_choice: &Option<Value>,
    tools: &Option<Vec<Tool>>,
) -> Result<Option<Value>, String> {
    let Some(choice) = tool_choice.as_ref() else {
        return Ok(None);
    };

    let choice_type = match choice {
        Value::String(raw) => raw.trim(),
        Value::Object(_) => choice
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .ok_or_else(|| "tool_choice.type 无效".to_string())?,
        _ => return Err("tool_choice 格式无效".to_string()),
    };

    // 官方取值：
    // - Anthropic: auto | any | tool | none
    // - OpenAI Chat/Responses: auto | none | required | function
    // 内部统一为 auto | none | required | function(name)
    match choice_type {
        "auto" => Ok(Some(json!({ "type": "auto" }))),
        "none" => Ok(Some(json!({ "type": "none" }))),
        // Anthropic any ≡ OpenAI required
        "any" | "required" => {
            if tools.as_ref().is_none_or(|items| items.is_empty()) {
                return Err("tool_choice=required/any 时必须同时提供 tools".to_string());
            }
            Ok(Some(json!({ "type": "required" })))
        }
        // Anthropic tool ≡ OpenAI function
        "function" | "tool" => {
            let name = choice
                .get("name")
                .or_else(|| choice.pointer("/function/name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "tool_choice 指定具体工具时 name 不能为空（Anthropic: type=tool+name；OpenAI: type=function+function.name）"
                        .to_string()
                })?;

            // 工具名在转换时会被 sanitize 成 camelCase 并截断到 64 字符
            // （web_search -> webSearch，见 convert_anthropic_tool 及 openai_chat/responses 的
            // 同名处理），而 tool_choice 里带的是客户端原始名。必须做同样的归一化再比对，
            // 否则合法请求会被误判为「工具不存在」——例如 Anthropic 服务端工具
            // {type:"web_search_20250305", name:"web_search"} 配 tool_choice.name="web_search"。
            let normalized = shorten_tool_name(&sanitize_tool_name(name));

            let tool_exists = tools
                .as_ref()
                .map(|items| {
                    items
                        .iter()
                        .any(|tool| tool.function.name == name || tool.function.name == normalized)
                })
                .unwrap_or(false);
            if !tool_exists {
                return Err(format!("tool_choice 指定的工具不存在: {name}"));
            }

            Ok(Some(json!({
                "type": "function",
                // 返回上游实际使用的（已归一化的）工具名
                "name": normalized
            })))
        }
        other => Err(format!(
            "不支持的 tool_choice.type: \"{other}\"，官方取值: auto|none|any|required|tool|function"
        )),
    }
}

pub fn convert_tools(tools: &Option<Vec<Tool>>) -> Option<Vec<KiroTool>> {
    tools.as_ref().map(|items| {
        let mut result = Vec::new();

        // 插入所有工具定义
        for tool in items {
            result.push(KiroTool::ToolSpecification {
                tool_specification: KiroToolSpec {
                    name: tool.function.name.clone(),
                    description: tool_description(tool),
                    input_schema: KiroInputSchema {
                        json: tool_input_schema(tool),
                    },
                },
            });
        }

        // 注意：不要在 tools 数组末尾添加 cachePoint
        // Kiro API 会拒绝这种格式，导致 "Improperly formed request" 错误
        // Prompt Caching 应该通过其他方式触发（如消息的 cache_point metadata）

        result
    })
}

/// 取工具的 Kiro 描述。
///
/// Kiro 会拒绝 `toolSpecification.description` 为空串的工具，实测返回
/// 400 `{"message":"Invalid tool use format.","reason":"REQUEST_BODY_INVALID"}`。
/// 已用四个变体隔离确认：**空 description 是唯一触发条件**，
/// inputSchema 缺少 properties 并不影响（`{"json":{"type":"object"}}` 可以通过）。
///
/// Anthropic 的服务端工具（web_search / web_fetch 等）在请求里本来就没有 description，
/// 所以这里必须兜底成非空文本。
pub fn tool_description(tool: &Tool) -> String {
    let description = tool
        .function
        .description
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();

    if description.is_empty() {
        format!("The {} tool.", tool.function.name)
    } else {
        description.to_string()
    }
}

pub fn tool_input_schema(tool: &Tool) -> Value {
    normalize_json_schema(
        tool.function
            .parameters
            .clone()
            .unwrap_or_else(|| json!({})),
    )
}

pub fn normalize_json_schema(value: Value) -> Value {
    let mut schema = match value {
        Value::Object(map) => map,
        _ => Map::new(),
    };

    // 清理 schema（删除 required: null 和空数组，递归清理嵌套结构）
    clean_schema(&mut schema);

    // 确保顶层有 type: "object"
    if !schema.contains_key("type") {
        schema.insert("type".to_string(), Value::String("object".to_string()));
    }

    Value::Object(schema)
}

/// 递归清理 JSON Schema，删除无效的 required 字段
/// Kiro API 会拒绝 required: null 或空的 required: []
pub fn clean_schema(schema: &mut Map<String, Value>) {
    // 修复 required 字段：必须是非空数组或不存在
    if let Some(required) = schema.get("required") {
        let should_remove = match required {
            Value::Null => true,
            Value::Array(arr) if arr.is_empty() => true,
            _ => false,
        };
        if should_remove {
            schema.remove("required");
        }
    }

    // 递归清理 properties
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        for value in properties.values_mut() {
            if let Value::Object(sub_schema) = value {
                clean_schema(sub_schema);
            }
        }
    }

    // 递归清理 items
    if let Some(Value::Object(items)) = schema.get_mut("items") {
        clean_schema(items);
    }

    // 递归清理 additionalProperties
    if let Some(Value::Object(additional)) = schema.get_mut("additionalProperties") {
        clean_schema(additional);
    }

    // 递归清理 allOf, oneOf, anyOf
    for key in &["allOf", "oneOf", "anyOf"] {
        if let Some(Value::Array(schemas)) = schema.get_mut(*key) {
            for item in schemas {
                if let Value::Object(sub_schema) = item {
                    clean_schema(sub_schema);
                }
            }
        }
    }
}

pub fn process_tools_with_long_descriptions(
    tools: &Option<Vec<Tool>>,
) -> (Option<Vec<Tool>>, Option<String>) {
    let Some(tools) = tools else {
        return (None, None);
    };

    let mut processed = Vec::new();
    let mut long_docs = Vec::new();

    for tool in tools {
        let description = tool.function.description.clone().unwrap_or_default();
        if description.len() > TOOL_DESCRIPTION_MAX_LENGTH {
            long_docs.push(format!(
                "## Tool: {}\n\n{}",
                tool.function.name, description
            ));
            processed.push(Tool {
                tool_type: tool.tool_type.clone(),
                function: ToolFunction {
                    name: tool.function.name.clone(),
                    description: Some(format!(
                        "[Full documentation in system prompt under '## Tool: {}']",
                        tool.function.name
                    )),
                    parameters: tool.function.parameters.clone(),
                },
                cache_control: tool.cache_control.clone(),
            });
        } else {
            processed.push(tool.clone());
        }
    }

    let docs = if long_docs.is_empty() {
        None
    } else {
        Some(format!(
            "# Tool Documentation\n\n{}",
            long_docs.join("\n\n")
        ))
    };

    (Some(processed), docs)
}
