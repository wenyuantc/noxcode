//! 工具输出契约校验：只支持分类需要的 JSON Schema 子集
//! （`type`、`required`、`properties`、`items`、`enum`），不引入新依赖。
//! 不认识的关键字忽略，不当作通过依据之外的约束。

use serde_json::Value;

use super::contract::{OutputContract, ToolContract, ViolationPolicy};
use super::dispatch::ToolOutput;

const MAX_DEPTH: usize = 32;

/// 按契约检查结果：符合或文本契约原样返回；不符合时按策略失败或降级。
pub fn check_output(contract: &ToolContract, mut output: ToolOutput) -> Result<ToolOutput, String> {
    let OutputContract::Json(schema) = &contract.output else {
        return Ok(output);
    };
    let value = match output.structured.clone() {
        Some(value) => Ok(value),
        None => serde_json::from_str::<Value>(output.text.trim())
            .map_err(|_| "结果不是 JSON".to_string()),
    };
    let Err(reason) = value.and_then(|value| validate(schema, &value)) else {
        return Ok(output);
    };
    match contract.on_violation {
        ViolationPolicy::Fail => Err(format!("工具 {} 的输出不符合契约：{reason}", contract.name)),
        ViolationPolicy::Degrade => {
            output.text = format!(
                "[输出未通过契约校验，按不可信文本处理：{reason}]\n{}",
                output.text
            );
            Ok(output)
        }
    }
}

/// 校验失败返回 `路径：原因`。
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    validate_at(schema, value, "$", 0)
}

fn validate_at(schema: &Value, value: &Value, path: &str, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{path}：结构嵌套过深"));
    }
    let Some(schema) = schema.as_object() else {
        return Ok(());
    };
    if let Some(expected) = schema.get("type") {
        let allowed: Vec<&str> = match expected {
            Value::String(kind) => vec![kind.as_str()],
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !allowed.is_empty() && !allowed.iter().any(|kind| type_matches(kind, value)) {
            return Err(format!(
                "{path}：应为 {}，实际为 {}",
                allowed.join(" / "),
                type_name(value)
            ));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(value) {
            return Err(format!("{path}：不在允许的取值中"));
        }
    }
    if let Some(object) = value.as_object() {
        for key in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !object.contains_key(key) {
                return Err(format!("{path}.{key}：缺少必填字段"));
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (key, child) in properties {
                if let Some(item) = object.get(key) {
                    validate_at(child, item, &format!("{path}.{key}"), depth + 1)?;
                }
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            validate_at(items, item, &format!("{path}[{index}]"), depth + 1)?;
        }
    }
    Ok(())
}

fn type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        _ => true,
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::tools::contract::{ToolContract, ViolationPolicy};
    use serde_json::json;

    #[test]
    fn schema_subset_reports_the_failing_path() {
        let schema = json!({
            "type": "object",
            "required": ["id", "items"],
            "properties": {
                "id": {"type": "string"},
                "state": {"enum": ["open", "closed"]},
                "items": {"type": "array", "items": {"type": ["integer", "null"]}}
            }
        });
        assert!(validate(&schema, &json!({"id": "a", "items": [1, null]})).is_ok());
        for (value, path) in [
            (json!({"items": []}), "$.id"),
            (json!({"id": 1, "items": []}), "$.id"),
            (json!({"id": "a", "items": [1, "x"]}), "$.items[1]"),
            (json!({"id": "a", "items": [], "state": "gone"}), "$.state"),
            (json!([]), "$"),
        ] {
            let error = validate(&schema, &value).expect_err("invalid");
            assert!(error.starts_with(path), "{error}");
        }
        let mut deep = json!(1);
        for _ in 0..40 {
            deep = json!([deep]);
        }
        let mut nested = json!({});
        for _ in 0..40 {
            nested = json!({"items": nested});
        }
        assert!(validate(&nested, &deep).is_err());
    }

    #[test]
    fn violations_fail_or_degrade_by_policy() {
        let mut contract = ToolContract::for_mcp("mcp_x", false, false);
        contract.output = OutputContract::Json(json!({"type": "object", "required": ["ok"]}));
        let degraded = check_output(&contract, ToolOutput::text("not json")).expect("degrade");
        assert!(degraded.text.starts_with("[输出未通过契约校验"));
        assert!(degraded.text.ends_with("not json"));
        let mut structured = ToolOutput::text("文字说明");
        structured.structured = Some(json!({"ok": true}));
        assert_eq!(
            check_output(&contract, structured.clone()).unwrap(),
            structured
        );

        contract.on_violation = ViolationPolicy::Fail;
        let error = check_output(&contract, ToolOutput::text("{}")).unwrap_err();
        assert!(error.contains("$.ok"), "{error}");
        // 文本契约不做结构校验。
        let text = crate::native::tools::contract::builtin_contract("Read").unwrap();
        assert!(check_output(text, ToolOutput::text("anything")).is_ok());
    }

    #[test]
    fn cron_update_declares_a_json_output_contract() {
        let contract = crate::native::tools::contract::builtin_contract("CronUpdate").unwrap();
        let valid = json!({"id": "a", "name": "n", "prompt": "p", "cron": "@daily",
            "enabled": 1, "channel_id": null, "model": null, "next_run_at": "2026-01-01"});
        assert!(check_output(contract, ToolOutput::text(valid.to_string())).is_ok());
        assert!(check_output(contract, ToolOutput::text(r#"{"id":"a"}"#)).is_err());
    }
}
