//! 工具定义转换与 tool_choice 归一化的回归测试。

use super::*;

/// 回归测试：工具描述超长且截断点落在多字节字符（中文）中间时不得 panic。
/// 旧实现 `&desc[..TOOL_DESCRIPTION_MAX_LENGTH]` 按字节切，会在
/// `byte index ... is not a char boundary` 处 panic（叠加 panic=abort 直接崩进程）。
#[test]
fn convert_anthropic_tool_truncates_multibyte_description_without_panic() {
    // '中' = 3 字节。3413 个 '中' = 10239 字节 > 10237(TOOL_DESCRIPTION_MAX_LENGTH)，
    // 且字节边界只落在 3 的倍数上，10237 不是边界 → 旧代码会 panic。
    let multibyte_desc = "中".repeat(3413);
    assert!(multibyte_desc.len() > TOOL_DESCRIPTION_MAX_LENGTH);
    assert!(
        !multibyte_desc.is_char_boundary(TOOL_DESCRIPTION_MAX_LENGTH),
        "测试前提：截断点必须落在多字节字符中间，否则测不到该 bug"
    );

    let tool = AnthropicTool {
        r#type: Some("custom".to_string()),
        name: "长描述工具".to_string(),
        description: Some(multibyte_desc),
        input_schema: json!({ "type": "object" }),
        cache_control: None,
    };

    // 不 panic 即通过；同时校验截断后的描述本身是合法 UTF-8（隐含于 String 类型）
    let (converted, _mapping) = convert_anthropic_tool(&tool);
    let desc = converted.function.description.expect("描述应存在");
    assert!(desc.ends_with("..."), "超长描述应被截断并加省略号");
    // 截断主体（去掉结尾的 "..."）必须落在字符边界，长度不超过字节上限
    let body_len = desc.len() - "...".len();
    assert!(body_len <= TOOL_DESCRIPTION_MAX_LENGTH);
}

#[test]
fn normalize_tool_choice_maps_anthropic_any_and_tool() {
    // 生产环境里 Tool.function.name 一定是 sanitize 过的 camelCase
    // （convert_anthropic_tool / openai_chat / responses 三处都会 sanitize），
    // 所以这里用 searchDocs，而不是客户端原始名 search_docs。
    let tools = Some(vec![Tool {
        tool_type: "function".to_string(),
        function: crate::gateway::models::ToolFunction {
            name: "searchDocs".to_string(),
            description: Some("搜索".to_string()),
            parameters: Some(json!({"type": "object"})),
        },
        cache_control: None,
    }]);

    assert_eq!(
        normalize_tool_choice(&Some(json!({ "type": "any" })), &tools).unwrap(),
        Some(json!({ "type": "required" }))
    );
    // 客户端用原始名（含下划线）引用工具：必须能匹配到 sanitize 后的名字
    assert_eq!(
        normalize_tool_choice(
            &Some(json!({ "type": "tool", "name": "search_docs" })),
            &tools
        )
        .unwrap(),
        Some(json!({ "type": "function", "name": "searchDocs" }))
    );
    // 直接用归一化后的名字也应匹配
    assert_eq!(
        normalize_tool_choice(
            &Some(json!({ "type": "tool", "name": "searchDocs" })),
            &tools
        )
        .unwrap(),
        Some(json!({ "type": "function", "name": "searchDocs" }))
    );
    assert_eq!(
        normalize_tool_choice(&Some(json!({ "type": "required" })), &tools).unwrap(),
        Some(json!({ "type": "required" }))
    );
    let err = normalize_tool_choice(&Some(json!({ "type": "mystery" })), &tools)
        .expect_err("unknown type should fail");
    assert!(err.contains("不支持的 tool_choice.type"));
}

/// 回归：Anthropic 服务端工具（type=web_search_20250305, name=web_search）
/// 会被 sanitize 成 webSearch，而 tool_choice.name 仍是客户端原始名 web_search。
/// 旧实现直接拿原始名比对 → 误报「tool_choice 指定的工具不存在: web_search」，
/// 使所有带服务端 web 搜索的请求 400。
#[test]
fn normalize_tool_choice_accepts_server_side_tool_original_name() {
    let (converted, mapping) = convert_anthropic_tool(&AnthropicTool {
        r#type: Some("web_search_20250305".to_string()),
        name: "web_search".to_string(),
        description: None,
        input_schema: json!(null),
        cache_control: None,
    });

    assert_eq!(
        converted.function.name, "webSearch",
        "服务端工具名应被 sanitize 成 camelCase"
    );
    assert_eq!(
        mapping,
        Some(("webSearch".to_string(), "web_search".to_string()))
    );

    let tools = Some(vec![converted]);
    let result = normalize_tool_choice(
        &Some(json!({ "type": "tool", "name": "web_search" })),
        &tools,
    )
    .expect("原始工具名应能匹配 sanitize 后的工具，不应报「工具不存在」");
    assert_eq!(
        result,
        Some(json!({ "type": "function", "name": "webSearch" }))
    );
}

/// 回归：超过 64 字符的工具名会被截断，tool_choice 用原始长名也必须能匹配。
#[test]
fn normalize_tool_choice_accepts_truncated_tool_name() {
    let long_name = "a".repeat(80);
    let (converted, _) = convert_anthropic_tool(&AnthropicTool {
        r#type: Some("custom".to_string()),
        name: long_name.clone(),
        description: None,
        input_schema: json!({"type": "object"}),
        cache_control: None,
    });
    assert_eq!(converted.function.name.len(), 64);

    let tools = Some(vec![converted]);
    assert!(
        normalize_tool_choice(&Some(json!({ "type": "tool", "name": long_name })), &tools).is_ok(),
        "超长工具名应经截断后匹配成功"
    );
}

/// 回归：Kiro 拒绝空的 `toolSpecification.description`，实测返回
/// 400 `{"message":"Invalid tool use format.","reason":"REQUEST_BODY_INVALID"}`。
/// 已用四变体隔离确认空 description 是唯一触发条件（inputSchema 缺 properties 不影响）。
/// Anthropic 服务端工具（type=web_search_20250305）请求里没有 description，
/// 必须在转换时补成非空，否则整条转发 400。
#[test]
fn kiro_tool_specification_description_is_never_empty() {
    let (converted, _) = convert_anthropic_tool(&AnthropicTool {
        r#type: Some("web_search_20250305".to_string()),
        name: "web_search".to_string(),
        description: None,
        input_schema: json!(null),
        cache_control: None,
    });

    assert!(
        !tool_description(&converted).trim().is_empty(),
        "服务端工具必须补出非空 description"
    );

    // 走完整转换链，确认最终发往 Kiro 的 toolSpecification.description 非空
    let kiro_tools = convert_tools(&Some(vec![converted])).expect("tools should convert");
    let mut checked = false;
    for tool in &kiro_tools {
        if let KiroTool::ToolSpecification { tool_specification } = tool {
            assert!(
                !tool_specification.description.trim().is_empty(),
                "Kiro toolSpecification.description 不能为空，否则上游 400"
            );
            checked = true;
        }
    }
    assert!(checked, "应至少产生一个 ToolSpecification");
}

/// 普通工具的空 / 纯空白 description 也要兜底成非空。
#[test]
fn tool_description_falls_back_for_blank_description() {
    let make = |name: &str, desc: Option<String>| Tool {
        tool_type: "function".to_string(),
        function: crate::gateway::models::ToolFunction {
            name: name.to_string(),
            description: desc,
            parameters: Some(json!({"type": "object"})),
        },
        cache_control: None,
    };

    assert_eq!(
        tool_description(&make("noDesc", None)),
        "The noDesc tool.",
        "缺失 description 应兜底"
    );
    assert_eq!(
        tool_description(&make("blankDesc", Some("   ".to_string()))),
        "The blankDesc tool.",
        "纯空白 description 也应兜底"
    );
    // 显式提供的 description 必须原样保留
    assert_eq!(
        tool_description(&make("searchDocs", Some("搜索文档".to_string()))),
        "搜索文档"
    );
}
