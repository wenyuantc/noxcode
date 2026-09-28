use super::*;

impl AgentRunner {
    pub fn new(workspace: LocalWorkspace) -> Self {
        let background = Arc::new(BackgroundTaskRegistry::new());
        let mut ctx = ToolCtx::new(workspace);
        ctx.background = Some(background.clone());
        Self {
            ctx,
            background,
            messages: Vec::new(),
            max_turns: DEFAULT_NATIVE_MAX_TURNS as u32,
            max_subagent_turns: crate::native::settings::DEFAULT_NATIVE_MAX_SUBAGENT_TURNS as u32,
            max_concurrent_subagents: crate::native::agent::subagent::MAX_CONCURRENT_SUBAGENTS
                as u32,
            subagent_semaphore: None,
            subagent_policy: crate::native::settings::DEFAULT_NATIVE_SUBAGENT_POLICY.to_string(),
            context_char_limit: DEFAULT_CONTEXT_CHARS,
            rollout_budget: RolloutBudget::shared(DEFAULT_ROLLOUT_TOKEN_BUDGET),
            child_quota: None,
            subagent_budget_share_percent:
                crate::native::settings::DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT as u32,
            steer_rx: None,
            context_window: ContextWindow::new(
                crate::native::settings::DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as usize,
            ),
            tool_result_token_limit: DEFAULT_TOOL_RESULT_TOKEN_LIMIT,
            diagnostics: Arc::new(AgentDiagnostics::default()),
            on_event: None,
            on_usage: None,
            on_activity: None,
            on_checkpoint: None,
            subagent_stub: None,
            custom_subagents: Vec::new(),
            reload_custom_subagents: None,
            child_model_loader: None,
            workspace_context: String::new(),
            project_agents: String::new(),
            required_subagent_type: None,
            extra_tools: Vec::new(),
            extra_tool_contracts: Vec::new(),
            artifacts: None,
            lite_model: None,
            skills_prompt: String::new(),
            last_usage: None,
            allowed_tools: None,
            disallowed_tools: None,
            turns: 0,
            last_tool_key: None,
            last_tool_repeat: 0,
            stop_hook_continues: 0,
            output_continuations: 0,
            output_partial: String::new(),
            output_pending: false,
            output_fragment: None,
            pending_manual_compact: None,
            turn_suffix: None,
            pending_downshift_compact: false,
            microcompact_enabled: true,
            reactive_compactions: 0,
            depth: 0,
            event_prefix: String::new(),
            subagent_seq: 0,
            model_turn: None,
            pending_budget_reservation: 0,
            pending_child_reservation: 0,
            pending_steer_finish: false,
            budget_exhausted: false,
            streaming: false,
            call_started_ms: Arc::new(AtomicU64::new(0)),
            reasoning_started_ms: Arc::new(AtomicU64::new(0)),
            started_tool_ids: HashSet::new(),
            tool_started_ms: HashMap::new(),
            tool_seq: 0,
            live_model: None,
            live_model_revision: 0,
            recovery: None,
            pending_recovery: Vec::new(),
        }
    }

    pub fn note_live_model_revision(&mut self, revision: u64) {
        self.live_model_revision = revision;
    }

    pub fn apply_pending_live_model(&mut self) -> Option<LiveModelSnapshot> {
        let slot = self.live_model.as_ref()?;
        let next = slot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if next.revision <= self.live_model_revision {
            return None;
        }
        self.live_model_revision = next.revision;
        self.model_turn = Some(ModelTurnCfg {
            model: next.model.clone(),
            effort: next.effort.clone(),
            max_output_tokens: next.max_output_tokens,
            thinking_enabled: next.thinking_enabled,
        });
        self.lite_model = next.lite_model.clone();
        self.ctx.hook_agent = next.hook_agent.clone();
        if let Some(scope) = self.ctx.session_scope.as_mut() {
            scope.channel_id = next.channel_id.clone();
            scope.model = next.model.clone();
        }
        if next.context_token_limit > 0
            && next.context_token_limit != self.context_window.token_limit
        {
            if next.context_token_limit < self.context_window.token_limit {
                self.request_downshift_compaction();
            }
            self.context_char_limit = next.context_token_limit.saturating_mul(2);
            self.context_window
                .set_token_limit(next.context_token_limit);
        }
        Some(next)
    }

    pub fn cancel(&self) {
        self.ctx.cancel.cancel();
    }

    pub fn set_extra_tools(&mut self, tools: Vec<ToolSpec>) {
        self.extra_tools = tools;
    }

    /// 注册动态工具（MCP）的契约，用于并行判定与结果预算。
    pub fn set_extra_tool_contracts(&mut self, contracts: Vec<ToolContract>) {
        self.extra_tool_contracts = contracts;
    }

    pub fn set_artifact_store(&mut self, store: Arc<ArtifactStore>) {
        self.artifacts = Some(store);
    }

    pub fn set_allowed_tools<S: AsRef<str>>(&mut self, names: &[S]) {
        self.allowed_tools = Some(names.iter().map(|name| name.as_ref().to_string()).collect());
    }

    /// 子 Agent 档案的 `disallowedTools`：从可见工具里剔除。
    pub fn set_disallowed_tools<S: AsRef<str>>(&mut self, names: &[S]) {
        self.disallowed_tools = Some(names.iter().map(|name| name.as_ref().to_string()).collect());
    }

    pub(super) fn contract_registry(&self) -> ContractRegistry {
        let mut registry = ContractRegistry::new(tool_contracts());
        registry.extend(self.extra_tool_contracts.iter().cloned());
        registry
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.ctx.set_read_only(read_only);
    }

    pub fn set_plan_mode(&mut self, plan_mode: bool) {
        self.ctx.set_plan_mode(plan_mode);
    }

    pub fn is_plan_mode(&self) -> bool {
        self.ctx.is_plan_mode()
    }

    /// 给下一条用户消息附加文本（记忆回忆）；不影响事件流里的 `[USER_INPUT]` 行。
    pub fn set_turn_suffix(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.turn_suffix = if text.trim().is_empty() {
            None
        } else {
            Some(text)
        };
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_with_client(
        &mut self,
        client: &ModelClient,
        user: &str,
        model: &str,
        effort: Option<&str>,
        max_output_tokens: Option<u32>,
        thinking_enabled: bool,
        images: Vec<NativeImage>,
    ) -> Result<String, String> {
        self.model_turn = Some(ModelTurnCfg {
            model: model.to_string(),
            effort: effort.map(ToOwned::to_owned),
            max_output_tokens,
            thinking_enabled,
        });
        self.begin_user_turn(user, images).await?;
        self.checkpoint_transcript().await?;
        let mut client = self.observe_client(client);
        let mut model = model.to_string();
        let mut effort = effort.map(ToOwned::to_owned);
        let mut max_output_tokens = max_output_tokens;
        let mut thinking_enabled = thinking_enabled;
        self.streaming = true;
        loop {
            if let Some(live) = self.apply_pending_live_model() {
                client = self.observe_client(&live.client);
                model = live.model;
                effort = live.effort;
                max_output_tokens = live.max_output_tokens;
                thinking_enabled = live.thinking_enabled;
            }
            self.inject_user_steer().await?;
            if self.inject_steer_messages() {
                self.checkpoint_transcript().await?;
            }
            let mut last_turn = self.prepare_model_call(Some(&client)).await?;
            if self.pending_steer_finish {
                last_turn = true;
            }
            let tools = self.combined_tools();
            let mut tools_now: &[ToolSpec] = if last_turn { &[] } else { &tools };
            let call_budget =
                if let Some(budget) = self.reserve_model_call(max_output_tokens, tools_now) {
                    budget
                } else {
                    // A request with tools may not fit the remaining shared
                    // budget. Retry once as a tool-free final answer, then stop
                    // locally if even that request cannot be reserved.
                    if !last_turn {
                        last_turn = true;
                        self.append_budget_reminder();
                    }
                    tools_now = &[];
                    let Some(budget) = self.reserve_model_call(max_output_tokens, tools_now) else {
                        if let Some(mailbox) = &self.ctx.user_steer {
                            mailbox.cancel("本回合预算已用尽；补充指令未应用").await?;
                        }
                        return self.finish_without_model();
                    };
                    budget
                };
            self.begin_model_call();
            let request_messages = self.model_request_messages();
            let result = client
                .chat(ChatRequest {
                    messages: &request_messages,
                    tools: tools_now,
                    model: &model,
                    effort: effort.as_deref(),
                    max_output_tokens: call_budget.max_output_tokens,
                    thinking_enabled,
                })
                .await;
            let ModelResponse {
                message: assistant,
                usage,
                finish_reason,
                ..
            } = match result {
                Ok(value) => value,
                Err(error) => {
                    self.release_model_reservation();
                    self.emit_delta_clear();
                    if self.try_reactive_compaction(&client, &error).await? {
                        continue;
                    }
                    return Err(error.into());
                }
            };
            let requested_with_tools = !tools_now.is_empty();
            self.settle_model_usage(usage, Some(&assistant));
            self.emit_usage(usage);
            if matches!(
                finish_reason,
                FinishReason::OutputLimit | FinishReason::ContextLimit
            ) {
                match self
                    .consume_partial(assistant, finish_reason, &client)
                    .await?
                {
                    TurnControl::Continue => continue,
                    TurnControl::Stop(text) => {
                        if self.seal_user_turn().await? {
                            return Ok(text);
                        } else {
                            continue;
                        }
                    }
                }
            }
            last_turn = last_turn_after_response(
                last_turn,
                requested_with_tools,
                &assistant,
                self.budget_exhausted,
            );
            match self
                .consume_assistant(assistant, last_turn, Some(&client))
                .await?
            {
                TurnControl::Stop(text) => {
                    if self.seal_user_turn().await? {
                        return Ok(text);
                    } else {
                        continue;
                    }
                }
                TurnControl::Continue => {}
            }
        }
    }

    /// Commit useful partial text without ever publishing an unfinished tool pair.
    /// This counter belongs to the user turn, so compaction and steering cannot reset it.
    pub(super) async fn consume_partial(
        &mut self,
        mut assistant: Message,
        reason: FinishReason,
        client: &ModelClient,
    ) -> Result<TurnControl, String> {
        assistant.tool_calls.clear();
        if let Some(line) = thinking_start_line(
            &assistant.reasoning_content,
            self.thinking_elapsed_seconds(),
        ) {
            self.emit(line);
        }
        if !assistant.content.is_empty() {
            self.emit_assistant_text(&assistant.content);
            self.output_partial.push_str(&assistant.content);
        }
        self.messages.push(assistant);
        self.emit_delta_clear();
        self.checkpoint_transcript().await?;
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        if reason == FinishReason::ContextLimit {
            self.clear_output_recovery();
            let error = ModelError::new(ModelErrorKind::ContextLimit, "模型上下文已达上限");
            if self.try_reactive_compaction(client, &error).await? {
                return Ok(TurnControl::Continue);
            }
            return Err(error.into());
        }
        if self.output_continuations >= 3
            || self.budget_exhausted
            || self.rollout_budget.is_exhausted()
            || self
                .child_quota
                .as_ref()
                .is_some_and(|quota| quota.remaining() == 0)
        {
            let text = self.finish_incomplete_output();
            self.checkpoint_transcript().await?;
            return Ok(TurnControl::Stop(text));
        }
        self.output_continuations += 1;
        self.output_pending = true;
        Ok(TurnControl::Continue)
    }

    pub(super) fn finish_incomplete_output(&mut self) -> String {
        let notice = "[未完成] 模型输出达到上限，已停止自动续接。已保留此前输出。";
        self.emit(notice);
        self.messages.push(Message::assistant_text(notice));
        let text = format!("{}\n\n{notice}", self.output_partial);
        self.clear_output_recovery();
        text
    }

    pub(super) fn model_request_messages(&self) -> Vec<Message> {
        let mut messages = self.messages.clone();
        let limits = crate::native::media_plan::budget_for_model(
            self.model_turn
                .as_ref()
                .map(|cfg| cfg.model.as_str())
                .unwrap_or(""),
        );
        let mut notices = crate::native::media_plan::shrink_message_images(&mut messages, &limits);
        let (degraded, degraded_notices) =
            crate::native::media_plan::degrade_history_media(messages, limits.max_request_bytes);
        messages = degraded;
        notices.extend(degraded_notices);
        for notice in notices {
            self.emit(&notice);
        }
        if self.output_pending {
            messages.push(Message::system(OUTPUT_RECOVERY_REMINDER));
        }
        messages
    }

    pub(super) fn clear_output_recovery(&mut self) {
        self.output_pending = false;
        self.output_partial.clear();
        self.output_fragment = None;
    }

    fn emit_assistant_text(&self, text: &str) {
        if let (Some(tx), Some(fragment)) = (&self.on_event, &self.output_fragment) {
            let _ = tx.send(NativeEvent::Assistant {
                text: text.to_string(),
                fragment: fragment.clone(),
            });
        } else {
            self.emit(text);
        }
    }

    pub(super) fn complete_output_chain(&mut self, suffix: &str) -> String {
        let text = format!("{}{suffix}", self.output_partial);
        self.clear_output_recovery();
        text
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_child_with_client(
        &mut self,
        parent: Option<&ModelClient>,
        client: &ModelClient,
        user: &str,
        model: &str,
        effort: Option<&str>,
        max_output_tokens: Option<u32>,
        thinking_enabled: bool,
        spec: Option<&SubagentSpec>,
    ) -> Result<String, String> {
        self.model_turn = Some(ModelTurnCfg {
            model: model.to_string(),
            effort: effort.map(ToOwned::to_owned),
            max_output_tokens,
            thinking_enabled,
        });
        self.begin_user_turn(user, Vec::new()).await?;
        self.checkpoint_transcript().await?;
        let client = self.observe_child_client(parent, client, spec);
        loop {
            if self.inject_steer_messages() {
                self.checkpoint_transcript().await?;
            }
            let mut last_turn = self.prepare_model_call(Some(&client)).await?;
            if self.pending_steer_finish {
                last_turn = true;
            }
            let tools = self.combined_tools();
            let mut tools_now: &[ToolSpec] = if last_turn { &[] } else { &tools };
            let call_budget =
                if let Some(budget) = self.reserve_model_call(max_output_tokens, tools_now) {
                    budget
                } else {
                    if !last_turn {
                        last_turn = true;
                        self.append_budget_reminder();
                    }
                    tools_now = &[];
                    let Some(budget) = self.reserve_model_call(max_output_tokens, tools_now) else {
                        return self.finish_without_model();
                    };
                    budget
                };
            self.begin_model_call();
            let request_messages = self.model_request_messages();
            let result = client
                .chat(ChatRequest {
                    messages: &request_messages,
                    tools: tools_now,
                    model,
                    effort,
                    max_output_tokens: call_budget.max_output_tokens,
                    thinking_enabled,
                })
                .await;
            let ModelResponse {
                message: assistant,
                usage,
                finish_reason,
                ..
            } = match result {
                Ok(value) => value,
                Err(error) => {
                    self.release_model_reservation();
                    if self.try_reactive_compaction(&client, &error).await? {
                        continue;
                    }
                    return Err(error.into());
                }
            };
            let requested_with_tools = !tools_now.is_empty();
            self.settle_model_usage(usage, Some(&assistant));
            self.emit_usage(usage);
            if matches!(
                finish_reason,
                FinishReason::OutputLimit | FinishReason::ContextLimit
            ) {
                match self
                    .consume_partial(assistant, finish_reason, &client)
                    .await?
                {
                    TurnControl::Continue => continue,
                    TurnControl::Stop(text) => return Ok(text),
                }
            }
            last_turn = last_turn_after_response(
                last_turn,
                requested_with_tools,
                &assistant,
                self.budget_exhausted,
            );
            match self
                .consume_assistant_serial(assistant, last_turn, Some(&client))
                .await?
            {
                TurnControl::Stop(text) => {
                    if self.inject_steer_messages() {
                        self.checkpoint_transcript().await?;
                        continue;
                    }
                    if !self.seal_background_messages() {
                        continue;
                    }
                    return Ok(text);
                }
                TurnControl::Continue => {}
            }
        }
    }

    fn seal_background_messages(&self) -> bool {
        let (Some((registry, task_id)), Some(receiver)) = (&self.ctx.coordinator, &self.steer_rx)
        else {
            return true;
        };
        let Ok(receiver) = receiver.try_lock() else {
            return false;
        };
        registry.seal_messages_if_empty(task_id, &receiver)
    }

    pub async fn run_scripted(
        &mut self,
        user: &str,
        replies: Vec<Message>,
    ) -> Result<String, String> {
        self.begin_user_turn(user, Vec::new()).await?;
        self.checkpoint_transcript().await?;
        let mut queue = VecDeque::from(replies);
        loop {
            let mut last_turn = self.prepare_model_call(None).await?;
            let tools = self.combined_tools();
            let mut tools_now: &[ToolSpec] = if last_turn { &[] } else { &tools };
            if self.reserve_model_call(None, tools_now).is_none() {
                if !last_turn {
                    last_turn = true;
                    self.append_budget_reminder();
                    tools_now = &[];
                    if self.reserve_model_call(None, tools_now).is_none() {
                        return self.finish_without_model();
                    }
                } else {
                    return self.finish_without_model();
                }
            }
            let requested_with_tools = !tools_now.is_empty();
            let Some(assistant) = queue.pop_front() else {
                self.release_model_reservation();
                return Err("scripted model exhausted".to_string());
            };
            self.settle_model_usage(Usage::default(), Some(&assistant));
            last_turn = last_turn_after_response(
                last_turn,
                requested_with_tools,
                &assistant,
                self.budget_exhausted,
            );
            match self.consume_assistant(assistant, last_turn, None).await? {
                TurnControl::Stop(text) => return Ok(text),
                TurnControl::Continue => {}
            }
        }
    }

    pub(super) async fn begin_user_turn(
        &mut self,
        user: &str,
        images: Vec<NativeImage>,
    ) -> Result<(), String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        if let Some(mailbox) = &self.ctx.user_steer {
            self.ctx.main_origin = Some(mailbox.begin_turn().await);
        }
        self.turns = 0;
        self.last_tool_key = None;
        self.last_tool_repeat = 0;
        self.stop_hook_continues = 0;
        self.output_continuations = 0;
        self.clear_output_recovery();
        self.reactive_compactions = 0;
        self.drain_pending_recovery().await?;
        if let Some(recovery) = &self.recovery {
            if recovery.attempts_exhausted().await? {
                recovery.complete_turn().await?;
            }
        }
        self.bind_recovery_turn().await;
        let mut text = user.to_string();
        if let Some(suffix) = self.turn_suffix.take() {
            text = format!("{text}\n\n{suffix}");
        }
        if self.depth == 0 && !self.ctx.hooks.is_empty() {
            match run_user_prompt_submit_hooks(&self.ctx.hook_runtime(), user).await {
                Ok(context) if !context.is_empty() => {
                    self.emit("[钩子] user_prompt_submit 注入了附加上下文");
                    text = format!("{text}\n\n[钩子上下文]\n{}", context.join("\n"));
                }
                Ok(_) => {}
                Err(reason) => return Err(format!("输入被钩子阻断：{reason}")),
            }
        }
        self.messages.push(Message::user_with_images(text, images));
        Ok(())
    }

    /// 回合结束前询问 stop 钩子；要求继续时把理由作为用户消息追加，最多 3 次。
    async fn stop_hooks_want_continue(&mut self, final_text: &str) -> bool {
        if self.depth > 0 || self.ctx.hooks.is_empty() {
            return false;
        }
        if self.stop_hook_continues >= MAX_STOP_HOOK_CONTINUES {
            return false;
        }
        let result = run_stop_hooks(&self.ctx.hook_runtime(), final_text).await;
        for warning in &result.warnings {
            self.emit(format!("[钩子警告] {warning}"));
        }
        let Some(reason) = result.continue_reason else {
            return false;
        };
        self.stop_hook_continues += 1;
        self.emit(format!(
            "[钩子] stop 钩子要求继续（{}/{}）：{reason}",
            self.stop_hook_continues, MAX_STOP_HOOK_CONTINUES
        ));
        self.messages
            .push(Message::user(format!("[Stop 钩子要求继续] {reason}")));
        true
    }

    pub(super) async fn consume_assistant(
        &mut self,
        mut assistant: Message,
        last_turn: bool,
        client: Option<&ModelClient>,
    ) -> Result<TurnControl, String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        assistant
            .tool_calls
            .retain(|call| !call.name.trim().is_empty());
        if last_turn {
            assistant.tool_calls.clear();
        }
        // Keep live fragments until these lines arrive so the transcript does not blink.
        if let Some(line) = thinking_start_line(
            &assistant.reasoning_content,
            self.thinking_elapsed_seconds(),
        ) {
            self.emit(line);
        }
        let text = assistant.content.clone();
        let tool_calls = assistant.tool_calls.clone();
        if !text.is_empty() {
            self.emit_assistant_text(&text);
        }
        self.messages.push(assistant);
        self.emit_delta_clear();
        let text = self.complete_output_chain(&text);
        if tool_calls.is_empty() {
            if self.inject_user_steer().await? {
                return Ok(TurnControl::Continue);
            }
            let text = if text.trim().is_empty() && last_turn {
                self.emit(LAST_TURN_FALLBACK.to_string());
                LAST_TURN_FALLBACK.to_string()
            } else {
                text
            };
            if !last_turn && self.stop_hooks_want_continue(&text).await {
                self.checkpoint_transcript().await?;
                return Ok(TurnControl::Continue);
            }
            self.checkpoint_transcript().await?;
            return Ok(TurnControl::Stop(text));
        }
        self.execute_tool_calls(tool_calls, client).await?;
        self.checkpoint_transcript().await?;
        if self.goal_verification_paused().await {
            return Ok(TurnControl::Stop(
                "目标核验次数已用完，已暂停自动继续".to_string(),
            ));
        }
        Ok(TurnControl::Continue)
    }

    async fn consume_assistant_serial(
        &mut self,
        mut assistant: Message,
        last_turn: bool,
        client: Option<&ModelClient>,
    ) -> Result<TurnControl, String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        assistant
            .tool_calls
            .retain(|call| !call.name.trim().is_empty());
        if last_turn {
            assistant.tool_calls.clear();
        }
        if let Some(line) = thinking_start_line(
            &assistant.reasoning_content,
            self.thinking_elapsed_seconds(),
        ) {
            self.emit(line);
        }
        let text = assistant.content.clone();
        let mut tool_calls = assistant.tool_calls.clone();
        if !text.is_empty() {
            self.emit_assistant_text(&text);
        }
        self.messages.push(assistant);
        self.emit_delta_clear();
        let text = self.complete_output_chain(&text);
        if tool_calls.is_empty() {
            let text = if text.trim().is_empty() && last_turn {
                self.emit(LAST_TURN_FALLBACK.to_string());
                LAST_TURN_FALLBACK.to_string()
            } else {
                text
            };
            return Ok(TurnControl::Stop(text));
        }
        self.install_goal_reviewer(client);
        for call in &mut tool_calls {
            self.assign_call_id(call);
        }
        self.persist_tool_plan(&tool_calls).await?;
        for call in tool_calls {
            if self.ctx.cancel.is_cancelled() {
                return Err("已取消".to_string());
            }
            if call.name == "Agent" {
                let output = self.reject_nested_agent(&call).await;
                self.push_tool_output(&call, output).await?;
                continue;
            }
            self.emit_tool_start(&call).await?;
            let output = self.execute_logged_tool(&call).await;
            self.record_tool_result(&call, output).await?;
        }
        if self.goal_verification_paused().await {
            return Ok(TurnControl::Stop(
                "目标核验次数已用完，已暂停自动继续".to_string(),
            ));
        }
        Ok(TurnControl::Continue)
    }
}

fn last_turn_after_response(
    planned_last_turn: bool,
    requested_with_tools: bool,
    assistant: &Message,
    budget_exhausted: bool,
) -> bool {
    let honor_tools = requested_with_tools
        && assistant
            .tool_calls
            .iter()
            .any(|call| !call.name.trim().is_empty());
    (planned_last_turn || budget_exhausted) && !honor_tools
}
