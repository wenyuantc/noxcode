//! Transport completion is a precondition for committing a model response.
use super::{
    anthropic::parse_anthropic_sse, openai::parse_openai_sse, responses::parse_responses_sse,
};

#[test]
fn openai_partial_stream_without_terminal_is_rejected() {
    let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"unfinished\"}}]}\n\n";
    assert!(parse_openai_sse(stream).is_err());
}

#[test]
fn anthropic_partial_stream_without_message_stop_is_rejected() {
    let stream = concat!(
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"unfinished\"}}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
    );
    assert!(parse_anthropic_sse(stream).is_err());
}

#[test]
fn responses_partial_stream_without_response_terminal_is_rejected() {
    let stream = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"unfinished\"}\n\n";
    assert!(parse_responses_sse(stream).is_err());
}

#[test]
fn provider_error_after_partial_openai_text_does_not_commit_success() {
    let stream = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"unfinished\"}}]}\n\n",
        "data: {\"error\":{\"message\":\"upstream failed\",\"type\":\"server_error\"}}\n\n",
        "data: [DONE]\n\n",
    );
    assert!(parse_openai_sse(stream).is_err());
}

#[test]
fn complete_openai_stream_with_invalid_tool_arguments_is_rejected() {
    let stream = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"Write\",\"arguments\":\"{\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    assert!(parse_openai_sse(stream).is_err());
}

#[test]
fn complete_legacy_openai_stream_without_reason_is_accepted() {
    let stream = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"complete answer\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    assert!(parse_openai_sse(stream).is_ok());
}

use super::response::{FinishReason, ModelErrorKind, ModelResponse};
use super::{
    anthropic::parse_anthropic_json, openai::parse_openai_json, responses::parse_responses_json,
};
use serde_json::{json, Value};

fn event(kind: &str, payload: Value) -> String {
    format!("event: {kind}\ndata: {payload}\n\n")
}

#[test]
fn all_protocol_json_limits_preserve_text_and_drop_entire_tool_batch() {
    let responses = [
        parse_openai_json(
            &json!({"id":"o1","choices":[{"finish_reason":"length","message":{"content":"partial","reasoning_content":"think","tool_calls":[{"id":"x","function":{"name":"Write","arguments":"{"}}]}}]}),
        ),
        parse_anthropic_json(
            &json!({"id":"a1","stop_reason":"max_tokens","content":[{"type":"text","text":"partial"},{"type":"thinking","thinking":"think"},{"type":"tool_use","id":"x","name":"Write"}]}),
        ),
        parse_responses_json(
            &json!({"id":"r1","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"message","content":[{"type":"output_text","text":"partial"}]},{"type":"reasoning","summary":[{"text":"think"}]},{"type":"function_call","call_id":"x","name":"Write","arguments":"{"}]}),
        ),
    ];
    for response in responses {
        let response = response.unwrap();
        assert_eq!(response.finish_reason, FinishReason::OutputLimit);
        assert_eq!(response.message.content, "partial");
        assert_eq!(response.message.reasoning_content, "think");
        assert!(response.message.tool_calls.is_empty());
        assert!(response.raw_finish_reason.is_some());
        assert!(
            response.complete_message().is_err(),
            "auxiliary calls must reject partial output"
        );
    }
}

#[test]
fn all_protocol_stream_limits_accept_empty_output_and_preserve_raw_reasons() {
    let outputs = [
        parse_openai_sse(&event(
            "",
            json!({"choices":[{"delta":{},"finish_reason":"length"}]}),
        )),
        parse_anthropic_sse(
            &(event(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
            ) + &event("message_stop", json!({"type":"message_stop"}))),
        ),
        parse_responses_sse(&event(
            "response.incomplete",
            json!({"type":"response.incomplete","response":{"id":"r1","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}),
        )),
    ];
    for (response, raw) in outputs
        .into_iter()
        .zip(["length", "max_tokens", "max_output_tokens"])
    {
        let response = response.unwrap();
        assert_eq!(response.finish_reason, FinishReason::OutputLimit);
        assert_eq!(response.raw_finish_reason.as_deref(), Some(raw));
        assert!(response.message.content.is_empty());
    }
}

#[test]
fn all_protocol_json_missing_reason_is_compatible_but_bad_tool_batch_is_not() {
    let outputs = [
        parse_openai_json(&json!({"choices":[{"message":{"content":"ok"}}]})),
        parse_anthropic_json(&json!({"content":[{"type":"text","text":"ok"}]})),
        parse_responses_json(&json!({"output_text":"ok"})),
    ];
    for response in outputs {
        assert_eq!(response.unwrap().finish_reason, FinishReason::Unknown);
    }
    let invalid = [
        parse_openai_json(
            &json!({"choices":[{"message":{"content":"ok","tool_calls":[{"id":"a","function":{"name":"Read","arguments":"{}"}},{"id":"b","function":{"name":"Write","arguments":"{"}}]}}]}),
        ),
        parse_anthropic_json(
            &json!({"content":[{"type":"tool_use","id":"a","name":"Read","input":{}},{"type":"tool_use","id":"b","name":"Write","input":[]}]}),
        ),
        parse_responses_json(
            &json!({"output":[{"type":"function_call","call_id":"a","name":"Read","arguments":"{}"},{"type":"function_call","call_id":"b","name":"Write","arguments":"{"}]}),
        ),
    ];
    for response in invalid {
        assert_eq!(response.unwrap_err().kind, ModelErrorKind::InvalidResponse);
    }
}

#[test]
fn all_protocol_refusals_and_context_limits_are_classified() {
    for (raw, expected) in [
        ("refusal", FinishReason::Refusal),
        ("model_context_window_exceeded", FinishReason::ContextLimit),
    ] {
        let outputs = [
            parse_openai_json(&json!({"choices":[{"finish_reason":raw,"message":{"content":""}}]})),
            parse_anthropic_json(&json!({"stop_reason":raw,"content":[]})),
            parse_responses_json(
                &json!({"status":"incomplete","incomplete_details":{"reason":raw},"output":[]}),
            ),
        ];
        for response in outputs {
            assert_eq!(response.unwrap().finish_reason, expected);
        }
    }
}

#[test]
fn anthropic_and_responses_errors_after_text_are_structured_failures() {
    let anthropic = event(
        "content_block_delta",
        json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"partial"}}),
    ) + &event(
        "error",
        json!({"type":"error","error":{"type":"overloaded_error","message":"failed"}}),
    ) + &event("message_stop", json!({"type":"message_stop"}));
    let responses = event(
        "response.output_text.delta",
        json!({"type":"response.output_text.delta","delta":"partial"}),
    ) + &event(
        "response.failed",
        json!({"type":"response.failed","response":{"error":{"code":"server_error","message":"failed"}}}),
    );
    assert_eq!(
        parse_anthropic_sse(&anthropic).unwrap_err().kind,
        ModelErrorKind::Provider
    );
    assert_eq!(
        parse_responses_sse(&responses).unwrap_err().kind,
        ModelErrorKind::Provider
    );
}

#[test]
fn responses_refusal_delta_survives_terminal_without_reason() {
    let sse = event(
        "response.refusal.delta",
        json!({"type":"response.refusal.delta","delta":"cannot"}),
    ) + &event(
        "response.completed",
        json!({"type":"response.completed","response":{"id":"r1"}}),
    );
    assert_eq!(
        parse_responses_sse(&sse).unwrap().finish_reason,
        FinishReason::Refusal
    );
}

#[test]
fn responses_orphan_tool_fragment_is_never_silently_dropped() {
    let sse = event(
        "response.output_text.delta",
        json!({"type":"response.output_text.delta","delta":"ok"}),
    ) + &event(
        "response.function_call_arguments.delta",
        json!({"type":"response.function_call_arguments.delta","call_id":"missing","delta":"{}"}),
    ) + &event(
        "response.completed",
        json!({"type":"response.completed","response":{}}),
    );
    assert_eq!(
        parse_responses_sse(&sse).unwrap_err().kind,
        ModelErrorKind::InvalidResponse
    );
}

#[test]
fn responses_parallel_tool_items_use_item_identity_and_done_does_not_duplicate() {
    let mut sse = String::new();
    for (id, call) in [("item1", "call1"), ("item2", "call2")] {
        sse += &event(
            "response.output_item.added",
            json!({"type":"response.output_item.added","item":{"type":"function_call","id":id,"call_id":call,"name":"Read","arguments":""}}),
        );
    }
    for (id, call, argument) in [
        ("item2", "call2", r#"{"path":"b"}"#),
        ("item1", "call1", r#"{"path":"a"}"#),
    ] {
        sse += &event(
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","item_id":id,"delta":argument}),
        );
        sse += &event(
            "response.output_item.done",
            json!({"type":"response.output_item.done","item":{"type":"function_call","id":id,"call_id":call,"name":"Read","arguments":argument}}),
        );
    }
    sse += &event(
        "response.completed",
        json!({"type":"response.completed","response":{"status":"completed"}}),
    );
    let ModelResponse { message, .. } = parse_responses_sse(&sse).unwrap();
    assert_eq!(message.tool_calls.len(), 2);
    assert_eq!(message.tool_calls[0].arguments, r#"{"path":"a"}"#);
    assert_eq!(message.tool_calls[1].arguments, r#"{"path":"b"}"#);
}

#[test]
fn responses_completed_tool_output_normalizes_without_losing_raw_status() {
    let response = parse_responses_json(&json!({"status":"completed","output":[{"type":"function_call","call_id":"a","name":"Read","arguments":"{}"}]})).unwrap();
    assert_eq!(response.finish_reason, FinishReason::ToolCalls);
    assert_eq!(response.raw_finish_reason.as_deref(), Some("completed"));
}

#[test]
fn refusal_content_keeps_the_providers_raw_finish_reason() {
    let openai = parse_openai_json(
        &json!({"choices":[{"finish_reason":"stop","message":{"refusal":"no"}}]}),
    )
    .unwrap();
    assert_eq!(openai.finish_reason, FinishReason::Refusal);
    assert_eq!(openai.raw_finish_reason.as_deref(), Some("stop"));
    let response = parse_responses_json(&json!({"status":"completed","output":[{"type":"message","content":[{"type":"refusal","refusal":"no"}]}]})).unwrap();
    assert_eq!(response.finish_reason, FinishReason::Refusal);
    assert_eq!(response.raw_finish_reason.as_deref(), Some("completed"));
}

#[test]
fn responses_explicit_incomplete_tool_is_not_executable_with_unknown_reason() {
    let response = parse_responses_json(
        &json!({"output":[{"type":"function_call","status":"incomplete","call_id":"a","name":"Read","arguments":"{}"}]}),
    );
    assert_eq!(response.unwrap_err().kind, ModelErrorKind::InvalidResponse);
}

#[test]
fn unclassified_incomplete_responses_cannot_execute_tools() {
    let response = json!({"status":"incomplete","incomplete_details":{"reason":"future_unknown_limit"},"output":[{"type":"function_call","call_id":"a","name":"Read","arguments":"{}"}]});
    assert!(parse_responses_json(&response).is_err());
    assert!(parse_responses_sse(&event(
        "response.incomplete",
        json!({"type":"response.incomplete","response":response})
    ))
    .is_err());
}

#[test]
fn all_protocol_stream_missing_reasons_remain_compatible_with_valid_terminals() {
    let outputs = [
        parse_openai_sse(
            &(event("", json!({"choices":[{"delta":{"content":"ok"}}]})) + "data: [DONE]\n\n"),
        ),
        parse_anthropic_sse(
            &(event(
                "content_block_start",
                json!({"type":"content_block_start","content_block":{"type":"text","text":"ok"}}),
            ) + &event("message_stop", json!({"type":"message_stop"}))),
        ),
        parse_responses_sse(&event(
            "response.completed",
            json!({"type":"response.completed","response":{"output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}]}}),
        )),
    ];
    for response in outputs {
        let response = response.unwrap();
        assert_eq!(response.finish_reason, FinishReason::Unknown);
        assert_eq!(response.message.content, "ok");
    }
}

#[test]
fn all_protocol_stream_output_limits_discard_truncated_tool_arguments() {
    let openai = event(
        "",
        json!({"choices":[{"delta":{"content":"partial","tool_calls":[{"index":0,"id":"x","function":{"name":"Write","arguments":"{"}}]}}]}),
    ) + &event(
        "",
        json!({"choices":[{"delta":{},"finish_reason":"length"}]}),
    );
    let anthropic = event(
        "content_block_start",
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"partial"}}),
    ) + &event(
        "content_block_start",
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"x","name":"Write","input":{}}}),
    ) + &event(
        "content_block_delta",
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{"}}),
    ) + &event(
        "message_delta",
        json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
    ) + &event("message_stop", json!({"type":"message_stop"}));
    let responses = event(
        "response.output_text.delta",
        json!({"type":"response.output_text.delta","delta":"partial"}),
    ) + &event(
        "response.output_item.added",
        json!({"type":"response.output_item.added","item":{"type":"function_call","id":"item","call_id":"x","name":"Write","arguments":""}}),
    ) + &event(
        "response.function_call_arguments.delta",
        json!({"type":"response.function_call_arguments.delta","item_id":"item","delta":"{"}),
    ) + &event(
        "response.incomplete",
        json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}),
    );
    for response in [
        parse_openai_sse(&openai),
        parse_anthropic_sse(&anthropic),
        parse_responses_sse(&responses),
    ] {
        let response = response.unwrap();
        assert_eq!(response.finish_reason, FinishReason::OutputLimit);
        assert_eq!(response.message.content, "partial");
        assert!(response.message.tool_calls.is_empty());
    }
}

#[test]
fn explicit_unknown_responses_item_identity_never_uses_single_tool_fallback() {
    for identity in [
        json!({"item_id":"missing"}),
        json!({"call_id":"a","item_id":"missing"}),
    ] {
        let mut delta = json!({"type":"response.function_call_arguments.delta","delta":"{}"});
        delta
            .as_object_mut()
            .unwrap()
            .extend(identity.as_object().unwrap().clone());
        let sse = event(
            "response.output_item.added",
            json!({"type":"response.output_item.added","item":{"type":"function_call","id":"item-a","call_id":"a","name":"Read","arguments":""}}),
        ) + &event("response.function_call_arguments.delta", delta)
            + &event(
                "response.completed",
                json!({"type":"response.completed","response":{}}),
            );
        assert!(parse_responses_sse(&sse).is_err());
    }
}

#[test]
fn every_responses_terminal_rejects_unknown_incomplete_status() {
    for terminal in ["response.completed", "response.done", "response.incomplete"] {
        let sse = event(
            terminal,
            json!({"type":terminal,"response":{"status":"incomplete","incomplete_details":{"reason":"unknown_limit"},"output":[{"type":"function_call","call_id":"a","name":"Read","arguments":"{}"}]}}),
        );
        assert!(parse_responses_sse(&sse).is_err(), "{terminal}");
    }
}
