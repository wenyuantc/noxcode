use serde_json::Value;

use crate::engine::UsageDelta;

use super::types::Usage;

pub fn parse_usage(value: &Value) -> Usage {
    let prompt = first_u32(
        value,
        &[
            "prompt_tokens",
            "input_tokens",
            "promptTokens",
            "inputTokens",
        ],
    );
    let completion = first_u32(
        value,
        &[
            "completion_tokens",
            "output_tokens",
            "completionTokens",
            "outputTokens",
        ],
    );
    let mut cached = first_u32(
        value,
        &[
            "cached_tokens",
            "prompt_cache_hit_tokens",
            "cache_read_input_tokens",
            "cache_read_tokens",
        ],
    );
    if cached == 0 {
        if let Some(details) = value
            .get("prompt_tokens_details")
            .or_else(|| value.get("input_tokens_details"))
        {
            cached = first_u32(details, &["cached_tokens", "cachedTokens"]);
        }
    }
    // Anthropic 的 `input_tokens` 不含缓存读写部分，窗口实际占用是三者之和。
    let anthropic_cache = value.get("cache_read_input_tokens").is_some()
        || value.get("cache_creation_input_tokens").is_some();
    let prompt = if anthropic_cache && value.get("input_tokens").is_some() {
        prompt
            .saturating_add(first_u32(value, &["cache_read_input_tokens"]))
            .saturating_add(first_u32(value, &["cache_creation_input_tokens"]))
    } else {
        prompt
    };
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        cached_tokens: if prompt > 0 {
            cached.min(prompt)
        } else {
            cached
        },
    }
}

pub fn usage_to_delta(usage: Usage) -> Option<UsageDelta> {
    if usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
        return None;
    }
    let input = u64::from(usage.prompt_tokens);
    let output = u64::from(usage.completion_tokens);
    Some(UsageDelta {
        input_tokens: Some(input),
        output_tokens: Some(output),
        total_tokens: Some(input + output),
        reasoning_tokens: None,
        cached_tokens: Some(u64::from(usage.cached_tokens)),
    })
}

fn first_u32(value: &Value, keys: &[&str]) -> u32 {
    for key in keys {
        if let Some(number) = value.get(*key).and_then(json_u32) {
            return number;
        }
    }
    0
}

fn json_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .map(|item| item as u32)
        .or_else(|| value.as_i64().and_then(|item| u32::try_from(item).ok()))
}

#[cfg(test)]
mod tests {
    use super::super::types::Usage;
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_openai_and_anthropic_shapes() {
        let openai = parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 4,
            "prompt_tokens_details": {"cached_tokens": 3}
        }));
        assert_eq!(openai.prompt_tokens, 10);
        assert_eq!(openai.completion_tokens, 4);
        assert_eq!(openai.cached_tokens, 3);

        let anthropic = parse_usage(&json!({
            "input_tokens": 8,
            "output_tokens": 2,
            "cache_read_input_tokens": 1
        }));
        assert_eq!(anthropic.prompt_tokens, 9);
        assert_eq!(anthropic.completion_tokens, 2);
        assert_eq!(anthropic.cached_tokens, 1);
    }

    #[test]
    fn anthropic_prompt_includes_cache_read_and_creation() {
        let usage = parse_usage(&json!({
            "input_tokens": 12,
            "cache_read_input_tokens": 90_000,
            "cache_creation_input_tokens": 3_000,
            "output_tokens": 5
        }));
        assert_eq!(usage.prompt_tokens, 93_012);
        assert_eq!(usage.cached_tokens, 90_000);

        // 只有输出的 message_delta 不应被当成新的输入量。
        let delta = parse_usage(&json!({"output_tokens": 40}));
        assert_eq!(delta.prompt_tokens, 0);

        // OpenAI Responses 的 input_tokens 已含缓存，不能重复相加。
        let responses = parse_usage(&json!({
            "input_tokens": 100,
            "input_tokens_details": {"cached_tokens": 60}
        }));
        assert_eq!(responses.prompt_tokens, 100);
        assert_eq!(responses.cached_tokens, 60);
    }

    #[test]
    fn usage_to_delta_skips_empty_and_keeps_zero_cache() {
        assert!(usage_to_delta(Usage::default()).is_none());

        let delta = usage_to_delta(Usage {
            prompt_tokens: 10,
            completion_tokens: 4,
            cached_tokens: 0,
        })
        .expect("delta");
        assert_eq!(delta.input_tokens, Some(10));
        assert_eq!(delta.output_tokens, Some(4));
        assert_eq!(delta.total_tokens, Some(14));
        assert_eq!(delta.cached_tokens, Some(0));
    }
}
