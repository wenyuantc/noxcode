use super::super::subagent::parse_subagent_args;
use super::*;
use crate::native::model::types::Message;
use std::fs;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_runner() -> (AgentRunner, std::path::PathBuf) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("codex-ai-agent-{stamp}-{seq}"));
    fs::create_dir_all(&root).expect("mkdir");
    fs::write(root.join("hello.txt"), "hello world\n").expect("write");
    let runner = AgentRunner::new(LocalWorkspace::new(root.clone()));
    runner
        .ctx
        .allow_all_high_risk
        .store(true, std::sync::atomic::Ordering::SeqCst);
    (runner, root)
}

fn readonly_custom_subagent() -> NativeSubagent {
    NativeSubagent {
        id: "1".to_string(),
        name: "reviewer".to_string(),
        description: "review".to_string(),
        model_mode: "inherit".to_string(),
        channel_id: None,
        model: None,
        tool_mode: "custom".to_string(),
        tools: vec!["Read".to_string(), "Grep".to_string()],
        system_prompt: "你是审查员".to_string(),
        inject_agents_md: false,
        scope: "all".to_string(),
        workspace_ids: Vec::new(),
        permission_mode: None,
        disallowed_tools: Vec::new(),
        source: "json".to_string(),
        path: None,
        max_turns: None,
        skills: Vec::new(),
        reasoning_effort: None,
    }
}

fn drain_events(rx: &mut mpsc::UnboundedReceiver<NativeEvent>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Ok(event) = rx.try_recv() {
        match event {
            NativeEvent::Line(line) | NativeEvent::Tool { line, .. } => lines.push(line),
            NativeEvent::Assistant { text, fragment } => {
                lines.push(
                    fragment
                        .subagent_tag
                        .map(|tag| format!("{tag} {text}"))
                        .unwrap_or(text),
                );
            }
            NativeEvent::UserInput { text, .. } => {
                lines.push(format!("[USER_INPUT] {text}"));
            }
            _ => {}
        }
    }
    lines
}

fn capture_checkpoints(runner: &mut AgentRunner) -> Arc<tokio::sync::Mutex<Vec<Vec<Message>>>> {
    let snapshots = Arc::new(tokio::sync::Mutex::new(Vec::<Vec<Message>>::new()));
    let captured = snapshots.clone();
    runner.on_checkpoint = Some(Arc::new(move |messages| {
        let captured = captured.clone();
        Box::pin(async move {
            captured.lock().await.push(messages.clone());
            Ok(messages)
        })
    }));
    snapshots
}

#[tokio::test]
async fn checkpoint_failure_stops_before_the_next_model_call() {
    let (mut runner, root) = temp_runner();
    runner.on_checkpoint = Some(Arc::new(|_messages| {
        Box::pin(async { Err("保存会话历史失败: disk".to_string()) })
    }));
    let error = runner
        .run_scripted("hi", vec![Message::assistant_text("should not run")])
        .await
        .unwrap_err();
    assert!(error.contains("保存会话历史失败"));
    assert_eq!(runner.turns, 0);
    fs::remove_dir_all(root).unwrap();
}

async fn mock_child_model(
    registry: Arc<BackgroundTaskRegistry>,
    task_id: String,
    responses: Vec<(Value, Option<String>)>,
) -> (ModelClient, tokio::task::JoinHandle<Vec<Value>>) {
    mock_model_before_response(registry, task_id, responses, None).await
}

type BeforeModelResponse =
    Arc<dyn Fn(usize) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

async fn mock_model_before_response(
    registry: Arc<BackgroundTaskRegistry>,
    task_id: String,
    responses: Vec<(Value, Option<String>)>,
    before: Option<BeforeModelResponse>,
) -> (ModelClient, tokio::task::JoinHandle<Vec<Value>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind model");
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut captured = Vec::new();
        for (index, (response, steer)) in responses.into_iter().enumerate() {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0, "incomplete model request");
                bytes.extend_from_slice(&buffer[..count]);
                let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
                    continue;
                };
                let body_start = header_end + 4;
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, length)| length.trim().parse::<usize>().ok())
                    .expect("content length");
                if bytes.len() < body_start + content_length {
                    continue;
                }
                captured.push(
                    serde_json::from_slice::<Value>(
                        &bytes[body_start..body_start + content_length],
                    )
                    .unwrap(),
                );
                break;
            }
            if let Some(before) = &before {
                before(index).await;
            }
            if let Some(steer) = steer {
                registry.send_message(&task_id, &steer).await.unwrap();
            }
            let body = response.to_string();
            let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(body.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        }
        captured
    });
    let client = ModelClient::new(crate::native::model::client::ModelClientConfig {
        protocol: crate::native::protocol::PROTOCOL_OPENAI.to_string(),
        base_url: format!("http://{address}"),
        api_key: "test".to_string(),
        extra_headers: HashMap::new(),
        retry: crate::native::model::RetryConfig::none(),
        timeout: Duration::from_secs(5),
        network: crate::app::network_settings::NetworkSettings::default(),
        responses_continuation: crate::native::model::ResponsesContinuationMode::Auto,
    })
    .unwrap();
    (client, server)
}

fn assistant_tool_calls(calls: &[(&str, &str, &str)]) -> Message {
    Message {
        role: Role::Assistant,
        content: String::new(),
        tool_calls: calls
            .iter()
            .map(|(id, name, arguments)| ToolCall {
                id: (*id).to_string(),
                name: (*name).to_string(),
                arguments: (*arguments).to_string(),
            })
            .collect(),
        tool_call_id: String::new(),
        name: String::new(),
        reasoning_content: String::new(),
        images: Vec::new(),
        media: Vec::new(),
        history_id: String::new(),
    }
}

#[tokio::test]
async fn parallel_read_only_batch_keeps_result_order_and_serializes_writes() {
    let (mut runner, root) = temp_runner();
    fs::write(root.join("second.txt"), "second file\n").expect("write");
    let (tx, mut rx) = mpsc::unbounded_channel();
    runner.on_event = Some(tx);
    let text = runner
        .run_scripted(
            "并行读取",
            vec![
                assistant_tool_calls(&[
                    ("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                    ("c2", "Glob", r#"{"pattern":"*.txt"}"#),
                    ("c3", "Read", r#"{"file_path":"second.txt"}"#),
                    (
                        "c4",
                        "Write",
                        r#"{"file_path":"hello.txt","content":"rewritten"}"#,
                    ),
                    ("c5", "Read", r#"{"file_path":"hello.txt"}"#),
                ]),
                Message::assistant_text("done"),
            ],
        )
        .await
        .expect("run");
    assert_eq!(text, "done");
    let tool_messages: Vec<&Message> = runner
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .collect();
    assert_eq!(
        tool_messages
            .iter()
            .map(|message| message.tool_call_id.as_str())
            .collect::<Vec<_>>(),
        vec!["c1", "c2", "c3", "c4", "c5"]
    );
    assert!(tool_messages[0].content.contains("hello world"));
    assert!(tool_messages[1].content.contains("second.txt"));
    assert!(tool_messages[2].content.contains("second file"));
    assert!(tool_messages[3].content.contains("Wrote"));
    // 写入排在并行批之后串行执行，之后的 Read 必须看到新内容。
    assert!(tool_messages[4].content.contains("rewritten"));
    let lines = drain_events(&mut rx);
    let starts: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("[读取]") || line.starts_with("[工具] Glob"))
        .collect();
    assert_eq!(starts.len(), 4);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn committed_tool_results_are_reused_and_unknown_writes_are_not_replayed() {
    use sqlx::Row;

    let (mut runner, root) = temp_runner();
    let pool = crate::db::test_support::setup_migrated_pool().await;
    let recovery = Arc::new(RecoveryState::new(pool.clone(), "sess-live", 7));
    recovery.bind_turn("turn-live");
    runner.recovery = Some(recovery);
    runner
        .consume_assistant(
            assistant_tool_calls(&[
                ("read-1", "Read", r#"{"file_path":"hello.txt"}"#),
                (
                    "write-1",
                    "Write",
                    r#"{"file_path":"out.txt","content":"v1"}"#,
                ),
            ]),
            false,
            None,
        )
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(root.join("out.txt")).unwrap(), "v1");
    let rows = sqlx::query(
            "SELECT call_id, status FROM native_tool_runs WHERE session_record_id = 'sess-live' ORDER BY call_index",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
    let stored: Vec<(String, String)> = rows
        .iter()
        .map(|row| (row.get("call_id"), row.get("status")))
        .collect();
    assert_eq!(
        stored,
        vec![
            ("read-1".to_string(), "committed".to_string()),
            ("write-1".to_string(), "committed".to_string()),
        ]
    );

    let recovery = Arc::new(RecoveryState::new(pool.clone(), "sess-resume", 7));
    recovery.bind_turn("turn-resume");
    recovery
        .commit_plan(&[
            PlannedCall {
                call_id: "read-old".into(),
                name: "Read".into(),
                arguments: r#"{"file_path":"hello.txt"}"#.into(),
                side_effect: false,
            },
            PlannedCall {
                call_id: "write-old".into(),
                name: "Write".into(),
                arguments: r#"{"file_path":"nope.txt","content":"replayed"}"#.into(),
                side_effect: true,
            },
        ])
        .await
        .unwrap();
    recovery.mark_started("read-old").await.unwrap();
    recovery.mark_started("write-old").await.unwrap();
    recovery
        .commit_result("read-old", "已提交的读取结果", false)
        .await
        .unwrap();

    let (mut resumed, resumed_root) = temp_runner();
    resumed.recovery = Some(Arc::new(RecoveryState::new(pool, "sess-resume", 7)));
    resumed.apply_tool_recovery().await.unwrap();
    resumed
        .begin_user_turn("continue", Vec::new())
        .await
        .unwrap();
    assert!(!resumed_root.join("nope.txt").exists());
    assert!(resumed.messages.iter().any(|message| {
        message.tool_call_id == "read-old" && message.content.contains("已提交的读取结果")
    }));
    assert!(resumed.messages.iter().any(|message| {
        message.tool_call_id == "write-old" && message.content.contains(UNKNOWN_RESULT)
    }));
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(resumed_root);
}

#[tokio::test]
async fn manual_compaction_writes_boundary_and_summarizes_history() {
    let (mut runner, root) = temp_runner();
    let (tx, mut rx) = mpsc::unbounded_channel();
    runner.on_event = Some(tx);
    // 先跑两个回合积累历史，再请求手动压缩。
    runner
        .run_scripted("第一件事", vec![Message::assistant_text("做完第一件")])
        .await
        .expect("first");
    runner
        .run_scripted("第二件事", vec![Message::assistant_text("做完第二件")])
        .await
        .expect("second");
    runner.request_manual_compaction(Some("保留失败堆栈".to_string()));
    let text = runner
        .run_scripted("第三件事", vec![Message::assistant_text("继续")])
        .await
        .expect("third");
    assert_eq!(text, "继续");
    let lines = drain_events(&mut rx);
    let boundary_line = lines
        .iter()
        .find(|line| line.starts_with("[COMPACT_BOUNDARY]"))
        .expect("boundary line");
    let boundary = CompactBoundary::parse_line(boundary_line).expect("parse");
    assert_eq!(boundary.trigger, CompactTrigger::Manual);
    assert_eq!(boundary.source, "local");
    assert_eq!(boundary.instructions.as_deref(), Some("保留失败堆栈"));
    assert!(boundary.post_messages <= boundary.pre_messages);
    assert_eq!(runner.context_window.compactions, 1);
    assert!(runner
        .messages
        .iter()
        .any(|message| message.content.contains("第三件事")));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn background_agent_does_not_emit_duplicate_start() {
    let (mut runner, root) = temp_runner();
    runner.subagent_stub = Some(Arc::new(|spec| format!("stub:{}", spec.description)));
    let (tx, mut rx) = mpsc::unbounded_channel();
    runner.on_event = Some(tx);
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call(
                    "c1",
                    "Agent",
                    r#"{"description":"bg","prompt":"do it","run_in_background":true}"#,
                ),
                Message::assistant_text("done"),
            ],
        )
        .await
        .expect("run");
    let lines = drain_events(&mut rx);
    let tag = "[子 Agent 1(general) - bg]";
    let starts = lines
        .iter()
        .filter(|line| line.contains("[子 Agent] bg"))
        .count();
    assert_eq!(starts, 1, "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with(&format!("{tag} [子 Agent] bg"))),
        "{lines:?}"
    );
    let results = lines
        .iter()
        .filter(|line| line.contains("[工具结果]") && line.starts_with(tag))
        .count();
    assert_eq!(results, 1, "{lines:?}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn background_agent_returns_task_id_and_task_output_waits() {
    let (mut runner, root) = temp_runner();
    runner.subagent_stub = Some(Arc::new(|spec| format!("stub:{}", spec.description)));
    let text = runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call(
                    "c1",
                    "Agent",
                    r#"{"description":"bg","prompt":"do it","run_in_background":true}"#,
                ),
                assistant_tool_call(
                    "c2",
                    "TaskOutput",
                    r#"{"task_id":"task-1","wait":true,"timeout_ms":5000}"#,
                ),
                assistant_tool_call("c3", "TaskStop", r#"{"task_id":"task-1"}"#),
                Message::assistant_text("done"),
            ],
        )
        .await
        .expect("run");
    assert_eq!(text, "done");
    let tools: Vec<&Message> = runner
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .collect();
    assert!(
        tools[0].content.contains("task_id=task-1"),
        "{}",
        tools[0].content
    );
    assert!(tools[1].content.contains("stub:bg"), "{}", tools[1].content);
    assert!(
        tools[2].content.contains("早已结束"),
        "{}",
        tools[2].content
    );
    // 主 Agent 能看到后台任务工具，子 Agent 看不到。
    assert!(runner.tool_names().iter().any(|name| name == "TaskOutput"));
    let spec = parse_subagent_args(r#"{"prompt":"x","subagent_type":"general"}"#).unwrap();
    let child = runner.spawn_child_runner(&spec, 9);
    assert!(!child.tool_names().iter().any(|name| name == "TaskOutput"));
    assert!(!child
        .tool_names()
        .iter()
        .any(|name| name == "RespondToCoordinator"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compaction_options_apply_threshold_and_microcompact_flag() {
    let (mut runner, root) = temp_runner();
    runner.set_compaction_options(60, false);
    assert_eq!(runner.context_window.threshold_percent, 60);
    assert!(!runner.microcompact_enabled);
    runner.set_compaction_options(5, true);
    assert_eq!(runner.context_window.threshold_percent, 30);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn checkpoints_user_message_before_model_and_after_tool_round() {
    let (mut runner, root) = temp_runner();
    let snapshots = capture_checkpoints(&mut runner);
    let text = runner
        .run_scripted(
            "分析项目",
            vec![
                assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                Message::assistant_text("done"),
            ],
        )
        .await
        .expect("run");
    assert_eq!(text, "done");
    let snaps = snapshots.lock().await;
    assert!(
        snaps.len() >= 3,
        "expected user, tool-round, and final checkpoints: {}",
        snaps.len()
    );
    assert_eq!(
        snaps[0]
            .iter()
            .map(|message| (message.role, message.content.as_str()))
            .collect::<Vec<_>>(),
        vec![(Role::User, "分析项目")]
    );
    assert!(!snaps[0]
        .iter()
        .any(|message| message.role == Role::Assistant));
    let after_tools = snaps
        .iter()
        .find(|snapshot| snapshot.iter().any(|message| message.role == Role::Tool))
        .expect("tool-round checkpoint");
    assert!(after_tools
        .iter()
        .any(|message| { message.role == Role::Assistant && !message.tool_calls.is_empty() }));
    assert!(after_tools
        .iter()
        .any(|message| message.role == Role::Tool && message.content.contains("hello world")));
    let last = snaps.last().expect("final checkpoint");
    assert!(last
        .iter()
        .any(|message| message.role == Role::Assistant && message.content == "done"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn checkpoints_injected_steer_before_next_model_call() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(1);
    let (steer_tx, steer_rx) = mpsc::channel(1);
    runner.steer_rx = Some(Arc::new(Mutex::new(steer_rx)));
    steer_tx
        .send(NativeFollowup::input("补充约束"))
        .await
        .expect("steer");
    let snapshots = capture_checkpoints(&mut runner);
    let client = ModelClient::new(crate::native::model::client::ModelClientConfig {
        protocol: crate::native::protocol::PROTOCOL_OPENAI.to_string(),
        base_url: "http://127.0.0.1:1".to_string(),
        api_key: "test".to_string(),
        extra_headers: std::collections::HashMap::new(),
        retry: crate::native::model::RetryConfig::none(),
        timeout: Duration::from_millis(50),
        network: crate::app::network_settings::NetworkSettings::default(),
        responses_continuation: crate::native::model::ResponsesContinuationMode::Auto,
    })
    .expect("client");
    let text = runner
        .run_with_client(&client, "go", "test-model", None, None, false, Vec::new())
        .await
        .expect("budget stop");
    assert_eq!(text, LAST_TURN_FALLBACK);
    let snaps = snapshots.lock().await;
    assert!(
        snaps.iter().any(|snapshot| snapshot
            .iter()
            .any(|message| { message.role == Role::User && message.content == "go" })),
        "missing first user checkpoint: {snaps:?}"
    );
    let with_steer = snaps
        .iter()
        .find(|snapshot| {
            snapshot
                .iter()
                .any(|message| message.role == Role::User && message.content == "补充约束")
        })
        .expect("steer checkpoint");
    assert_eq!(
        with_steer
            .iter()
            .filter(|message| message.role == Role::User)
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>(),
        vec!["go", "补充约束"]
    );
    assert!(!with_steer
        .iter()
        .any(|message| message.role == Role::Assistant));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reads_then_edits_file() {
    let (mut runner, root) = temp_runner();
    let replies = vec![
        assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
        assistant_tool_call(
            "c2",
            "Edit",
            r#"{"file_path":"hello.txt","old_string":"hello world","new_string":"goodbye world"}"#,
        ),
        Message::assistant_text("done"),
    ];
    let text = runner
        .run_scripted("fix the greeting", replies)
        .await
        .expect("run");
    assert_eq!(text, "done");
    let content = fs::read_to_string(root.join("hello.txt")).expect("read result");
    assert_eq!(content, "goodbye world\n");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn cancel_stops_before_next_model_call() {
    let (mut runner, root) = temp_runner();
    runner.max_turns = 8;
    runner.cancel();
    let error = runner
        .run_scripted(
            "go",
            vec![assistant_tool_call(
                "c1",
                "Read",
                r#"{"file_path":"hello.txt"}"#,
            )],
        )
        .await
        .unwrap_err();
    assert_eq!(error, "已取消");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn last_turn_stops_with_fallback_instead_of_error() {
    let (mut runner, root) = temp_runner();
    runner.max_turns = 1;
    let text = runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                Message::assistant_text("should not run"),
            ],
        )
        .await
        .expect("last turn");
    assert_eq!(text, LAST_TURN_FALLBACK);
    let original = fs::read_to_string(root.join("hello.txt")).expect("read");
    assert_eq!(original, "hello world\n");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn last_turn_keeps_model_text() {
    let (mut runner, root) = temp_runner();
    runner.max_turns = 2;
    let text = runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                Message::assistant_text("审查通过"),
            ],
        )
        .await
        .expect("run");
    assert_eq!(text, "审查通过");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn budget_exhaustion_after_tool_response_still_runs_tool_calls() {
    let (mut probe, probe_root) = temp_runner();
    probe.set_allowed_tools(&["Read"]);
    probe.set_rollout_budget_limit(1_000_000);
    probe.messages.push(Message::user("go"));
    let tools = probe.combined_tools();
    assert!(probe.reserve_model_call(None, &tools).is_some());
    let spent = probe.budget_snapshot().spent;
    probe.release_model_reservation();
    let _ = fs::remove_dir_all(probe_root);

    let (mut runner, root) = temp_runner();
    runner.set_allowed_tools(&["Read"]);
    runner.set_rollout_budget_limit(spent);
    let _ = runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                Message::assistant_text("已读完"),
            ],
        )
        .await
        .expect("run");
    assert!(
        runner.messages.iter().any(|message| {
            message.role == Role::Tool && message.content.contains("hello world")
        }),
        "tool call from a tools-enabled request must still execute: {:?}",
        runner.messages
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn emits_tool_progress_lines() {
    let (mut runner, root) = temp_runner();
    let (tx, mut rx) = mpsc::unbounded_channel();
    runner.on_event = Some(tx);
    let _ = runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                Message::assistant_text("done"),
            ],
        )
        .await
        .expect("run");
    let lines = drain_events(&mut rx);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("[读取] hello.txt")),
        "missing read start: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("[工具结果]\n") && line.contains("hello world")),
        "missing full tool result: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line == "done"),
        "missing final: {lines:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn completed_plan_and_followup_never_enable_unapproved_mutations() {
    use crate::native::session::{next_loop_step, NativeLoopAction, NativeLoopEvent};
    for yolo in [false, true] {
        let (mut runner, root) = temp_runner();
        runner.set_plan_mode(true);
        runner.ctx.allow_all_high_risk.store(yolo, Ordering::SeqCst);
        runner.ctx.request_permission = Some(Arc::new(|_, _| {
            panic!("writers must be blocked before permission prompts")
        }));
        let original = fs::read(root.join("hello.txt")).unwrap();
        let plan = runner
            .run_scripted(
                "排查 API 日志",
                vec![Message::assistant_text("计划：修复请求并验证")],
            )
            .await
            .unwrap();
        assert!(!plan.is_empty());
        assert_eq!(
            next_loop_step(true, NativeLoopEvent::TurnFinished),
            NativeLoopAction::WaitFollowup
        );
        assert!(runner.ctx.is_read_only() && runner.is_plan_mode());
        let patch = r#"{"patch":"*** Begin Patch\n*** Update File: hello.txt\n*** Move to: moved.txt\n@@\n-hello world\n+changed\n*** Add File: new.txt\n+new\n*** End Patch"}"#;
        runner.run_scripted("计划阶段已结束，按方案立即实施", vec![
                assistant_tool_calls(&[
                    ("patch", "ApplyPatch", patch),
                    ("write", "Write", r#"{"file_path":"hello.txt","content":"changed"}"#),
                    ("edit", "Edit", r#"{"file_path":"hello.txt","old_string":"hello","new_string":"changed"}"#),
                    ("mcp", "mcp_fs_write", "{}"),
                ]),
                Message::assistant_text("等待批准"),
            ]).await.unwrap();
        assert!(runner.is_plan_mode());
        assert_eq!(fs::read(root.join("hello.txt")).unwrap(), original);
        assert!(!root.join("moved.txt").exists() && !root.join("new.txt").exists());
        assert_eq!(
            runner
                .messages
                .iter()
                .filter(|message| message.role == Role::Tool
                    && message.content.contains("只读规划模式禁止"))
                .count(),
            4
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn plan_approval_controls_later_writes_in_the_same_tool_batch() {
    use crate::native::tools::dispatch::PlanApprovalAnswer;
    for approved in [false, true] {
        let (mut runner, root) = temp_runner();
        runner.set_plan_mode(true);
        runner.ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
        runner.ctx.request_plan_approval = Some(Arc::new(move |_, tx| {
            tx.send(PlanApprovalAnswer {
                approved,
                feedback: String::new(),
                ..Default::default()
            })
            .unwrap();
        }));
        runner.run_scripted("提出计划", vec![
                assistant_tool_calls(&[
                    ("exit", "ExitPlanMode", r#"{"plan":"add new.txt"}"#),
                    ("patch", "ApplyPatch", r#"{"patch":"*** Begin Patch\n*** Add File: new.txt\n+approved\n*** End Patch"}"#),
                ]),
                Message::assistant_text("done"),
            ]).await.unwrap();
        assert_eq!(root.join("new.txt").exists(), approved);
        assert_eq!(runner.is_plan_mode(), !approved);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn plan_mode_advertises_sqlite_query_and_returns_database_rows_to_model() {
    use sqlx::Connection;
    let (mut runner, root) = temp_runner();
    let database = root.join("logs.sqlite");
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::raw_sql("CREATE TABLE logs(message TEXT); INSERT INTO logs VALUES('request failed');")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    runner.set_read_only(true);
    runner.set_plan_mode(true);
    assert!(runner.tool_names().iter().any(|name| name == "SQLiteQuery"));
    assert!(runner.tool_names().iter().any(|name| name == "Bash"));
    runner
        .run_scripted(
            "查一下数据库",
            vec![
                assistant_tool_call(
                    "query-logs",
                    "SQLiteQuery",
                    r#"{"file_path":"logs.sqlite","query":"SELECT message FROM logs"}"#,
                ),
                Message::assistant_text("已查到失败日志"),
            ],
        )
        .await
        .unwrap();
    assert!(runner
        .messages
        .iter()
        .any(|message| message.role == Role::Tool && message.content.contains("request failed")));
    assert_eq!(
        tool_args_summary(
            "SQLiteQuery",
            r#"{"file_path":"logs.sqlite","query":"SELECT message FROM logs"}"#
        ),
        "logs.sqlite"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn read_only_emits_read_and_blocks_write() {
    let (mut runner, root) = temp_runner();
    runner.set_read_only(true);
    runner.set_allowed_tools(&crate::native::tools::read_only_tool_names());
    let (tx, mut rx) = mpsc::unbounded_channel();
    runner.on_event = Some(tx);
    let replies = vec![
        assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
        assistant_tool_call(
            "c2",
            "Write",
            r#"{"file_path":"hello.txt","content":"changed"}"#,
        ),
        Message::assistant_text("plan ready"),
    ];
    let text = runner
        .run_scripted("plan the work", replies)
        .await
        .expect("run");
    assert_eq!(text, "plan ready");
    assert_eq!(
        fs::read_to_string(root.join("hello.txt")).expect("read"),
        "hello world\n"
    );
    let lines = drain_events(&mut rx);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("[读取] hello.txt")),
        "missing read: {lines:?}"
    );
    assert!(
        runner
            .messages
            .iter()
            .any(|message| message.content.contains("只读规划模式禁止调用工具 Write")),
        "expected write rejection in tool results: {:?}",
        runner
            .messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn rejects_repeated_identical_tool() {
    let (mut runner, root) = temp_runner();
    let replies = vec![
        assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
        assistant_tool_call("c2", "Read", r#"{"file_path":"hello.txt"}"#),
        assistant_tool_call("c3", "Read", r#"{"file_path":"hello.txt"}"#),
        Message::assistant_text("ok"),
    ];
    let text = runner.run_scripted("go", replies).await.expect("run");
    assert_eq!(text, "ok");
    let refused = runner
        .messages
        .iter()
        .any(|message| message.content.contains("重复调用被拒绝"));
    assert!(refused, "expected repeat rejection in tool results");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn empty_tool_names_do_not_continue() {
    let (mut runner, root) = temp_runner();
    let mut dummy = Message::assistant_text("hello");
    dummy.tool_calls = vec![ToolCall {
        id: "empty".to_string(),
        name: String::new(),
        arguments: "{}".to_string(),
    }];
    let text = runner.run_scripted("go", vec![dummy]).await.expect("run");
    assert_eq!(text, "hello");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn thinking_start_line_keeps_full_content() {
    assert_eq!(thinking_start_line("   ", 3), None);
    assert_eq!(
        thinking_start_line("先看入口再改 Composer", 8),
        Some("[思考] 8秒\n先看入口再改 Composer".to_string())
    );
    assert_eq!(thinking_duration_seconds(0), 1);
    assert_eq!(thinking_duration_seconds(499), 1);
    assert_eq!(thinking_duration_seconds(500), 1);
    assert_eq!(thinking_duration_seconds(1499), 1);
    assert_eq!(thinking_duration_seconds(1500), 2);
}

#[test]
fn todo_write_start_line_lists_all_items() {
    let line = tool_start_line(
        "TodoWrite",
        r#"{"todos":[
                {"id":"1","content":"定位 TestController","status":"in_progress","priority":"high"},
                {"id":"2","content":"实现 ok 接口","status":"pending"},
                {"id":"3","content":"补测试","status":"pending","priority":"low"}
            ]}"#,
    );
    assert_eq!(
            line,
            "[待办]\n- [in_progress] 定位 TestController (high)\n- [pending] 实现 ok 接口 (medium)\n- [pending] 补测试 (low)"
        );
    assert!(!line.contains("TodoWrite"));
}

#[test]
fn todo_write_start_line_empty_and_invalid() {
    assert_eq!(
        tool_start_line("TodoWrite", r#"{"todos":[]}"#),
        "[待办] (空)"
    );
    assert_eq!(
        tool_start_line("TodoWrite", "not-json"),
        "[待办] 更新任务清单"
    );
    assert_eq!(tool_start_line("TodoWrite", "{}"), "[待办] 更新任务清单");
}

#[test]
fn todo_read_start_line_is_label() {
    assert_eq!(tool_start_line("TodoRead", "{}"), "[待办] 读取任务清单");
}

#[test]
fn tool_start_line_covers_unmatched_builtins_and_mcp() {
    assert_eq!(
        tool_start_line("WebFetch", r#"{"url":"https://example.com"}"#),
        "[工具] WebFetch https://example.com"
    );
    assert_eq!(
        tool_start_line("WebSearch", r#"{"query":"tokio runtime"}"#),
        "[工具] WebSearch tokio runtime"
    );
    assert_eq!(
        tool_start_line(
            "AskUserQuestion",
            r#"{"questions":[{"prompt":"用哪种方案？"}]}"#
        ),
        "[工具] 提问 用哪种方案？"
    );
    assert_eq!(
        tool_start_line("mcp_fs_tools_list_files", r#"{"path":"/tmp"}"#),
        "[MCP工具] mcp_fs_tools_list_files /tmp"
    );
    assert_eq!(
        tool_start_line_ex(
            "mcp_fs_tools_list_files",
            r#"{"path":"/tmp"}"#,
            Some("fs.tools"),
            Some("list-files"),
        ),
        "[MCP工具] fs.tools / list-files /tmp"
    );
    assert_eq!(tool_event_title("[读取] src/main.ts"), "读取 src/main.ts");
}

#[test]
fn tool_args_summary_covers_computer() {
    assert_eq!(
        tool_args_summary("Computer", r#"{"action":"wait","duration_ms":500}"#),
        "等待 500 ms"
    );
    assert_eq!(
        tool_args_summary("Computer", r#"{"action":"click","x":100,"y":200}"#),
        "点击 (100, 200)"
    );
    assert_eq!(
        tool_args_summary("Computer", r#"{"action":"get_app_state","app":"Safari"}"#),
        "读取状态 Safari"
    );
}

#[test]
fn todo_write_result_summarizes_count() {
    let output = "- [in_progress] 定位 TestController (medium)\n- [pending] 实现 ok 接口 (medium)";
    assert_eq!(
        tool_result_line("TodoWrite", output),
        "[工具结果] 已更新 2 项"
    );
    assert_eq!(
        tool_result_line("TodoWrite", "(no todos)"),
        "[工具结果] 已更新 0 项"
    );
    assert_eq!(
        tool_result_line("TodoWrite", "todos 必须是数组"),
        "[工具结果]\ntodos 必须是数组"
    );
}

#[test]
fn todo_read_result_keeps_full_list() {
    let output =
        "- [completed] 定位 TestController (medium)\n- [in_progress] 实现 ok 接口 (medium)";
    assert_eq!(
            tool_result_line("TodoRead", output),
            "[工具结果]\n- [completed] 定位 TestController (medium)\n- [in_progress] 实现 ok 接口 (medium)"
        );
}

#[test]
fn other_tool_result_keeps_full_output() {
    assert_eq!(
        tool_result_line("Read", "line1\nline2"),
        "[工具结果]\nline1\nline2"
    );
    assert_eq!(
        tool_result_line("Grep", "a.rs:3:hit\nb.rs:9:hit"),
        "[工具结果]\na.rs:3:hit\nb.rs:9:hit"
    );
}

#[test]
fn tool_result_display_caps_lines_and_chars() {
    let many_lines = (0..2001)
        .map(|index| format!("L{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let line_result = tool_result_line("Read", &many_lines);
    assert!(line_result.starts_with("[工具结果]\nL0\n"));
    assert!(line_result.contains("L1999"));
    assert!(!line_result.contains("L2000"));
    assert!(line_result.contains("…（已截断，共 2001 行 / "));

    let huge = "a".repeat(TOOL_RESULT_DISPLAY_MAX_CHARS + 1);
    let char_result = tool_result_line("Bash", &huge);
    assert!(char_result.starts_with("[工具结果]\n"));
    assert!(
        char_result.contains("…（已截断，共 1 行 / 65537 字）"),
        "missing char cap notice: {}",
        char_result.chars().rev().take(40).collect::<String>()
    );
    let body = char_result
        .strip_prefix("[工具结果]\n")
        .and_then(|text| text.strip_suffix("\n…（已截断，共 1 行 / 65537 字）"))
        .expect("display wrapper");
    assert_eq!(body.chars().count(), TOOL_RESULT_DISPLAY_MAX_CHARS);
    assert!(body.chars().all(|ch| ch == 'a'));
}

#[tokio::test]
async fn emits_todo_write_list() {
    let (mut runner, root) = temp_runner();
    let (tx, mut rx) = mpsc::unbounded_channel();
    runner.on_event = Some(tx);
    let _ = runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call(
                    "t1",
                    "TodoWrite",
                    r#"{"todos":[
                            {"content":"定位 TestController","status":"in_progress"},
                            {"content":"实现 ok 接口","status":"pending"},
                            {"content":"补测试","status":"pending"}
                        ]}"#,
                ),
                Message::assistant_text("done"),
            ],
        )
        .await
        .expect("run");
    let lines = drain_events(&mut rx);
    let start = lines
        .iter()
        .find(|line| line.starts_with("[待办]"))
        .expect("missing todo start");
    assert!(
        start.contains("- [in_progress] 定位 TestController (medium)")
            && start.contains("- [pending] 实现 ok 接口 (medium)")
            && start.contains("- [pending] 补测试 (medium)"),
        "todo list missing items: {start}"
    );
    assert!(
        lines.iter().any(|line| line == "[工具结果] 已更新 3 项"),
        "missing todo result count: {lines:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn child_uses_dedicated_subagent_turns() {
    let (mut runner, root) = temp_runner();
    let spec = parse_subagent_args(r#"{"prompt":"go"}"#).unwrap();
    let default_child = runner.spawn_child_runner(&spec, 1);
    assert_eq!(default_child.max_turns, 20);
    assert_eq!(default_child.max_subagent_turns, 20);

    runner.max_turns = 40;
    runner.max_subagent_turns = 80;
    let child = runner.spawn_child_runner(&spec, 2);
    assert_eq!(child.max_turns, 80);
    assert_eq!(child.max_subagent_turns, 80);

    runner.max_turns = 0;
    runner.max_subagent_turns = 20;
    let limited = runner.spawn_child_runner(&spec, 3);
    assert_eq!(limited.max_turns, 20);

    runner.max_subagent_turns = 0;
    let unlimited = runner.spawn_child_runner(&spec, 4);
    assert_eq!(unlimited.max_turns, 0);
    assert_eq!(unlimited.max_subagent_turns, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parent_has_agent_child_and_readonly_do_not() {
    let (mut runner, root) = temp_runner();
    runner.ctx.extra_env = vec![("HTTPS_PROXY".to_string(), "http://proxy".to_string())];
    assert!(runner.tool_names().iter().any(|name| name == "Agent"));
    let general = parse_subagent_args(r#"{"prompt":"go","description":"改文件"}"#).unwrap();
    let child = runner.spawn_child_runner(&general, 1);
    assert!(!child.tool_names().iter().any(|name| name == "Agent"));
    assert!(!child.ctx.is_read_only());
    assert_eq!(child.ctx.extra_env, runner.ctx.extra_env);
    assert_eq!(child.event_prefix, "[子 Agent 1(general) - 改文件] ");
    let explore = parse_subagent_args(r#"{"prompt":"go","subagent_type":"explore"}"#).unwrap();
    let explore_child = runner.spawn_child_runner(&explore, 2);
    assert!(explore_child.ctx.is_read_only());
    assert!(!explore_child
        .tool_names()
        .iter()
        .any(|name| name == "Agent"));
    assert!(!explore_child
        .tool_names()
        .iter()
        .any(|name| name == "Write"));
    assert!(explore_child.tool_names().iter().any(|name| name == "Bash"));
    let mut custom_runner = AgentRunner::new(LocalWorkspace::new(root.clone()));
    custom_runner.custom_subagents = vec![readonly_custom_subagent()];
    custom_runner.workspace_context = "Working directory: /repo".to_string();
    custom_runner.project_agents = "secret agents".to_string();
    let custom = parse_subagent_args_with(
        r#"{"prompt":"go","subagent_type":"reviewer","description":"审"}"#,
        &custom_runner.custom_subagents,
    )
    .unwrap();
    let custom_child = custom_runner.spawn_child_runner(&custom, 3);
    assert!(custom_child.ctx.is_read_only());
    assert!(custom_child.tool_names().iter().any(|name| name == "Read"));
    assert!(!custom_child.tool_names().iter().any(|name| name == "Write"));
    assert!(!custom_child.tool_names().iter().any(|name| name == "Agent"));
    assert!(!custom_child.tool_names().iter().any(|name| name == "Bash"));
    let system = custom_child
        .messages
        .iter()
        .find(|message| message.role == Role::System)
        .map(|message| message.content.clone())
        .unwrap_or_default();
    assert!(system.contains("你是审查员"));
    assert!(system.contains("Working directory: /repo"));
    assert!(!system.contains("secret agents"));
    let mut readonly = AgentRunner::new(LocalWorkspace::new(root.clone()));
    readonly.set_read_only(true);
    readonly.set_allowed_tools(&crate::native::tools::read_only_tool_names());
    assert!(!readonly.tool_names().iter().any(|name| name == "Agent"));
    assert!(!readonly.tool_names().iter().any(|name| name == "Bash"));
    let mut extra = AgentRunner::new(LocalWorkspace::new(root.clone()));
    extra.set_read_only(true);
    extra.set_extra_tools(vec![ToolSpec {
        name: "mcp_fs_write".to_string(),
        description: "mcp".to_string(),
        parameters: serde_json::json!({}),
    }]);
    let extra_names = extra.tool_names();
    assert!(extra_names.iter().any(|name| name == "Read"));
    assert!(!extra_names.iter().any(|name| name == "Write"));
    assert!(!extra_names.iter().any(|name| name == "Agent"));
    assert!(!extra_names.iter().any(|name| name == "mcp_fs_write"));
    extra.set_read_only(false);
    assert!(extra.tool_names().iter().any(|name| name == "Write"));
    let mut plan = AgentRunner::new(LocalWorkspace::new(root.clone()));
    plan.custom_subagents = vec![readonly_custom_subagent()];
    plan.set_read_only(true);
    plan.set_plan_mode(true);
    let plan_tools = plan.combined_tools();
    let plan_agent = plan_tools
        .iter()
        .find(|tool| tool.name == "Agent")
        .expect("plan Agent tool");
    assert!(plan_agent.description.contains("- general:"));
    assert!(plan_agent.description.contains("- explore:"));
    assert!(plan_agent
        .description
        .contains("- reviewer: review (Tools: Read, Grep)"));
    assert!(plan_agent
        .description
        .contains("Plan mode: only the built-in subagent_type=explore"));
    let plan_names = plan_tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    assert!(plan_names.iter().any(|name| name == "SQLiteQuery"));
    assert!(plan_names.iter().any(|name| name == "Bash"));
    assert!(plan_names.iter().any(|name| name == "Agent"));
    assert!(plan_names.iter().any(|name| name == "AskUserQuestion"));
    assert!(plan_names.iter().any(|name| name == "ExitPlanMode"));
    assert!(!plan_names.iter().any(|name| name == "EnterPlanMode"));
    assert!(plan_names.iter().any(|name| name == "Skill"));
    assert!(!plan_names.iter().any(|name| name == "Write"));
    assert!(!plan_names.iter().any(|name| name == "ApplyPatch"));
    assert!(!plan_names.iter().any(|name| name == "Computer"));
    plan.set_plan_mode(false);
    plan.set_read_only(false);
    let exec_names = plan.tool_names();
    // 执行模式：提问工具始终可用，EnterPlanMode 可见，ExitPlanMode 隐藏。
    assert!(exec_names.iter().any(|name| name == "AskUserQuestion"));
    assert!(exec_names.iter().any(|name| name == "EnterPlanMode"));
    assert!(!exec_names.iter().any(|name| name == "ExitPlanMode"));
    // 子 Agent 没有交互通道，看不到提问与计划模式工具。
    let child_names = child.tool_names();
    assert!(!child_names.iter().any(|name| name == "AskUserQuestion"));
    assert!(!child_names.iter().any(|name| name == "EnterPlanMode"));
    runner.cancel();
    assert!(child.ctx.cancel.is_cancelled());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn child_runner_shares_rollout_budget() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(10_000);
    let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
    let child = runner.spawn_child_runner(&spec, 1);
    assert!(Arc::ptr_eq(&runner.rollout_budget, &child.rollout_budget));
    assert_eq!(child.budget_snapshot().limit, 10_000);
    assert_eq!(
        child.child_quota.as_ref().map(|quota| quota.limit()),
        Some(4_000)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn child_quota_stops_before_parent_budget() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(10_000);
    runner.subagent_budget_share_percent = 40;
    let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
    let mut child = runner.spawn_child_runner(&spec, 1);
    child.child_quota = Some(ChildQuota::shared(1));
    child.messages.push(Message::user("hello world"));
    assert!(child.reserve_model_call(Some(1_000), &[]).is_none());
    assert_eq!(runner.budget_snapshot().remaining, 10_000);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn batch_children_share_one_quota_pool() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(10_000);
    runner.subagent_budget_share_percent = 40;
    let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
    let pool = runner.child_quota_for_share();
    let first = runner.spawn_child_with_quota(&spec, 1, pool.clone());
    let second = runner.spawn_child_with_quota(&spec, 2, pool.clone());
    let third = runner.spawn_child_with_quota(&spec, 3, pool.clone());
    assert!(Arc::ptr_eq(
        first.child_quota.as_ref().unwrap(),
        second.child_quota.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        second.child_quota.as_ref().unwrap(),
        third.child_quota.as_ref().unwrap()
    ));
    assert_eq!(
        first.child_quota.as_ref().map(|quota| quota.limit()),
        Some(4_000)
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn budget_exhaustion_forces_tool_free_final_turn() {
    let (mut runner, root) = temp_runner();
    // The first request estimate is deliberately larger than this budget,
    // so the scripted model must receive a tool-free final turn.
    runner.set_rollout_budget_limit(8);
    let text = runner
        .run_scripted(
            "go",
            vec![assistant_tool_call(
                "c1",
                "Read",
                r#"{"file_path":"hello.txt"}"#,
            )],
        )
        .await
        .expect("budget final turn");
    assert_eq!(text, LAST_TURN_FALLBACK);
    assert!(runner.budget_snapshot().limit == 8);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn exhausted_budget_stops_without_an_extra_model_request() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(1);
    let client = ModelClient::new(crate::native::model::client::ModelClientConfig {
        protocol: crate::native::protocol::PROTOCOL_OPENAI.to_string(),
        base_url: "http://127.0.0.1:1".to_string(),
        api_key: "test".to_string(),
        extra_headers: std::collections::HashMap::new(),
        retry: crate::native::model::RetryConfig::none(),
        timeout: Duration::from_millis(50),
        network: crate::app::network_settings::NetworkSettings::default(),
        responses_continuation: crate::native::model::ResponsesContinuationMode::Auto,
    })
    .expect("client");
    let text = runner
        .run_with_client(&client, "go", "test-model", None, None, false, Vec::new())
        .await
        .expect("budget stop");
    assert_eq!(text, LAST_TURN_FALLBACK);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn final_request_output_is_clamped_to_remaining_budget() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(300);
    runner.messages.push(Message::user("task"));
    let budget = runner
        .reserve_model_call(Some(10_000), &[])
        .expect("reservation");
    assert!(budget.max_output_tokens.expect("output cap") < 10_000);
    runner.release_model_reservation();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ample_budget_caps_unconfigured_output_to_fallback_guard() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(1_000_000);
    runner.messages.push(Message::user("task"));
    let budget = runner.reserve_model_call(None, &[]).expect("reservation");
    assert_eq!(
        budget.max_output_tokens,
        Some(FALLBACK_OUTPUT_TOKEN_GUARD as u32)
    );
    runner.release_model_reservation();
    let budget = runner
        .reserve_model_call(Some(60_000), &[])
        .expect("reservation");
    assert_eq!(budget.max_output_tokens, Some(60_000));
    runner.release_model_reservation();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_usage_settles_using_response_text() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(100_000);
    runner.messages.push(Message::user("task"));
    let _ = runner.reserve_model_call(None, &[]).expect("reserve");
    let reserved = runner.budget_snapshot().spent;
    let assistant = Message::assistant_text("x".repeat(80_000));
    runner.settle_model_usage(Usage::default(), Some(&assistant));
    assert!(
        runner.budget_snapshot().spent > reserved,
        "missing usage must charge the response text, spent={} reserved={reserved}",
        runner.budget_snapshot().spent
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn near_exhaustion_caps_unconfigured_output_to_remaining_budget() {
    let (mut runner, root) = temp_runner();
    runner.set_rollout_budget_limit(2_000);
    runner.messages.push(Message::user("task"));
    let budget = runner.reserve_model_call(None, &[]).expect("reservation");
    let cap = u64::from(budget.max_output_tokens.expect("near-exhaustion cap"));
    assert!(cap < 2_000);
    runner.release_model_reservation();
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn oversized_tool_schemas_force_tool_free_final_turn() {
    let (mut runner, root) = temp_runner();
    runner.context_window.set_token_limit(32);
    let (tx, mut rx) = mpsc::unbounded_channel();
    runner.on_event = Some(tx);
    runner.messages.push(Message::system("rules"));
    runner.messages.push(Message::user("task"));
    runner.set_extra_tools(vec![ToolSpec {
        name: "mcp_large".to_string(),
        description: "schema ".repeat(512),
        parameters: serde_json::json!({"type":"object"}),
    }]);

    let last_turn = runner
        .prepare_model_call(None)
        .await
        .expect("prepare model call");
    assert!(last_turn);
    assert!(drain_events(&mut rx)
        .iter()
        .any(|line| line.contains("工具定义已超过上下文窗口")));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn large_tool_result_is_bounded_in_model_history() {
    let (mut runner, root) = temp_runner();
    runner.tool_result_token_limit = 40;
    let mut assistant = Message::assistant_text("");
    assistant.content.clear();
    assistant.tool_calls = vec![ToolCall {
        id: "c1".to_string(),
        name: "Bash".to_string(),
        arguments: r#"{"command":"printf huge"}"#.to_string(),
    }];
    // Use a scripted tool call that returns the normal shell output path;
    // direct helper coverage in truncate.rs covers the large payload.
    let _ = runner
        .run_scripted("go", vec![assistant, Message::assistant_text("done")])
        .await
        .expect("run");
    let tool_messages = runner
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .collect::<Vec<_>>();
    assert!(tool_messages
        .iter()
        .all(|message| message.content.chars().count() < 200));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostics_count_tool_result_truncation() {
    let (mut runner, root) = temp_runner();
    runner.tool_result_token_limit = 16;
    let call = ToolCall {
        id: "c1".to_string(),
        name: "Read".to_string(),
        arguments: r#"{"file_path":"hello.txt"}"#.to_string(),
    };
    runner.append_tool_message(&call, ToolOutput::text("line\n".repeat(500)));
    assert_eq!(runner.diagnostics_snapshot().tool_results_truncated, 1);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn agent_missing_prompt_keeps_loop_going() {
    let (mut runner, root) = temp_runner();
    let text = runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call("c1", "Agent", r#"{"description":"x"}"#),
                Message::assistant_text("ok"),
            ],
        )
        .await
        .expect("run");
    assert_eq!(text, "ok");
    assert!(runner
        .messages
        .iter()
        .any(|message| message.content.contains("prompt 不能为空")));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_agent_stubs_overlap() {
    let (mut runner, root) = temp_runner();
    let (tx, mut rx) = mpsc::unbounded_channel();
    runner.on_event = Some(tx);
    let live = Arc::new(AtomicU32::new(0));
    let max = Arc::new(AtomicU32::new(0));
    let live_clone = live.clone();
    let max_clone = max.clone();
    runner.subagent_stub = Some(Arc::new(move |spec: &SubagentSpec| {
        let now = live_clone.fetch_add(1, Ordering::SeqCst) + 1;
        max_clone.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(80));
        live_clone.fetch_sub(1, Ordering::SeqCst);
        format!("done {}", spec.description)
    }));
    let mut assistant = Message::assistant_text("");
    assistant.content.clear();
    assistant.tool_calls = vec![
        ToolCall {
            id: "a1".to_string(),
            name: "Agent".to_string(),
            arguments: r#"{"description":"one","prompt":"p1"}"#.to_string(),
        },
        ToolCall {
            id: "a2".to_string(),
            name: "Agent".to_string(),
            arguments: r#"{"description":"two","prompt":"p2"}"#.to_string(),
        },
    ];
    let text = runner
        .run_scripted(
            "go",
            vec![assistant, Message::assistant_text("parent done")],
        )
        .await
        .expect("run");
    assert_eq!(text, "parent done");
    assert!(
        max.load(Ordering::SeqCst) >= 2,
        "expected overlapping stubs, max={}",
        max.load(Ordering::SeqCst)
    );
    let lines = drain_events(&mut rx);
    assert!(lines
        .iter()
        .any(|line| line.contains("[子 Agent 1(general) - one] 启动")));
    assert!(lines
        .iter()
        .any(|line| line.contains("[子 Agent 2(general) - two] 启动")));
    assert!(lines
        .iter()
        .any(|line| line.starts_with("[子 Agent 1(general) - one] [子 Agent] one")));
    assert!(lines
        .iter()
        .any(|line| line.starts_with("[子 Agent 2(general) - two] [子 Agent] two")));
    assert!(lines
        .iter()
        .any(|line| line.starts_with("[子 Agent 1(general) - one] [工具结果]")));
    assert!(lines
        .iter()
        .any(|line| line.starts_with("[子 Agent 2(general) - two] [工具结果]")));
    assert!(runner
        .messages
        .iter()
        .any(|message| message.content.contains("done one")));
    assert!(runner
        .messages
        .iter()
        .any(|message| message.content.contains("done two")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cron_update_is_visible_only_to_non_plan_parent() {
    let (mut runner, root) = temp_runner();
    assert!(runner.tool_names().iter().any(|name| name == "CronUpdate"));
    runner.set_plan_mode(true);
    assert!(!runner.tool_names().iter().any(|name| name == "CronUpdate"));
    runner.set_plan_mode(false);
    let spec = parse_subagent_args(r#"{"prompt":"go","description":"child"}"#).unwrap();
    let child = runner.spawn_child_runner(&spec, 1);
    assert!(!child.tool_names().iter().any(|name| name == "CronUpdate"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn computer_is_advertised_only_for_local_enabled_parent() {
    let (mut runner, root) = temp_runner();
    assert!(!runner.tool_names().iter().any(|name| name == "Computer"));
    runner.ctx.computer_control_enabled = true;
    assert!(runner.tool_names().iter().any(|name| name == "Computer"));
    runner.set_plan_mode(true);
    runner.set_read_only(true);
    assert!(!runner.tool_names().iter().any(|name| name == "Computer"));
    runner.set_plan_mode(false);
    runner.set_read_only(false);
    let spec = parse_subagent_args(r#"{"prompt":"go","description":"child"}"#).unwrap();
    let child = runner.spawn_child_runner(&spec, 1);
    assert!(!child.tool_names().iter().any(|name| name == "Computer"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn agent_tool_description_uses_runner_cap() {
    let (mut runner, root) = temp_runner();
    runner.max_concurrent_subagents = 4;
    runner.subagent_policy = "aggressive".to_string();
    let agent = runner
        .combined_tools()
        .into_iter()
        .find(|tool| tool.name == "Agent")
        .expect("Agent tool");
    assert!(
        agent.description.contains("max 4"),
        "description should include cap: {}",
        agent.description
    );
    assert!(agent.description.contains("Policy aggressive"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_cap_one_does_not_overlap() {
    let (mut runner, root) = temp_runner();
    runner.max_concurrent_subagents = 1;
    let live = Arc::new(AtomicU32::new(0));
    let max = Arc::new(AtomicU32::new(0));
    let live_clone = live.clone();
    let max_clone = max.clone();
    runner.subagent_stub = Some(Arc::new(move |spec: &SubagentSpec| {
        let now = live_clone.fetch_add(1, Ordering::SeqCst) + 1;
        max_clone.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        live_clone.fetch_sub(1, Ordering::SeqCst);
        format!("done {}", spec.description)
    }));
    let mut assistant = Message::assistant_text("");
    assistant.content.clear();
    assistant.tool_calls = vec![
        ToolCall {
            id: "a1".to_string(),
            name: "Agent".to_string(),
            arguments: r#"{"description":"one","prompt":"p1"}"#.to_string(),
        },
        ToolCall {
            id: "a2".to_string(),
            name: "Agent".to_string(),
            arguments: r#"{"description":"two","prompt":"p2"}"#.to_string(),
        },
    ];
    let _ = runner
        .run_scripted(
            "go",
            vec![assistant, Message::assistant_text("parent done")],
        )
        .await
        .expect("run");
    assert_eq!(max.load(Ordering::SeqCst), 1, "cap 1 should not overlap");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn agent_deny_rule_prevents_delegation() {
    use crate::native::tools::contract::{PatternSource, PermissionCapability};
    use crate::native::tools::permission::{PermissionRule, RuleEffect, RuleScope};
    let (mut runner, root) = temp_runner();
    runner.subagent_stub = Some(Arc::new(|_| panic!("denied Agent must not start")));
    runner.ctx.permission_rules.write().unwrap().push(
        RuleEffect::Deny,
        PermissionRule {
            id: "deny-agent".to_string(),
            external_path: None,
            plan_bash: None,
            capability: PermissionCapability::Subagent,
            pattern: "*".to_string(),
            source: PatternSource::ToolName,
            scope: RuleScope::Workspace,
            note: String::new(),
        },
    );
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call("agent", "Agent", r#"{"prompt":"do it"}"#),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(runner.diagnostics_snapshot().subagents_started, 0);
    assert!(runner.messages.iter().any(|message| {
        message.role == Role::Tool && message.content.contains("权限规则拒绝")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn enter_plan_mode_blocks_non_explore_agent_in_same_tool_batch() {
    let (mut runner, root) = temp_runner();
    runner.subagent_stub = Some(Arc::new(|_| panic!("general Agent must not start")));
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_calls(&[
                    ("plan", "EnterPlanMode", "{}"),
                    ("agent", "Agent", r#"{"prompt":"change files"}"#),
                ]),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    assert!(runner.is_plan_mode());
    assert_eq!(runner.diagnostics_snapshot().subagents_started, 0);
    assert!(runner.messages.iter().any(|message| {
        message.role == Role::Tool
            && message
                .content
                .contains("计划模式只能使用内置 explore 子智能体")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn enter_plan_mode_allows_explore_in_same_tool_batch() {
    let (mut runner, root) = temp_runner();
    runner.subagent_stub = Some(Arc::new(|spec| {
        assert!(matches!(&spec.kind, SubagentKind::Explore));
        "explored".to_string()
    }));
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_calls(&[
                    ("plan", "EnterPlanMode", "{}"),
                    (
                        "agent",
                        "Agent",
                        r#"{"prompt":"inspect files","subagent_type":"explore"}"#,
                    ),
                ]),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    assert!(runner.is_plan_mode());
    assert_eq!(runner.diagnostics_snapshot().subagents_started, 1);
    assert!(runner.messages.iter().any(|message| {
        message.role == Role::Tool
            && message.tool_call_id == "agent"
            && message.content.contains("explored")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn plan_mode_starts_only_builtin_explore() {
    let (mut runner, root) = temp_runner();
    runner.custom_subagents = vec![readonly_custom_subagent()];
    runner.set_read_only(true);
    runner.set_plan_mode(true);
    runner.subagent_stub = Some(Arc::new(|spec| {
        assert!(matches!(&spec.kind, SubagentKind::Explore));
        "explored".to_string()
    }));
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_calls(&[
                    (
                        "explore",
                        "Agent",
                        r#"{"prompt":"inspect","subagent_type":"explore"}"#,
                    ),
                    ("default", "Agent", r#"{"prompt":"default"}"#),
                    (
                        "general",
                        "Agent",
                        r#"{"prompt":"general","subagent_type":"general"}"#,
                    ),
                    (
                        "custom",
                        "Agent",
                        r#"{"prompt":"review","subagent_type":"reviewer"}"#,
                    ),
                ]),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(runner.diagnostics_snapshot().subagents_started, 1);
    assert!(runner.messages.iter().any(|message| {
        message.role == Role::Tool
            && message.tool_call_id == "explore"
            && message.content.contains("explored")
    }));
    for call_id in ["default", "general", "custom"] {
        assert!(runner.messages.iter().any(|message| {
            message.role == Role::Tool
                && message.tool_call_id == call_id
                && message
                    .content
                    .contains("计划模式只能使用内置 explore 子智能体")
        }));
    }
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn read_only_without_plan_still_rejects_agent_tool() {
    let (mut runner, root) = temp_runner();
    runner.set_read_only(true);
    runner.subagent_stub = Some(Arc::new(|_| panic!("read-only Agent must not start")));
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call(
                    "agent",
                    "Agent",
                    r#"{"prompt":"inspect","subagent_type":"explore"}"#,
                ),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    assert!(!runner.is_plan_mode());
    assert_eq!(runner.diagnostics_snapshot().subagents_started, 0);
    assert!(runner.messages.iter().any(|message| {
        message.role == Role::Tool
            && message.tool_call_id == "agent"
            && message.content.contains("只读规划模式禁止调用工具 Agent")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn agent_pre_hook_can_deny_or_rewrite_and_post_hook_is_applied() {
    use crate::db::models::NativeHook;
    let (mut runner, root) = temp_runner();
    runner.ctx.hooks = vec![NativeHook::shell(
        "deny-agent",
        "pre_tool_use",
        "Agent",
        r#"printf '%s' '{"decision":"deny","reason":"blocked delegation"}'"#,
        5,
        true,
    )];
    runner.subagent_stub = Some(Arc::new(|spec| format!("received:{}", spec.prompt)));
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call("blocked", "Agent", r#"{"prompt":"original"}"#),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(runner.diagnostics_snapshot().subagents_started, 0);
    assert!(runner
        .messages
        .iter()
        .any(|message| message.content.contains("blocked delegation")));
    runner.ctx.hooks = vec![
        NativeHook::shell(
            "rewrite-agent",
            "pre_tool_use",
            "Agent",
            r#"printf '%s' '{"updated_input":{"prompt":"rewritten"},"additional_context":"pre context"}'"#,
            5,
            true,
        ),
        NativeHook::shell(
            "post-agent",
            "post_tool_use",
            "Agent",
            r#"printf '%s' '{"additional_context":"post context"}'"#,
            5,
            true,
        ),
    ];
    runner
        .run_scripted(
            "again",
            vec![
                assistant_tool_call("rewrite", "Agent", r#"{"prompt":"original"}"#),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    let result = runner
        .messages
        .iter()
        .find(|message| message.tool_call_id == "rewrite")
        .unwrap();
    assert!(result.content.contains("received:rewritten"));
    assert!(result.content.contains("pre context"));
    assert!(result.content.contains("post context"));
    runner.ctx.hooks = vec![NativeHook::shell(
        "failure-agent",
        "post_tool_use_failure",
        "Agent",
        "printf 'failure recorded' > hook-failure.txt",
        5,
        true,
    )];
    runner
        .run_scripted(
            "invalid delegation",
            vec![
                assistant_tool_call("invalid", "Agent", "{}"),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("hook-failure.txt")).unwrap(),
        "failure recorded"
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreground_and_background_share_cap_across_batches() {
    let (mut runner, root) = temp_runner();
    runner.max_concurrent_subagents = 1;
    let live = Arc::new(AtomicU32::new(0));
    let peak = Arc::new(AtomicU32::new(0));
    let live_stub = live.clone();
    let peak_stub = peak.clone();
    runner.subagent_stub = Some(Arc::new(move |spec| {
        let current = live_stub.fetch_add(1, Ordering::SeqCst) + 1;
        peak_stub.fetch_max(current, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        live_stub.fetch_sub(1, Ordering::SeqCst);
        spec.description.clone()
    }));
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call(
                    "bg1",
                    "Agent",
                    r#"{"prompt":"one","run_in_background":true}"#,
                ),
                assistant_tool_call(
                    "bg2",
                    "Agent",
                    r#"{"prompt":"two","run_in_background":true}"#,
                ),
                assistant_tool_call("fg", "Agent", r#"{"prompt":"three"}"#),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    for task in runner.background.list() {
        let status = runner
            .background
            .wait(&task.id, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(matches!(
            status,
            super::super::background::TaskStatus::Done(_)
        ));
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
    assert_eq!(live.load(Ordering::SeqCst), 0);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn queued_background_task_can_be_cancelled_without_starting() {
    let (mut runner, root) = temp_runner();
    let semaphore = Arc::new(Semaphore::new(1));
    let held = semaphore.clone().acquire_owned().await.unwrap();
    runner.subagent_semaphore = Some(semaphore.clone());
    runner.subagent_stub = Some(Arc::new(|_| panic!("cancelled queued task started")));
    runner
        .run_scripted(
            "go",
            vec![
                assistant_tool_call(
                    "bg",
                    "Agent",
                    r#"{"prompt":"blocked","run_in_background":true}"#,
                ),
                Message::assistant_text("done"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(runner.background.snapshots()[0].status, "queued");
    assert_eq!(runner.background.stop("task-1"), Some(true));
    tokio::time::sleep(Duration::from_millis(60)).await;
    drop(held);
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(runner.background.snapshots()[0].status, "stopped");
    assert_eq!(semaphore.available_permits(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn background_cancellation_terminates_bash_descendants() {
    use serde_json::json;
    for cancel_parent in [false, true] {
        let (mut runner, root) = temp_runner();
        runner.max_concurrent_subagents = 1;
        runner.model_turn = Some(ModelTurnCfg {
            model: "test-model".to_string(),
            effort: None,
            max_output_tokens: Some(1024),
            thinking_enabled: false,
        });
        let command = concat!(
            "sh -c 'printf ready > started.txt; attempt=0; ",
            "while [ ! -f release.txt ] && [ \"$attempt\" -lt 500 ]; do ",
            "sleep 0.01; attempt=$((attempt + 1)); done; ",
            "if [ -f release.txt ]; then printf leaked > leaked.txt; fi' & wait"
        );
        let response = json!({"choices":[{"message":{
            "role":"assistant","content":null,"tool_calls":[{
                "id":"bash","type":"function","function":{
                    "name":"Bash","arguments":json!({"command":command}).to_string()
                }
            }]
        }}]});
        let (client, server) = mock_child_model(
            runner.background.clone(),
            "task-1".to_string(),
            vec![(response, None)],
        )
        .await;
        runner
            .run_agent_batch(
                &[ToolCall {
                    id: "background".to_string(),
                    name: "Agent".to_string(),
                    arguments: r#"{"prompt":"run command","run_in_background":true}"#.to_string(),
                }],
                Some(&client),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !root.join("started.txt").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Bash descendant must start before cancellation");
        assert!(!root.join("leaked.txt").exists());
        if cancel_parent {
            runner.cancel();
        } else {
            assert_eq!(runner.background.stop("task-1"), Some(true));
        }
        let semaphore = runner.subagent_semaphore.as_ref().unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while semaphore.available_permits() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled task must release its execution permit");
        assert_eq!(runner.background.snapshots()[0].status, "stopped");
        // Only a surviving descendant can observe this signal and write the leak marker.
        fs::write(root.join("release.txt"), "release").unwrap();
        tokio::time::sleep(Duration::from_millis(750)).await;
        assert!(
            !root.join("leaked.txt").exists(),
            "Bash descendant kept writing after cancellation (parent={cancel_parent})"
        );
        assert_eq!(server.await.unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn queued_inputs_wait_for_parent_summary_after_subagent_completion() {
    use crate::native::input_queue::NativeInputQueue;
    use serde_json::json;

    let (mut runner, root) = temp_runner();
    let queue = NativeInputQueue::new("session");
    queue.enqueue("deep rule analysis", vec![]).unwrap();
    queue.enqueue("review risks", vec![]).unwrap();
    let (_control_tx, control_rx) = mpsc::channel(8);
    runner.steer_rx = Some(Arc::new(Mutex::new(control_rx)));
    let response = |message| json!({"choices":[{"message":message}]});
    let (client, server) = mock_child_model(
            runner.background.clone(),
            String::new(),
            vec![
                (response(json!({"role":"assistant","content":null,"tool_calls":[{
                    "id":"agent","type":"function","function":{
                        "name":"Agent","arguments": "{\"prompt\":\"inspect project\",\"subagent_type\":\"explore\"}"
                    }
                }]})), None),
                (response(json!({"role":"assistant","content":"subagent project report"})), None),
                (response(json!({"role":"assistant","content":"complete project summary"})), None),
                (response(json!({"role":"assistant","content":"rule analysis result"})), None),
                (response(json!({"role":"assistant","content":"risk review result"})), None),
            ],
        ).await;

    let first = runner
        .run_with_client(
            &client,
            "analyze project",
            "test-model",
            None,
            Some(1024),
            false,
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(first, "complete project summary");
    assert_eq!(queue.snapshot().items.len(), 2);
    assert!(!runner
        .messages
        .iter()
        .any(|message| message.content == "deep rule analysis"));
    for expected in ["rule analysis result", "risk review result"] {
        let input = queue
            .recv(
                &runner.ctx.cancel,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .await
            .unwrap();
        let result = runner
            .run_with_client(
                &client,
                &input.text,
                "test-model",
                None,
                Some(1024),
                false,
                input.images,
            )
            .await
            .unwrap();
        assert_eq!(result, expected);
    }
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 5);
    for request in &requests[..3] {
        assert!(!request.to_string().contains("deep rule analysis"));
    }
    let messages = requests[3]["messages"].as_array().unwrap();
    let summary = messages
        .iter()
        .position(|message| message["content"] == "complete project summary")
        .unwrap();
    let next = messages
        .iter()
        .position(|message| message["content"] == "deep rule analysis")
        .unwrap();
    assert!(summary < next);
    assert!(!requests[3].to_string().contains("review risks"));
    assert!(requests[4].to_string().contains("rule analysis result"));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn child_model_consumes_steer_between_tools_and_before_final_return() {
    use serde_json::json;
    let (parent, root) = temp_runner();
    let spec = parse_subagent_args(r#"{"prompt":"read file"}"#).unwrap();
    let mut child = parent.spawn_child_runner(&spec, 1);
    let (task, receiver) = parent.background.register("steer test", "general");
    child.ctx.coordinator = Some((parent.background.clone(), task.id.clone()));
    child.steer_rx = Some(Arc::new(Mutex::new(receiver)));
    parent.background.mark_running(&task.id);
    let response = |message| json!({"choices":[{"message":message}]});
    let (client, server) = mock_child_model(
        parent.background.clone(),
        task.id.clone(),
        vec![
            (
                response(json!({"role":"assistant","content":null,"tool_calls":[{
                    "id":"read","type":"function","function":{
                        "name":"Read","arguments":"{\"file_path\":\"hello.txt\"}"
                    }
                }]})),
                Some("during tools".to_string()),
            ),
            (
                response(json!({"role":"assistant","content":"initial final"})),
                Some("during final".to_string()),
            ),
            (
                response(json!({"role":"assistant","content":"updated final"})),
                None,
            ),
        ],
    )
    .await;
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        child.run_child_with_client(
            Some(&client),
            &client,
            &spec.prompt,
            "test-model",
            None,
            Some(1024),
            false,
            Some(&spec),
        ),
    )
    .await
    .expect("child must not wait forever")
    .unwrap();
    assert_eq!(output, "updated final");
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["content"] == "during tools"));
    assert!(requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["content"] == "during final"));
    assert!(parent
        .background
        .send_message(&task.id, "late")
        .await
        .is_err());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn explore_child_rejects_write() {
    let (runner, root) = temp_runner();
    let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
    let child = runner.spawn_child_runner(&spec, 1);
    child.ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    let error = crate::native::tools::execute_tool(
        &child.ctx,
        "Write",
        r#"{"file_path":"hello.txt","content":"changed"}"#,
    )
    .await
    .expect_err("explore write");
    assert!(error.contains("只读规划模式禁止调用工具 Write"));
    assert_eq!(
        fs::read_to_string(root.join("hello.txt")).expect("read"),
        "hello world\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn explore_child_yolo_skips_bash_permission_but_blocks_write_tools() {
    let (runner, root) = temp_runner();
    let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
    let mut child = runner.spawn_child_runner(&spec, 1);
    child.ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    let wc =
        crate::native::tools::execute_tool(&child.ctx, "Bash", r#"{"command":"wc -l hello.txt"}"#)
            .await
            .expect("readonly bash");
    assert!(wc.contains('1') || wc.contains("hello"), "{wc}");
    let diff =
        crate::native::tools::execute_tool(&child.ctx, "Bash", r#"{"command":"git diff"}"#).await;
    let diff_text = match diff {
        Ok(output) => output,
        Err(error) => error,
    };
    assert!(
        !diff_text.contains("只读规划模式禁止调用工具 Bash"),
        "{diff_text}"
    );
    child.ctx.request_permission = Some(std::sync::Arc::new(|_, _| {
        panic!("yolo explore bash must not prompt")
    }));
    crate::native::tools::execute_tool(&child.ctx, "Bash", r#"{"command":"sort"}"#)
        .await
        .expect("yolo skips explore opaque bash");
    crate::native::tools::execute_tool(&child.ctx, "Bash", r#"{"command":"rm hello.txt"}"#)
        .await
        .expect("yolo skips explore rm");
    assert!(!root.join("hello.txt").exists());
    let write = crate::native::tools::execute_tool(
        &child.ctx,
        "Write",
        r#"{"file_path":"hello.txt","content":"changed"}"#,
    )
    .await
    .expect_err("explore write tool");
    assert!(write.contains("只读规划模式禁止调用工具 Write"), "{write}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_and_child_clients_inherit_session_scope() {
    let (runner, root) = temp_runner();
    let parent = ModelClient::new(crate::native::model::ModelClientConfig {
        protocol: "openai".to_string(),
        base_url: "http://127.0.0.1".to_string(),
        api_key: "sk-test".to_string(),
        extra_headers: std::collections::HashMap::new(),
        retry: crate::native::model::RetryConfig::none(),
        timeout: Duration::from_secs(1),
        network: crate::app::network_settings::NetworkSettings::default(),
        responses_continuation: crate::native::model::ResponsesContinuationMode::Auto,
    })
    .expect("client")
    .with_call_log_context(crate::native::model::call_log::CallLogContext::for_session(
        Some("ch-1".to_string()),
        Some("OpenAI".to_string()),
        Some("sess-1".to_string()),
        Some("emp-1".to_string()),
        Some("proj-ssh".to_string()),
        crate::native::model::call_log::CALL_KIND_CHAT,
        Some("ssh".to_string()),
    ));
    let compact = parent.clone_for_conversation().with_call_log_context(
        parent
            .call_log_context()
            .cloned()
            .unwrap_or_default()
            .with_call_kind(CALL_KIND_COMPACT),
    );
    let compact_ctx = compact.call_log_context().expect("compact context");
    assert_eq!(compact_ctx.call_kind.as_deref(), Some(CALL_KIND_COMPACT));
    assert_eq!(compact_ctx.session_id.as_deref(), Some("sess-1"));
    assert_eq!(compact_ctx.workspace_id.as_deref(), Some("proj-ssh"));
    assert_eq!(compact_ctx.execution_target.as_deref(), Some("ssh"));

    let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
    let child = runner.observe_child_client(Some(&parent), &parent, Some(&spec));
    let child_ctx = child.call_log_context().expect("child context");
    assert_eq!(child_ctx.call_kind.as_deref(), Some(CALL_KIND_SUBAGENT));
    assert_eq!(child_ctx.session_id.as_deref(), Some("sess-1"));
    assert_eq!(child_ctx.workspace_id.as_deref(), Some("proj-ssh"));
    assert_eq!(child_ctx.execution_target.as_deref(), Some("ssh"));
    assert_eq!(child_ctx.subagent_id.as_deref(), Some("explore"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn apply_pending_live_model_refreshes_turn_and_window() {
    use crate::native::live_model::{write_live_model, LiveModelSnapshot};
    let (mut runner, root) = temp_runner();
    runner.context_window.set_token_limit(64_000);
    let client = ModelClient::new(crate::native::model::ModelClientConfig {
        protocol: "openai".to_string(),
        base_url: "http://127.0.0.1".to_string(),
        api_key: "sk-test".to_string(),
        extra_headers: std::collections::HashMap::new(),
        retry: crate::native::model::RetryConfig::none(),
        timeout: Duration::from_secs(1),
        network: crate::app::network_settings::NetworkSettings::default(),
        responses_continuation: crate::native::model::ResponsesContinuationMode::Auto,
    })
    .expect("client");
    let slot = std::sync::Arc::new(std::sync::Mutex::new(LiveModelSnapshot {
        revision: 0,
        client: client.clone(),
        model: "old".to_string(),
        channel_id: "ch-old".to_string(),
        channel_name: "Old".to_string(),
        protocol: "openai".to_string(),
        lite_model: None,
        effort: Some("low".to_string()),
        max_output_tokens: None,
        thinking_enabled: false,
        context_tokens: Some(64_000),
        context_token_limit: 64_000,
        execution_target: None,
        hook_agent: None,
    }));
    runner.live_model = Some(slot.clone());
    assert!(runner.apply_pending_live_model().is_none());
    write_live_model(
        &slot,
        LiveModelSnapshot {
            revision: 0,
            client,
            model: "deepseek-v4-flash".to_string(),
            channel_id: "ch-new".to_string(),
            channel_name: "New".to_string(),
            protocol: "openai".to_string(),
            lite_model: Some("lite".to_string()),
            effort: Some("high".to_string()),
            max_output_tokens: Some(2048),
            thinking_enabled: true,
            context_tokens: Some(8_000),
            context_token_limit: 8_000,
            execution_target: None,
            hook_agent: None,
        },
    );
    let applied = runner.apply_pending_live_model().expect("applied");
    assert_eq!(applied.model, "deepseek-v4-flash");
    assert_eq!(runner.lite_model.as_deref(), Some("lite"));
    assert_eq!(runner.context_window.token_limit, 8_000);
    assert!(runner.apply_pending_live_model().is_none());
    let _ = fs::remove_dir_all(root);
}

fn completion_fixture(text: &str, reason: &str) -> Value {
    serde_json::json!({"choices":[{"finish_reason":reason,"message":{"content":text}}]})
}

async fn run_fixture_turn(
    runner: &mut AgentRunner,
    client: &ModelClient,
    child: bool,
) -> Result<String, String> {
    if child {
        runner
            .run_child_with_client(None, client, "go", "test", None, Some(1024), false, None)
            .await
    } else {
        runner
            .run_with_client(client, "go", "test", None, Some(1024), false, Vec::new())
            .await
    }
}

#[tokio::test]
async fn output_continuation_main_and_child_are_bounded_and_drop_partial_tools() {
    use serde_json::json;
    for child in [false, true] {
        for empty in [false, true] {
            let (mut runner, root) = temp_runner();
            if child {
                runner = runner
                    .spawn_child_runner(&parse_subagent_args(r#"{"prompt":"go"}"#).unwrap(), 1);
            }
            runner.ctx.hooks = vec![crate::db::models::NativeHook::shell(
                "stop",
                "stop",
                "",
                "printf called >> stop-called; printf '%s' '{\"continue\":true}'",
                5,
                true,
            )];
            let text = if empty { "" } else { "partial" };
            let partial = json!({"choices":[{"finish_reason":"length","message":{"content":text,"tool_calls":[{"id":"unfinished","function":{"name":"Write","arguments":"{"}}]}}]});
            let (client, server) = mock_child_model(
                runner.background.clone(),
                String::new(),
                vec![(partial, None); 4],
            )
            .await;
            let (tx, mut rx) = mpsc::unbounded_channel();
            runner.on_event = Some(tx);
            let result = tokio::time::timeout(
                Duration::from_secs(4),
                run_fixture_turn(&mut runner, &client, child),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(result.contains("[未完成]"));
            assert_eq!(result.matches("partial").count(), if empty { 0 } else { 4 });
            assert_eq!(server.await.unwrap().len(), 4);
            assert_eq!(runner.output_continuations, 3);
            assert_eq!(
                runner
                    .messages
                    .iter()
                    .filter(|m| m.role == Role::User)
                    .count(),
                1
            );
            assert!(runner
                .messages
                .iter()
                .all(|m| m.tool_calls.is_empty() && m.role != Role::Tool));
            assert!(
                !root.join("stop-called").exists(),
                "partial chunks must not run stop hooks"
            );
            assert!(!drain_events(&mut rx)
                .iter()
                .any(|line| line.starts_with("[USER_INPUT]")));
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[tokio::test]
async fn output_continuation_main_and_child_finish_on_third_additional_request() {
    for child in [false, true] {
        let (mut runner, root) = temp_runner();
        let responses = vec![
            (completion_fixture("a", "length"), None),
            (completion_fixture("b", "length"), None),
            (completion_fixture("c", "length"), None),
            (completion_fixture("d", "stop"), None),
        ];
        let (client, server) =
            mock_child_model(runner.background.clone(), String::new(), responses).await;
        assert_eq!(
            run_fixture_turn(&mut runner, &client, child).await.unwrap(),
            "abcd"
        );
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            requests[3]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["role"] == "user")
                .count(),
            1
        );
        runner.begin_user_turn("next", Vec::new()).await.unwrap();
        assert_eq!(runner.output_continuations, 0);
        assert!(runner.output_partial.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn output_continuation_does_not_replay_discarded_tools() {
    use serde_json::json;
    for child in [false, true] {
        let (mut runner, root) = temp_runner();
        runner.set_allowed_tools(&["Bash".to_string()]);
        let tool = json!({"id":"write-once","function":{"name":"Bash","arguments":r#"{"command":"printf x >> once.txt"}"#}});
        let partial = json!({"choices":[{"finish_reason":"length","message":{"content":"start","tool_calls":[tool.clone()]}}]});
        let complete = json!({"choices":[{"finish_reason":"tool_calls","message":{"content":"","tool_calls":[tool]}}]});
        let (client, server) = mock_child_model(
            runner.background.clone(),
            String::new(),
            vec![
                (partial, None),
                (complete, None),
                (completion_fixture("end", "stop"), None),
            ],
        )
        .await;
        assert_eq!(
            run_fixture_turn(&mut runner, &client, child).await.unwrap(),
            "end"
        );
        assert_eq!(fs::read_to_string(root.join("once.txt")).unwrap(), "x");
        assert_eq!(
            runner
                .messages
                .iter()
                .filter(|m| m.role == Role::Tool)
                .count(),
            1
        );
        assert_eq!(server.await.unwrap().len(), 3);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn output_continuation_cancellation_and_budget_prevent_another_call() {
    for child in [false, true] {
        for cancel in [false, true] {
            let (mut runner, root) = temp_runner();
            let mut partial = completion_fixture("kept", "length");
            if cancel {
                let flag = runner.ctx.cancel.clone();
                runner.on_checkpoint = Some(Arc::new(move |messages| {
                    let flag = flag.clone();
                    Box::pin(async move {
                        if messages.iter().any(|m| m.role == Role::Assistant) {
                            flag.cancel();
                        }
                        Ok(messages)
                    })
                }));
            } else {
                runner.set_rollout_budget_limit(100_000);
                partial["usage"] =
                    serde_json::json!({"prompt_tokens":100_000,"completion_tokens":1000});
            }
            let (mut client, server) = mock_child_model(
                runner.background.clone(),
                String::new(),
                vec![(partial, None)],
            )
            .await;
            if child {
                runner.depth = 1;
            }
            if cancel {
                let flag = runner.ctx.cancel.clone();
                client = client.with_call_log(Default::default(), Arc::new(move |_| flag.cancel()));
            }
            let result = run_fixture_turn(&mut runner, &client, child).await;
            if cancel {
                assert_eq!(result.unwrap_err(), "已取消");
            } else {
                assert!(result.unwrap().contains("[未完成]"));
            }
            assert_eq!(server.await.unwrap().len(), 1);
            assert!(runner.messages.iter().any(|m| m.content == "kept"));
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[tokio::test]
async fn output_continuation_is_not_triggered_by_complete_or_failed_responses() {
    for child in [false, true] {
        for reason in ["stop", "refusal", "future_reason"] {
            let (mut runner, root) = temp_runner();
            let (client, server) = mock_child_model(
                runner.background.clone(),
                String::new(),
                vec![(completion_fixture("done", reason), None)],
            )
            .await;
            assert_eq!(
                run_fixture_turn(&mut runner, &client, child).await.unwrap(),
                "done"
            );
            assert_eq!(server.await.unwrap().len(), 1);
            assert_eq!(runner.output_continuations, 0);
            fs::remove_dir_all(root).unwrap();
        }
        let (mut runner, root) = temp_runner();
        let (client, server) = mock_child_model(
            runner.background.clone(),
            String::new(),
            vec![(
                serde_json::json!({"error":{"code":"server_error","message":"failed"}}),
                None,
            )],
        )
        .await;
        assert!(run_fixture_turn(&mut runner, &client, child).await.is_err());
        assert_eq!(server.await.unwrap().len(), 1);
        assert_eq!(runner.output_continuations, 0);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn output_continuation_stop_hook_runs_only_for_completed_answer() {
    let (mut runner, root) = temp_runner();
    runner.ctx.hooks = vec![crate::db::models::NativeHook::shell(
        "stop",
        "stop",
        "*",
        "printf x >> stop-count; printf '%s' '{\"continue\":false}'",
        5,
        true,
    )];
    let (client, server) = mock_child_model(
        runner.background.clone(),
        String::new(),
        vec![
            (completion_fixture("a", "length"), None),
            (completion_fixture("b", "stop"), None),
        ],
    )
    .await;
    assert_eq!(
        run_fixture_turn(&mut runner, &client, false).await.unwrap(),
        "ab"
    );
    assert_eq!(fs::read_to_string(root.join("stop-count")).unwrap(), "x");
    assert_eq!(server.await.unwrap().len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn output_continuation_counter_survives_steering_and_compaction() {
    let (mut runner, root) = temp_runner();
    runner.begin_user_turn("initial", Vec::new()).await.unwrap();
    runner.output_continuations = 3;
    runner.output_partial = "preserved".into();
    for n in 0..20 {
        runner.messages.push(Message::user(format!("earlier {n}")));
        runner
            .messages
            .push(Message::assistant_text("old response"));
    }
    let (tx, rx) = mpsc::channel(8);
    runner.steer_rx = Some(Arc::new(Mutex::new(rx)));
    tx.send(NativeFollowup::input("steered")).await.unwrap();
    assert!(runner.inject_steer_messages());
    assert!(runner
        .run_compaction(None, CompactTrigger::Manual, None)
        .await
        .unwrap()
        .is_some());
    assert_eq!(runner.output_continuations, 3);
    assert!(runner.output_partial.is_empty());
    assert!(!runner.output_pending);
    runner
        .begin_user_turn("new turn", Vec::new())
        .await
        .unwrap();
    assert_eq!(runner.output_continuations, 0);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn output_continuation_context_limit_uses_reactive_compaction_without_resetting_counter() {
    for child in [false, true] {
        let (mut runner, root) = temp_runner();
        runner.begin_user_turn("go", Vec::new()).await.unwrap();
        for n in 0..20 {
            runner.messages.push(Message::user(format!("earlier {n}")));
            runner
                .messages
                .push(Message::assistant_text("old response"));
        }
        runner.output_continuations = 2;
        runner.model_turn = Some(ModelTurnCfg {
            model: "test".into(),
            effort: None,
            max_output_tokens: Some(1024),
            thinking_enabled: false,
        });
        if child {
            runner.depth = 1;
        }
        let summary = "User goal\nContinue task\nConstraints\nPreserve details\nCompleted work\nReviewed history\nPending work\nFinish answer";
        let (client, server) = mock_child_model(
            runner.background.clone(),
            String::new(),
            vec![(completion_fixture(summary, "stop"), None)],
        )
        .await;
        assert!(matches!(
            runner
                .consume_partial(
                    Message::assistant_text("partial"),
                    FinishReason::ContextLimit,
                    &client
                )
                .await
                .unwrap(),
            TurnControl::Continue
        ));
        assert_eq!(runner.reactive_compactions, 1);
        assert_eq!(runner.output_continuations, 2);
        assert_eq!(server.await.unwrap().len(), 1);
        assert!(runner.output_partial.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn compaction_output_limit_is_not_automatically_continued_or_accepted() {
    let (mut runner, root) = temp_runner();
    runner.model_turn = Some(ModelTurnCfg {
        model: "test".into(),
        effort: None,
        max_output_tokens: Some(1024),
        thinking_enabled: false,
    });
    let (client, server) = mock_child_model(
        runner.background.clone(),
        String::new(),
        vec![(
            completion_fixture(
                "User goal\nPartial summary\nPending work\nunfinished",
                "length",
            ),
            None,
        )],
    )
    .await;
    assert!(runner
        .request_compaction_summary(&client, &[Message::user("summarize")])
        .await
        .is_none());
    assert_eq!(runner.output_continuations, 0);
    assert_eq!(server.await.unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn invalid_last_tool_prevents_execution_of_first_tool_in_actual_loops() {
    use serde_json::json;
    for child in [false, true] {
        let (mut runner, root) = temp_runner();
        let response = json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[
            {"id":"valid","function":{"name":"Bash","arguments":r#"{"command":"printf x > should-not-exist"}"#}},
            {"id":"invalid","function":{"name":"Write","arguments":"{"}}
        ]}}]});
        let (client, server) = mock_child_model(
            runner.background.clone(),
            String::new(),
            vec![(response, None)],
        )
        .await;
        assert!(run_fixture_turn(&mut runner, &client, child).await.is_err());
        assert!(!root.join("should-not-exist").exists());
        assert!(runner.messages.iter().all(|m| m.role != Role::Tool));
        assert_eq!(server.await.unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn output_recovery_reminder_is_transient_to_pending_request() {
    for child in [false, true] {
        let (mut runner, root) = temp_runner();
        let (client, server) = mock_child_model(
            runner.background.clone(),
            String::new(),
            vec![
                (completion_fixture("a", "length"), None),
                (completion_fixture("b", "stop"), None),
                (completion_fixture("c", "stop"), None),
            ],
        )
        .await;
        assert_eq!(
            run_fixture_turn(&mut runner, &client, child).await.unwrap(),
            "ab"
        );
        assert!(!runner
            .messages
            .iter()
            .any(|m| m.role == Role::System && m.content.contains("上一条模型输出达到")));
        assert_eq!(
            run_fixture_turn(&mut runner, &client, child).await.unwrap(),
            "c"
        );
        let requests = server.await.unwrap();
        assert!(requests[1].to_string().contains("上一条模型输出达到"));
        assert!(!requests[2].to_string().contains("上一条模型输出达到"));
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn completed_recovery_does_not_prefix_a_later_stop_hook_answer() {
    let (mut runner, root) = temp_runner();
    runner.ctx.hooks = vec![crate::db::models::NativeHook::shell("stop", "stop", "*", "printf '%s\\n' \"$NATIVE_HOOK_PAYLOAD\" >> hook-payloads; if [ -f hook-called ]; then printf '%s' '{\"continue\":false}'; else touch hook-called; printf '%s' '{\"continue\":true}'; fi", 5, true)];
    let (client, server) = mock_child_model(
        runner.background.clone(),
        String::new(),
        vec![
            (completion_fixture("a", "length"), None),
            (completion_fixture("b", "stop"), None),
            (completion_fixture("c", "stop"), None),
        ],
    )
    .await;
    assert_eq!(
        run_fixture_turn(&mut runner, &client, false).await.unwrap(),
        "c"
    );
    let payloads = fs::read_to_string(root.join("hook-payloads")).unwrap();
    let payloads = payloads
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        payloads
            .iter()
            .map(|value| value["final_text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["ab", "c"]
    );
    assert_eq!(runner.output_continuations, 1);
    let requests = server.await.unwrap();
    assert!(!requests[2].to_string().contains("上一条模型输出达到"));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn output_identity_is_shared_only_until_the_model_response_is_complete() {
    for child in [false, true] {
        let (mut runner, root) = temp_runner();
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        let middle = serde_json::json!({"choices":[{"finish_reason":"tool_calls","message":{"content":"b","tool_calls":[{"id":"read","function":{"name":"Read","arguments":r#"{"file_path":"missing"}"#}}]}}]});
        let (client, server) = mock_child_model(
            runner.background.clone(),
            String::new(),
            vec![
                (completion_fixture("a", "length"), None),
                (middle, None),
                (completion_fixture("c", "stop"), None),
            ],
        )
        .await;
        assert_eq!(
            run_fixture_turn(&mut runner, &client, child).await.unwrap(),
            "c"
        );
        let mut fragments = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let NativeEvent::Assistant { text, fragment } = event {
                fragments.push((text, fragment));
            }
        }
        assert_eq!(
            fragments
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(fragments[0].1.chain_id, fragments[1].1.chain_id);
        assert_eq!((fragments[0].1.part, fragments[1].1.part), (0, 1));
        assert_ne!(fragments[1].1.chain_id, fragments[2].1.chain_id);
        assert_eq!(runner.output_continuations, 1);
        assert!(runner.output_partial.is_empty());
        assert_eq!(server.await.unwrap().len(), 3);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn budget_fallback_distinguishes_pending_output_from_used_continuation_allowance() {
    let (mut runner, root) = temp_runner();
    runner.output_continuations = 1;
    runner.output_pending = true;
    runner.output_partial = "a".to_string();
    assert_eq!(runner.complete_output_chain("b"), "ab");
    assert_eq!(runner.output_continuations, 1);
    assert_eq!(runner.finish_without_model().unwrap(), LAST_TURN_FALLBACK);
    assert_eq!(runner.output_continuations, 1);

    runner.output_pending = true;
    runner.output_partial = "pending".to_string();
    let incomplete = runner.finish_without_model().unwrap();
    assert!(incomplete.starts_with("pending"));
    assert!(incomplete.contains("[未完成]"));
    assert_eq!(runner.output_continuations, 1);
    fs::remove_dir_all(root).unwrap();
}
fn attach_user_mailbox(runner: &mut AgentRunner) -> Arc<crate::native::steer::SteerMailbox> {
    let mailbox = Arc::new(crate::native::steer::SteerMailbox::new(
        "session", "instance",
    ));
    mailbox.configure(Arc::new(|_| Box::pin(async { Ok(()) })), Arc::new(|_| {}));
    runner.ctx.user_steer = Some(mailbox.clone());
    mailbox
}

#[tokio::test]
async fn user_steer_during_model_response_skips_old_tools_and_prevents_final_seal() {
    for tools in [false, true] {
        let (mut runner, root) = temp_runner();
        let mailbox = attach_user_mailbox(&mut runner);
        let steer_mailbox = mailbox.clone();
        let before: BeforeModelResponse = Arc::new(move |index| {
            let mailbox = steer_mailbox.clone();
            Box::pin(async move {
                if index == 0 {
                    let turn = mailbox.snapshot().await.turn_id.unwrap();
                    mailbox
                        .accept(
                            &turn,
                            &uuid::Uuid::new_v4().to_string(),
                            "new instruction",
                            &[],
                            vec![],
                        )
                        .await
                        .unwrap();
                }
            })
        });
        let first = if tools {
            serde_json::json!({"choices":[{"finish_reason":"tool_calls","message":{"content":"old proposal","tool_calls":[{"id":"write","function":{"name":"Write","arguments":"{\"file_path\":\"forbidden.txt\",\"content\":\"old action\"}"}}]}}]})
        } else {
            completion_fixture("old answer", "stop")
        };
        let (client, server) = mock_model_before_response(
            runner.background.clone(),
            "".into(),
            vec![
                (first, None),
                (completion_fixture("new answer", "stop"), None),
            ],
            Some(before),
        )
        .await;
        let (tx, mut events) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        let answer = run_fixture_turn(&mut runner, &client, false).await.unwrap();
        assert_eq!(answer, "new answer");
        assert!(!root.join("forbidden.txt").exists());
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "user" && message["content"] == "new instruction"));
        if tools {
            assert!(runner
                .messages
                .iter()
                .any(|message| message.role == Role::Tool
                    && message.tool_call_id == "write"
                    && message.content.contains(crate::native::steer::SUPERSEDED)));
        }
        let snapshot = mailbox.snapshot().await;
        assert!(snapshot.turn_id.is_none());
        assert_eq!(
            snapshot.receipts[0].status,
            crate::native::steer::SteerStatus::Applied
        );
        assert!(!drain_events(&mut events)
            .iter()
            .any(|line| line.contains("[USER_INPUT] new instruction")));
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn user_steer_between_serial_tools_preserves_completed_result_and_recovery_budget() {
    let (mut runner, root) = temp_runner();
    let mailbox = attach_user_mailbox(&mut runner);
    runner.begin_user_turn("go", vec![]).await.unwrap();
    let mut hook =
        crate::db::models::NativeHook::shell("steer", "post_tool_use", "Write", "", 5, true);
    hook.handler_type = "agent".into();
    hook.agent_prompt = Some("steer".into());
    runner.ctx.hooks = vec![hook];
    let hook_mailbox = mailbox.clone();
    runner.ctx.hook_agent = Some(Arc::new(move |_, _| {
        let mailbox = hook_mailbox.clone();
        Box::pin(async move {
            let turn = mailbox.snapshot().await.turn_id.unwrap();
            mailbox
                .accept(
                    &turn,
                    &uuid::Uuid::new_v4().to_string(),
                    "changed",
                    &[],
                    vec![],
                )
                .await
                .unwrap();
            Ok("{}".into())
        })
    }));
    runner.output_continuations = 2;
    runner.output_pending = true;
    runner.output_partial = "partial".into();
    runner
        .consume_assistant(
            assistant_tool_calls(&[
                (
                    "first",
                    "Write",
                    r#"{"file_path":"first.txt","content":"done"}"#,
                ),
                (
                    "second",
                    "Write",
                    r#"{"file_path":"second.txt","content":"never"}"#,
                ),
            ]),
            false,
            None,
        )
        .await
        .unwrap();
    assert!(root.join("first.txt").exists());
    assert!(!root.join("second.txt").exists());
    assert!(runner.messages.iter().any(|m| m.role == Role::Tool
        && m.tool_call_id == "first"
        && !m.content.contains(crate::native::steer::SUPERSEDED)));
    assert!(runner.messages.iter().any(|m| m.role == Role::Tool
        && m.tool_call_id == "second"
        && m.content.contains(crate::native::steer::SUPERSEDED)));
    assert!(runner.inject_user_steer().await.unwrap());
    assert_eq!(runner.output_continuations, 2);
    assert!(!runner.output_pending);
    assert!(runner.output_partial.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn user_steer_rejected_by_hook_is_terminal_and_claimed_only_once() {
    let (mut runner, root) = temp_runner();
    let mailbox = attach_user_mailbox(&mut runner);
    runner.begin_user_turn("go", vec![]).await.unwrap();
    let mut hook =
        crate::db::models::NativeHook::shell("deny", "user_prompt_submit", "*", "", 5, true);
    hook.handler_type = "agent".into();
    hook.agent_prompt = Some("deny".into());
    runner.ctx.hooks = vec![hook];
    let invoked = Arc::new(AtomicU32::new(0));
    let counter = invoked.clone();
    runner.ctx.hook_agent = Some(Arc::new(move |_, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(r#"{"decision":"block","reason":"test rejection"}"#.into()) })
    }));
    let turn = mailbox.snapshot().await.turn_id.unwrap();
    mailbox
        .accept(
            &turn,
            &uuid::Uuid::new_v4().to_string(),
            "rejected",
            &[],
            vec![],
        )
        .await
        .unwrap();
    assert!(runner.inject_user_steer().await.unwrap());
    assert!(!runner.inject_user_steer().await.unwrap());
    assert_eq!(invoked.load(Ordering::SeqCst), 1);
    assert!(!runner.messages.iter().any(|m| m.content == "rejected"));
    assert_eq!(
        mailbox.snapshot().await.receipts[0].status,
        crate::native::steer::SteerStatus::Rejected
    );
    assert!(runner.seal_user_turn().await.unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn user_steer_accepted_inside_stop_hook_survives_final_boundary() {
    let (mut runner, root) = temp_runner();
    let mailbox = attach_user_mailbox(&mut runner);
    let mut hook = crate::db::models::NativeHook::shell("stop", "stop", "*", "", 5, true);
    hook.handler_type = "agent".into();
    hook.agent_prompt = Some("stop".into());
    runner.ctx.hooks = vec![hook];
    let hook_mailbox = mailbox.clone();
    let calls = Arc::new(AtomicU32::new(0));
    let hook_calls = calls.clone();
    runner.ctx.hook_agent = Some(Arc::new(move |_, _| {
        let mailbox = hook_mailbox.clone();
        let count = hook_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if count == 0 {
                let turn = mailbox.snapshot().await.turn_id.unwrap();
                mailbox
                    .accept(
                        &turn,
                        &uuid::Uuid::new_v4().to_string(),
                        "after hook",
                        &[],
                        vec![],
                    )
                    .await
                    .unwrap();
            }
            Ok(r#"{"continue":false}"#.into())
        })
    }));
    let (client, server) = mock_child_model(
        runner.background.clone(),
        "".into(),
        vec![
            (completion_fixture("old", "stop"), None),
            (completion_fixture("new", "stop"), None),
        ],
    )
    .await;
    assert_eq!(
        run_fixture_turn(&mut runner, &client, false).await.unwrap(),
        "new"
    );
    assert_eq!(server.await.unwrap().len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(mailbox.snapshot().await.turn_id.is_none());
    fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn bootstrap_steer_survives_begin_user_turn_without_adopting_unconsumed_generation() {
    let (mut runner, root) = temp_runner();
    let mailbox = attach_user_mailbox(&mut runner);
    runner.ctx.main_origin = Some(mailbox.begin_turn().await);
    let initial = runner.ctx.main_origin.clone();
    let turn = mailbox.snapshot().await.turn_id.unwrap();
    mailbox
        .accept(
            &turn,
            &uuid::Uuid::new_v4().to_string(),
            "during bootstrap authorization",
            &[],
            vec![],
        )
        .await
        .unwrap();
    runner.begin_user_turn("original", vec![]).await.unwrap();
    assert_eq!(runner.ctx.main_origin, initial);
    assert!(!runner.ctx.execution_current());
    assert_eq!(
        mailbox.snapshot().await.turn_id.as_deref(),
        Some(turn.as_str())
    );
    assert!(runner.inject_user_steer().await.unwrap());
    assert!(runner.ctx.execution_current());
    assert_eq!(
        runner.messages.last().unwrap().content,
        "during bootstrap authorization"
    );
    fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn user_steer_preserves_started_parallel_results_and_skips_remaining_serial_tools() {
    let (mut runner, root) = temp_runner();
    fs::write(root.join("second.txt"), "second result").unwrap();
    let mailbox = attach_user_mailbox(&mut runner);
    runner.begin_user_turn("go", vec![]).await.unwrap();
    let turn = mailbox.snapshot().await.turn_id.unwrap();
    let mut hook =
        crate::db::models::NativeHook::shell("parallel", "post_tool_use", "Read", "", 5, true);
    hook.handler_type = "agent".into();
    hook.agent_prompt = Some("wait".into());
    runner.ctx.hooks = vec![hook];
    let entered = Arc::new(tokio::sync::Barrier::new(3));
    let release = Arc::new(Semaphore::new(0));
    let hook_entered = entered.clone();
    let hook_release = release.clone();
    runner.ctx.hook_agent = Some(Arc::new(move |_, _| {
        let entered = hook_entered.clone();
        let release = hook_release.clone();
        Box::pin(async move {
            entered.wait().await;
            let _permit = release.acquire().await.unwrap();
            Ok("{}".into())
        })
    }));
    let running = tokio::spawn(async move {
        runner
            .consume_assistant(
                assistant_tool_calls(&[
                    ("read-1", "Read", r#"{"file_path":"hello.txt"}"#),
                    ("read-2", "Read", r#"{"file_path":"second.txt"}"#),
                    (
                        "write",
                        "Write",
                        r#"{"file_path":"forbidden.txt","content":"old"}"#,
                    ),
                ]),
                false,
                None,
            )
            .await
            .unwrap();
        runner
    });
    tokio::time::timeout(Duration::from_secs(3), entered.wait())
        .await
        .unwrap();
    mailbox
        .accept(
            &turn,
            &uuid::Uuid::new_v4().to_string(),
            "new direction",
            &[],
            vec![],
        )
        .await
        .unwrap();
    release.add_permits(2);
    let mut runner = tokio::time::timeout(Duration::from_secs(3), running)
        .await
        .unwrap()
        .unwrap();
    for (id, expected) in [
        ("read-1", "hello world"),
        ("read-2", "second result"),
        ("write", crate::native::steer::SUPERSEDED),
    ] {
        assert!(runner
            .messages
            .iter()
            .any(|message| message.role == Role::Tool
                && message.tool_call_id == id
                && message.content.contains(expected)));
    }
    assert!(!root.join("forbidden.txt").exists());
    assert!(!runner.ctx.cancel.is_cancelled());
    assert!(runner.inject_user_steer().await.unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn user_steer_does_not_abort_a_running_command() {
    let (mut runner, root) = temp_runner();
    let mailbox = attach_user_mailbox(&mut runner);
    runner.begin_user_turn("go", vec![]).await.unwrap();
    let turn = mailbox.snapshot().await.turn_id.unwrap();
    let running = tokio::spawn(async move {
        runner.consume_assistant(assistant_tool_calls(&[
                ("running", "Bash", r#"{"command":"touch ready; while [ ! -f release ]; do sleep 0.01; done; printf completed"}"#),
                ("unstarted", "Write", r#"{"file_path":"forbidden.txt","content":"old"}"#),
            ]), false, None).await.unwrap();
        runner
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.join("ready").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    mailbox
        .accept(
            &turn,
            &uuid::Uuid::new_v4().to_string(),
            "steer without killing",
            &[],
            vec![],
        )
        .await
        .unwrap();
    fs::write(root.join("release"), "release").unwrap();
    let runner = tokio::time::timeout(Duration::from_secs(3), running)
        .await
        .unwrap()
        .unwrap();
    assert!(runner
        .messages
        .iter()
        .any(|message| message.role == Role::Tool
            && message.tool_call_id == "running"
            && message.content.contains("completed")));
    assert!(!root.join("forbidden.txt").exists());
    assert!(!runner.ctx.cancel.is_cancelled());
    fs::remove_dir_all(root).unwrap();
}
