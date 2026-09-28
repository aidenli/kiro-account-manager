//! 服务端 web_search 模拟（Path A）。
//!
//! Anthropic 的 `web_search_20250305` 是**服务端工具**：搜索由 Anthropic 后端执行，
//! 并以 `server_tool_use` + `web_search_tool_result` 内容块返回。Kiro 的
//! `generateAssistantResponse` 没有这种服务端能力，于是这类请求此前要么被网关的
//! `tool_choice` 校验拒绝，要么被降级成普通 function 工具，模型只能回一个客户端无法
//! 履行的 `tool_use`，客户端最终渲染出**空的**搜索结果。
//!
//! 这里改由网关自己执行搜索：直接调用 Kiro 的 MCP 端点
//! `POST https://runtime.{region}.kiro.dev/mcp`（与 `generateAssistantResponse` 同域），
//! 用账号的 accessToken + profileArn 发 `tools/call` → `web_search`，再把真实结果按
//! Anthropic 服务端工具的格式回吐。
//!
//! 两个实测确认的协议细节：
//! - `profileArn` 必须放在 JSON-RPC **body 顶层**；放 query 参数 / header / `params`
//!   都会返回 400 `profileArn is required for this request.`。
//! - 响应里 `result.content[0].text` 是**字符串化的 JSON**，要再解析一次才拿到 `results[]`。
//!
//! 范围：只处理「纯服务端搜索请求」（tools 全部是服务端 web_search 工具），
//! 即 Claude Code 的 WebSearch 子请求形态。模型在普通对话中自行决定搜索的场景
//! （Path B：流式拦截 + 内部续跑）**未实现**。
//!
//! 由 `GatewayConfig.web_search_emulation` 开关控制，默认关闭。

use super::*;

/// 搜索命中的一条结果
#[derive(Debug, Clone, Default)]
pub struct WebSearchItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// 一次搜索的结果集合
#[derive(Debug, Clone, Default)]
pub struct WebSearchResults {
    pub items: Vec<WebSearchItem>,
    pub total: usize,
}

/// 判断是否为「纯服务端 web_search 请求」，是则返回搜索 query。
///
/// Claude Code 的 WebSearch 会单独发一条这样的请求：
/// ```json
/// { "tools": [{"type":"web_search_20250305","name":"web_search","max_uses":8}],
///   "tool_choice": {"type":"tool","name":"web_search"},
///   "messages": [{"role":"user","content":[{"type":"text",
///                  "text":"Perform a web search for the query: X"}]}] }
/// ```
///
/// 判据从严：`tools` 必须非空且**全部**是服务端 web_search 工具。普通对话的 tools
/// 要么为空、要么是客户端 function 工具，因此不会被误伤。
pub fn detect_pure_web_search(payload: &Value) -> Option<String> {
    let tools = payload.get("tools")?.as_array()?;
    if tools.is_empty() || !tools.iter().all(is_server_web_search_tool) {
        return None;
    }
    extract_query(payload)
}

fn is_server_web_search_tool(tool: &Value) -> bool {
    tool.get("type")
        .and_then(Value::as_str)
        .map(|value| {
            value
                .trim()
                .to_ascii_lowercase()
                .starts_with("web_search")
        })
        .unwrap_or(false)
}

fn extract_query(payload: &Value) -> Option<String> {
    let messages = payload.get("messages")?.as_array()?;
    let first_user = messages
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))?;
    let text = message_text(first_user.get("content")?);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    const PREFIX: &str = "Perform a web search for the query:";
    let query = text.strip_prefix(PREFIX).unwrap_or(text).trim();
    if query.is_empty() {
        None
    } else {
        Some(query.to_string())
    }
}

fn message_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn parse_results(value: &Value) -> WebSearchResults {
    let raw = value
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let total = value
        .get("totalResults")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(raw.len());

    let items = raw
        .iter()
        .map(|item| WebSearchItem {
            title: item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            url: item
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            snippet: item
                .get("snippet")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect();

    WebSearchResults { items, total }
}

/// 调用 Kiro MCP 执行一次 web_search。
pub async fn call_kiro_mcp_web_search(
    http: &Client,
    access_token: &str,
    profile_arn: &str,
    region: &str,
    query: &str,
    send_opt_out: bool,
    timeout_secs: u64,
) -> Result<WebSearchResults, String> {
    let region = region.trim();
    let region = if region.is_empty() { "us-east-1" } else { region };
    let url = format!("https://runtime.{region}.kiro.dev/mcp");

    let request_id = format!(
        "web_search_tooluse_{}_{}",
        uuid::Uuid::new_v4().simple(),
        chrono::Utc::now().timestamp_millis()
    );

    let body = json!({
        "id": request_id,
        "jsonrpc": "2.0",
        // 必须放顶层，否则 400 "profileArn is required for this request."
        "profileArn": profile_arn,
        "method": "tools/call",
        "params": {
            "name": "web_search",
            "arguments": { "query": query }
        }
    });

    let response = http
        .post(&url)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {access_token}"),
        )
        .header(
            "x-amzn-codewhisperer-optout",
            if send_opt_out { "true" } else { "false" },
        )
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json")
        .timeout(Duration::from_secs(timeout_secs.max(5)))
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Kiro MCP 搜索请求失败: {error}"))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "Kiro MCP 搜索返回 {status}: {}",
            safe_truncate(&text, 200)
        ));
    }

    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("Kiro MCP 搜索响应解析失败: {error}"))?;

    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Err(format!(
            "Kiro MCP 搜索返回错误: {}",
            safe_truncate(&error.to_string(), 200)
        ));
    }

    // result.content[0].text 是字符串化的 JSON，需要再解析一次
    let inner = value
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .ok_or_else(|| "Kiro MCP 搜索响应缺少 result.content[0].text".to_string())?;

    let parsed: Value = serde_json::from_str(inner)
        .map_err(|error| format!("Kiro MCP 搜索结果解析失败: {error}"))?;

    Ok(parse_results(&parsed))
}

/// 把结果整理成给客户端/模型看的文本
fn format_results_text(query: &str, results: &WebSearchResults) -> String {
    let mut out = format!("Web search results for query: \"{query}\"\n\n");
    if results.items.is_empty() {
        out.push_str("No results found.\n");
        return out;
    }

    for (index, item) in results.items.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", index + 1, item.title));
        if !item.url.is_empty() {
            out.push_str(&format!("   {}\n", item.url));
        }
        if !item.snippet.is_empty() {
            out.push_str(&format!("   {}\n", item.snippet));
        }
        out.push('\n');
    }
    out
}

fn server_tool_use_block(tool_use_id: &str, query: &str) -> Value {
    json!({
        "type": "server_tool_use",
        "id": tool_use_id,
        "name": "web_search",
        "input": { "query": query }
    })
}

fn web_search_tool_result_block(tool_use_id: &str, results: &WebSearchResults) -> Value {
    let content: Vec<Value> = results
        .items
        .iter()
        .map(|item| {
            json!({
                "type": "web_search_result",
                "title": item.title,
                "url": item.url,
                "encrypted_content": item.snippet,
                "page_age": Value::Null
            })
        })
        .collect();

    json!({
        "type": "web_search_tool_result",
        "tool_use_id": tool_use_id,
        "content": content
    })
}

fn anthropic_error(status: StatusCode, error_type: &str, message: &str) -> Response {
    (
        status,
        Json(json!({
            "type": "error",
            "error": { "type": error_type, "message": message }
        })),
    )
        .into_response()
}

fn sse_event(event: &str, data: &Value) -> Bytes {
    Bytes::from(format!("event: {event}\ndata: {data}\n\n"))
}

/// 非流式响应：一次性返回 server_tool_use + web_search_tool_result + 结果文本
fn non_streaming_response(
    model: &str,
    query: &str,
    tool_use_id: &str,
    results: &WebSearchResults,
) -> Response {
    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    Json(json!({
        "id": message_id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [
            server_tool_use_block(tool_use_id, query),
            web_search_tool_result_block(tool_use_id, results),
            { "type": "text", "text": format_results_text(query, results) }
        ],
        "stop_reason": "end_turn",
        "stop_sequence": Value::Null,
        "usage": { "input_tokens": 0, "output_tokens": 0 }
    }))
    .into_response()
}

/// 流式响应：按 Anthropic SSE 事件序列回吐
fn streaming_response(
    model: String,
    query: String,
    tool_use_id: String,
    results: WebSearchResults,
) -> Response {
    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(16);

    tokio::spawn(async move {
        let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
        let summary = format_results_text(&query, &results);

        let frames: Vec<(&str, Value)> = vec![
            (
                "message_start",
                json!({
                    "type": "message_start",
                    "message": {
                        "id": message_id,
                        "type": "message",
                        "role": "assistant",
                        "model": model,
                        "content": [],
                        "stop_reason": Value::Null,
                        "usage": { "input_tokens": 0, "output_tokens": 0 }
                    }
                }),
            ),
            (
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": server_tool_use_block(&tool_use_id, &query)
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": json!({ "query": query }).to_string()
                    }
                }),
            ),
            (
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": 0 }),
            ),
            (
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": web_search_tool_result_block(&tool_use_id, &results)
                }),
            ),
            (
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": 1 }),
            ),
            (
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": 2,
                    "content_block": { "type": "text", "text": "" }
                }),
            ),
        ];

        for (event, data) in frames {
            if tx.send(Ok(sse_event(event, &data))).await.is_err() {
                return;
            }
        }

        // 结果文本分片下发
        const CHUNK: usize = 200;
        let chars: Vec<char> = summary.chars().collect();
        for chunk in chars.chunks(CHUNK) {
            let piece: String = chunk.iter().collect();
            let data = json!({
                "type": "content_block_delta",
                "index": 2,
                "delta": { "type": "text_delta", "text": piece }
            });
            if tx
                .send(Ok(sse_event("content_block_delta", &data)))
                .await
                .is_err()
            {
                return;
            }
        }

        let tail = [
            (
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": 2 }),
            ),
            (
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": { "stop_reason": "end_turn", "stop_sequence": Value::Null },
                    "usage": { "output_tokens": 0 }
                }),
            ),
            ("message_stop", json!({ "type": "message_stop" })),
        ];
        for (event, data) in tail {
            if tx.send(Ok(sse_event(event, &data))).await.is_err() {
                return;
            }
        }
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .header(header::CONNECTION, HeaderValue::from_static("keep-alive"))
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// 处理被模拟的服务端 web_search 请求。
pub async fn handle_emulated_web_search(
    state: &RouterState,
    stream: bool,
    model: &str,
    query: &str,
    preferred_account_id: Option<&str>,
) -> Response {
    let upstream =
        match resolve_upstream_credentials(&state.config, state, preferred_account_id).await {
            Ok(credentials) => credentials,
            Err(message) => {
                return anthropic_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "api_error",
                    &format!("web_search 取账号失败: {}", sanitize_error(&message)),
                );
            }
        };

    let Some(profile_arn) = upstream
        .profile_arn
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return anthropic_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            "web_search 需要 profileArn，但当前账号没有可用值",
        );
    };

    log::info!(
        "[网关] 模拟服务端 web_search（账号 {}）query={}",
        sanitize_error(&upstream.source_label),
        query
    );

    let results = match call_kiro_mcp_web_search(
        &upstream.http,
        &upstream.access_token,
        profile_arn,
        &upstream.region,
        query,
        upstream.send_opt_out,
        state.config.web_search_timeout_secs,
    )
    .await
    {
        Ok(results) => results,
        Err(message) => {
            log::warn!("[网关] web_search 失败: {}", sanitize_error(&message));
            return anthropic_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                &sanitize_error(&message),
            );
        }
    };

    log::info!("[网关] web_search 完成: {} 条结果", results.total);

    let tool_use_id = format!("srvtoolu_{}", uuid::Uuid::new_v4().simple());

    if stream {
        streaming_response(model.to_string(), query.to_string(), tool_use_id, results)
    } else {
        non_streaming_response(model, query, &tool_use_id, &results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc_search_payload(query: &str) -> Value {
        json!({
            "model": "claude-opus-5.5-thinking",
            "max_tokens": 64000,
            "stream": true,
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": format!("Perform a web search for the query: {query}") }]
            }],
            "tool_choice": { "type": "tool", "name": "web_search" },
            "tools": [{ "type": "web_search_20250305", "name": "web_search", "max_uses": 8 }]
        })
    }

    #[test]
    fn detects_claude_code_web_search_request() {
        assert_eq!(
            detect_pure_web_search(&cc_search_payload("刘欢 歌手 最新消息")),
            Some("刘欢 歌手 最新消息".to_string())
        );
    }

    #[test]
    fn detects_bare_query_without_prefix() {
        let mut payload = cc_search_payload("x");
        payload["messages"][0]["content"][0]["text"] = json!("just a query");
        assert_eq!(
            detect_pure_web_search(&payload),
            Some("just a query".to_string())
        );
    }

    #[test]
    fn ignores_normal_client_tool_requests() {
        // 客户端 function 工具（带 input_schema）不应被当成服务端搜索
        let payload = json!({
            "model": "claude-opus-5.5-thinking",
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }] }],
            "tools": [{
                "name": "WebSearch",
                "description": "search",
                "input_schema": { "type": "object", "properties": { "query": { "type": "string" } } }
            }]
        });
        assert_eq!(detect_pure_web_search(&payload), None);
    }

    #[test]
    fn ignores_requests_without_tools() {
        let payload = json!({
            "model": "claude-opus-5.5-thinking",
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }] }]
        });
        assert_eq!(detect_pure_web_search(&payload), None);
    }

    #[test]
    fn ignores_mixed_server_and_client_tools() {
        // 混入客户端工具 → 交给模型决定（Path B，本实现不接管）
        let mut payload = cc_search_payload("x");
        payload["tools"] = json!([
            { "type": "web_search_20250305", "name": "web_search" },
            { "name": "Bash", "input_schema": { "type": "object" } }
        ]);
        assert_eq!(detect_pure_web_search(&payload), None);
    }

    #[test]
    fn parses_mcp_results_payload() {
        let parsed = json!({
            "results": [
                { "title": "T1", "url": "https://a.example", "snippet": "S1", "publishedDate": 1 },
                { "title": "T2", "url": "https://b.example", "snippet": "S2" }
            ],
            "totalResults": 10,
            "query": "q"
        });
        let results = parse_results(&parsed);
        assert_eq!(results.total, 10);
        assert_eq!(results.items.len(), 2);
        assert_eq!(results.items[0].title, "T1");
        assert_eq!(results.items[0].url, "https://a.example");
        assert_eq!(results.items[1].snippet, "S2");
    }

    #[test]
    fn formats_results_text_with_all_items() {
        let results = WebSearchResults {
            items: vec![WebSearchItem {
                title: "标题".to_string(),
                url: "https://example.com".to_string(),
                snippet: "摘要".to_string(),
            }],
            total: 1,
        };
        let text = format_results_text("q", &results);
        assert!(text.contains("Web search results for query: \"q\""));
        assert!(text.contains("标题"));
        assert!(text.contains("https://example.com"));
        assert!(text.contains("摘要"));
    }

    #[test]
    fn tool_result_block_shape_matches_anthropic_server_tool() {
        let results = WebSearchResults {
            items: vec![WebSearchItem {
                title: "T".to_string(),
                url: "https://example.com".to_string(),
                snippet: "S".to_string(),
            }],
            total: 1,
        };
        let block = web_search_tool_result_block("srvtoolu_x", &results);
        assert_eq!(block["type"], "web_search_tool_result");
        assert_eq!(block["tool_use_id"], "srvtoolu_x");
        assert_eq!(block["content"][0]["type"], "web_search_result");
        assert_eq!(block["content"][0]["url"], "https://example.com");
    }
}
