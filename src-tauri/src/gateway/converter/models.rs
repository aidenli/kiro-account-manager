//! 模型 ID 映射：公网模型名 <-> Kiro 上游内部模型 ID，以及可用模型清单。

use super::*;

pub fn get_internal_model_id(external_model: &str) -> Result<String, String> {
    let normalized = normalize_external_model_alias(external_model);

    // 1. 特殊别名（简写 / latest / 特殊值）
    let model_id = match normalized.as_str() {
        "auto" | "default" => return Ok("auto".to_string()),
        // opus 系列：裸 "opus" 指向当前最新的 Opus（5）
        "opus" | "opus-5" | "opus-5-0" => return Ok("claude-opus-5".to_string()),
        "opus-4-7" => return Ok("claude-opus-4.7".to_string()),
        "sonnet" | "sonnet-4-6" => return Ok("claude-sonnet-4.6".to_string()),
        "haiku" | "haiku-4-5" => return Ok("claude-haiku-4.5".to_string()),
        "claude-sonnet-latest" => return Ok("claude-sonnet-5".to_string()),
        "claude-sonnet-5" => return Ok("claude-sonnet-5".to_string()),
        // OpenAI GPT 兼容映射（默认映射到 claude-sonnet-4；用户可以在前端「模型映射」配置里覆盖）
        "gpt-4" | "gpt-4o" | "gpt-4-turbo" | "gpt-3.5-turbo" | "gpt-4o-mini" => {
            return Ok("claude-sonnet-4".to_string());
        }
        // GPT-5.6 系列（Kiro 原生 GPT 模型，不做映射）
        "gpt-5-6-sol" | "gpt-5.6-sol" => return Ok("gpt-5.6-sol".to_string()),
        "gpt-5-6-terra" | "gpt-5.6-terra" => return Ok("gpt-5.6-terra".to_string()),
        "gpt-5-6-luna" | "gpt-5.6-luna" => return Ok("gpt-5.6-luna".to_string()),
        "gpt-5-6" | "gpt-5.6" | "gpt5.6" => return Ok("gpt-5.6-sol".to_string()),
        // 开源模型别名
        "deepseek-3-2" | "deepseek-3.2" | "deepseek" => return Ok("deepseek-3.2".to_string()),
        "minimax-m2-5" | "minimax-m2.5" | "minimax" => return Ok("minimax-m2.5".to_string()),
        "minimax-m2-1" | "minimax-m2.1" => return Ok("minimax-m2.1".to_string()),
        "glm-5" | "glm5" => return Ok("glm-5".to_string()),
        "qwen3-coder-next" | "qwen3-coder" | "qwen3" | "qwen" => {
            return Ok("qwen3-coder-next".to_string())
        }
        _ => &normalized,
    };

    // 2. 正则归一化：Anthropic 公开格式 → Kiro 内部格式
    //    claude-{family}-{major}-{minor}[-thinking][-日期] → claude-{family}-{major}.{minor}
    let normalized_model = normalize_claude_model_format(model_id);

    // 3. 兜底：如果归一化后仍然不像 Kiro 支持的格式，映射到默认 sonnet-4.5 避免直接 400
    //    向前兼容：claude-{sonnet|haiku|opus}-* 格式透传，假定 Kiro 后续新发布的版本格式不变
    if is_kiro_supported_model_format(&normalized_model) {
        Ok(normalized_model)
    } else {
        log::warn!(
            "[模型映射] 未知模型 \"{}\" → 兜底到 claude-sonnet-4.5",
            external_model
        );
        Ok("claude-sonnet-4.5".to_string())
    }
}

/// 判断模型 ID 是否符合 Kiro API 接受的格式
/// - claude-{sonnet|haiku|opus}-{version} （包括 4.5 / 4.6 / 4.7 / 5 + 未来新版本）
/// - gpt-5.6-{sol|terra|luna} （Kiro 原生 GPT 模型）
/// - 开源模型：deepseek-3.2 / minimax-m2.5 / minimax-m2.1 / glm-5 / qwen3-coder-next
/// - 特殊值：auto
pub fn is_kiro_supported_model_format(model: &str) -> bool {
    if model == "auto" {
        return true;
    }
    if model.starts_with("claude-sonnet-")
        || model.starts_with("claude-haiku-")
        || model.starts_with("claude-opus-")
    {
        return true;
    }
    if model.starts_with("gpt-5.6-") || model.starts_with("gpt-5-6-") {
        return true;
    }
    matches!(
        model,
        "deepseek-3.2" | "minimax-m2.5" | "minimax-m2.1" | "glm-5" | "qwen3-coder-next"
    )
}

/// 将 Anthropic 公开模型名归一化为 Kiro 内部格式
///
/// 规则：
/// - 去掉日期后缀 -20xxxxxx（8位数字）
/// - 版本号横杠转点号：claude-{family}-{major}-{minor} → claude-{family}-{major}.{minor}
/// - 保留 -thinking 后缀（Kiro 通过模型 ID 区分是否启用思考）
/// - 已经是点号格式的直接返回
pub fn normalize_claude_model_format(model: &str) -> String {
    let mut s = model.to_string();

    // 去掉 -thinking 后缀（thinking 通过系统提示注入启用，Kiro API 不接受带 -thinking 的模型 ID）
    if let Some(stripped) = s.strip_suffix("-thinking") {
        s = stripped.to_string();
    }

    // 去掉日期后缀（-20xxxxxx，8位数字）
    if s.len() > 9 {
        let tail = &s[s.len() - 9..];
        if tail.starts_with('-')
            && tail[1..].chars().all(|c| c.is_ascii_digit())
            && tail[1..].starts_with("20")
        {
            s.truncate(s.len() - 9);
        }
    }

    // 版本号横杠转点号：claude-{family}-{major}-{minor} → claude-{family}-{major}.{minor}
    // 匹配模式：末尾是 -{digit}-{digit} 的情况
    if let Some(last_dash) = s.rfind('-') {
        let after_last = &s[last_dash + 1..];
        if after_last.len() == 1 && after_last.chars().all(|c| c.is_ascii_digit()) {
            // 检查倒数第二个 dash 后面是否也是单个数字
            let prefix = &s[..last_dash];
            if let Some(second_last_dash) = prefix.rfind('-') {
                let between = &prefix[second_last_dash + 1..];
                if between.len() == 1 && between.chars().all(|c| c.is_ascii_digit()) {
                    // claude-opus-4-7 → claude-opus-4.7
                    let base = &s[..second_last_dash + 1 + between.len()];
                    return format!("{}.{}", base, after_last);
                }
            }
        }
    }

    // GPT-5.6 系列横杠转点号：gpt-5-6-sol → gpt-5.6-sol
    // 匹配模式：gpt-{major}-{minor}-{variant}
    if s.starts_with("gpt-") {
        // 去掉 gpt- 前缀
        let rest = &s[4..];
        // 找第一个横杠（major 和 minor 之间）
        if let Some(first_dash) = rest.find('-') {
            let major = &rest[..first_dash];
            if major.len() == 1 && major.chars().all(|c| c.is_ascii_digit()) {
                let after_major = &rest[first_dash + 1..];
                // 找第二个横杠（minor 和 variant 之间）
                if let Some(second_dash) = after_major.find('-') {
                    let minor = &after_major[..second_dash];
                    if minor.len() == 1 && minor.chars().all(|c| c.is_ascii_digit()) {
                        let variant = &after_major[second_dash + 1..];
                        // gpt-5-6-sol → gpt-5.6-sol
                        return format!("gpt-{}.{}-{}", major, minor, variant);
                    }
                }
            }
        }
    }

    s
}

/// 带降级的模型映射函数
///
/// 根据账号可用模型列表（来自 ListAvailableModels API），自动将不可用的模型降级
///
/// ## 降级策略
///
/// Free 用户可用模型：sonnet-4.5, sonnet-4, haiku-4.5, 开源模型
/// Free 用户不可用：所有 Opus 系列、Sonnet 4.6+
/// GPT-5.6 不可用时降级到 gpt-5.6-luna（最便宜变体），避免跨协议降级
///
/// 简单策略：所有不可用模型一律降级到 claude-sonnet-4.5（保留 -thinking 后缀）
pub fn get_internal_model_id_with_fallback(
    external_model: &str,
    available_models: &[String],
) -> Result<String, String> {
    let mapped_model = get_internal_model_id(external_model)?;

    // 检查是否在可用列表中
    if available_models.contains(&mapped_model) {
        return Ok(mapped_model);
    }

    // 检测原始模型名是否要求 thinking（用于降级后保留 -thinking 后缀）
    let requires_thinking = external_model.to_lowercase().contains("thinking");

    // GPT-5.6 系列降级到最便宜的 luna 变体（避免跨协议降级到 Claude）
    if mapped_model.starts_with("gpt-5.6-") {
        if available_models.contains(&"gpt-5.6-luna".to_string()) {
            log::warn!(
                "[Gateway] 模型 {} 不在可用列表中，降级到 gpt-5.6-luna",
                mapped_model
            );
            return Ok("gpt-5.6-luna".to_string());
        }
        // 如果连 luna 都没有，再降级到 Claude
    }

    // 简单粗暴：一律降级到 claude-sonnet-4.5（Free 用户最高可用模型）
    let fallback = if requires_thinking {
        "claude-sonnet-4.5-thinking"
    } else {
        "claude-sonnet-4.5"
    };

    log::warn!(
        "[Gateway] 模型 {} 不在可用列表中，降级到 {}",
        mapped_model,
        fallback
    );

    Ok(fallback.to_string())
}

pub fn normalize_external_model_alias(external_model: &str) -> String {
    external_model.trim().to_ascii_lowercase()
}

pub fn get_available_models() -> Vec<ModelInfo> {
    // 数据来源：Kiro ListAvailableModels API 实际返回
    // 注意：Claude 模型只保留 -thinking 版本，不带后缀的已删除
    //       GPT-5.6 系列是 Kiro 原生 GPT 模型，没有 thinking 变体
    [
        // 自动选择
        ("auto", "anthropic"),
        // Claude 系列（仅 thinking 版本）
        ("claude-sonnet-5-thinking", "anthropic"),
        ("claude-opus-5.5-thinking", "anthropic"),
        ("claude-opus-4.8-thinking", "anthropic"),
        ("claude-opus-4.7-thinking", "anthropic"),
        ("claude-opus-4.6-thinking", "anthropic"),
        ("claude-sonnet-4.6-thinking", "anthropic"),
        ("claude-opus-4.5-thinking", "anthropic"),
        ("claude-sonnet-4.5-thinking", "anthropic"),
        ("claude-haiku-4.5-thinking", "anthropic"),
        ("claude-sonnet-4-thinking", "anthropic"),
        // GPT-5.6 系列（Kiro 原生 GPT 模型）
        ("gpt-5.6-sol", "openai"),
        ("gpt-5.6-terra", "openai"),
        ("gpt-5.6-luna", "openai"),
        // 开源模型
        ("deepseek-3.2", "deepseek"),
        ("minimax-m2.5", "minimax"),
        ("minimax-m2.1", "minimax"),
        ("glm-5", "zhipu"),
        ("qwen3-coder-next", "alibaba"),
    ]
    .into_iter()
    .map(|(id, owner)| ModelInfo {
        id: id.to_string(),
        object: "model".to_string(),
        created: 1_700_000_000,
        owned_by: owner.to_string(),
    })
    .collect()
}
