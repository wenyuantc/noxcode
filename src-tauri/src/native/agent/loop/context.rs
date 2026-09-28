use super::*;

impl AgentRunner {
    /// `/compact [指令]`：下一次模型调用前压缩。
    pub fn request_manual_compaction(&mut self, instructions: Option<String>) {
        self.pending_manual_compact =
            Some(NativeCompactionRequest::new(instructions, Arc::default()));
    }

    /// 会话恢复到更小窗口模型时调用；只有超阈值才真正压缩。
    pub fn request_downshift_compaction(&mut self) {
        self.pending_downshift_compact = true;
    }

    pub fn set_compaction_options(&mut self, threshold_percent: u32, microcompact_enabled: bool) {
        self.context_window
            .set_threshold_percent(threshold_percent as usize);
        self.microcompact_enabled = microcompact_enabled;
    }

    /// 等待输入状态下的 `/compact`：立刻压缩并写边界记录。
    pub async fn compact_now(
        &mut self,
        client: &ModelClient,
        instructions: Option<String>,
    ) -> Result<Option<CompactBoundary>, String> {
        self.compact_now_with(client, CompactTrigger::Manual, instructions)
            .await
    }

    /// 立刻压缩；模型切换降窗口时使用 `Downshift`。
    pub async fn compact_now_with(
        &mut self,
        client: &ModelClient,
        trigger: CompactTrigger,
        instructions: Option<String>,
    ) -> Result<Option<CompactBoundary>, String> {
        let client = self.observe_client(client);
        self.run_compaction(Some(&client), trigger, instructions)
            .await
    }

    /// 统一的压缩入口：微压缩 → 模型摘要 → 本地摘要 → 重置。返回边界记录（已写入事件流）。
    pub(super) async fn run_compaction(
        &mut self,
        client: Option<&ModelClient>,
        trigger: CompactTrigger,
        instructions: Option<String>,
    ) -> Result<Option<CompactBoundary>, String> {
        let pre_tokens = total_message_tokens(&self.messages);
        let pre_messages = self.messages.len();
        let mut source: Option<&str> = None;
        // 自动 / 降级 / 被动触发时，先试便宜的微压缩；用户明确 /compact 时直接做摘要。
        if trigger != CompactTrigger::Manual && self.microcompact_enabled {
            let replaced = microcompact(&mut self.messages);
            if replaced > 0 && !self.context_window.should_compact(&self.messages) {
                source = Some("microcompact");
            }
        }
        if source.is_none() {
            let mut compacted = false;
            if let Some(client) = client {
                compacted = self
                    .compact_with_model(client, instructions.as_deref())
                    .await;
                if compacted {
                    source = Some("model");
                }
            }
            if !compacted && compact_local(&mut self.messages) {
                source = Some("local");
                compacted = true;
            }
            if !compacted && reset_local(&mut self.messages) {
                source = Some("reset");
                compacted = true;
            }
            if !compacted {
                return Ok(None);
            }
        }
        let source = source.unwrap_or("local");
        if source == "reset" {
            self.context_window.mark_reset();
        } else {
            self.context_window.mark_compacted();
        }
        let boundary = CompactBoundary {
            trigger,
            source: source.to_string(),
            pre_tokens,
            post_tokens: total_message_tokens(&self.messages),
            pre_messages,
            post_messages: self.messages.len(),
            instructions: instructions.filter(|item| !item.trim().is_empty()),
        };
        let label = match source {
            "microcompact" => "已微压缩旧工具结果",
            "model" => "已压缩上下文（模型摘要）",
            "local" => "已压缩上下文（本地摘要）",
            _ => "已重置上下文窗口（保留当前任务）",
        };
        self.emit(format!(
            "[工具] {label}：{} → {} token（{}）",
            boundary.pre_tokens,
            boundary.post_tokens,
            boundary.trigger.as_str()
        ));
        self.emit(boundary.line());
        self.emit_context_usage();
        self.checkpoint_transcript().await?;
        Ok(Some(boundary))
    }

    pub(super) async fn checkpoint_transcript(&mut self) -> Result<(), String> {
        if self.depth > 0 {
            return Ok(());
        }
        let Some(hook) = self.on_checkpoint.clone() else {
            return Ok(());
        };
        let stamped = hook(self.messages.clone()).await?;
        apply_history_ids(&mut self.messages, &stamped);
        Ok(())
    }

    pub fn set_rollout_budget(&mut self, budget: Arc<RolloutBudget>) {
        self.release_model_reservation();
        self.rollout_budget = budget;
        self.budget_exhausted = false;
    }

    pub fn set_rollout_budget_limit(&mut self, limit: u64) {
        self.release_model_reservation();
        self.rollout_budget = RolloutBudget::shared(limit);
        self.budget_exhausted = false;
    }

    pub fn budget_snapshot(&self) -> BudgetSnapshot {
        self.rollout_budget.snapshot()
    }

    pub fn diagnostics_snapshot(&self) -> AgentDiagnosticsSnapshot {
        self.diagnostics.snapshot()
    }

    pub(super) fn emit_usage(&mut self, usage: crate::native::model::types::Usage) {
        let Some(delta) = usage_to_delta(usage) else {
            return;
        };
        if let Some(line) = delta.format_terminal_line() {
            self.emit(line);
        }
        if let Some(tx) = &self.on_usage {
            let _ = tx.send(delta);
        }
        self.last_usage = Some(usage);
        self.emit_context_usage();
    }

    pub(super) fn settle_model_usage(&mut self, usage: Usage, assistant: Option<&Message>) {
        let reserved = std::mem::take(&mut self.pending_budget_reservation);
        let reported =
            u64::from(usage.prompt_tokens).saturating_add(u64::from(usage.completion_tokens));
        let estimated = assistant
            .map(|message| message_tokens(message) as u64)
            .unwrap_or(0);
        let actual = if reported > 0 {
            reported
        } else {
            estimated.max(reserved)
        };
        let actual_opt = (actual > 0).then_some(actual);
        if reserved > 0 {
            self.rollout_budget.settle(reserved, actual_opt);
        } else {
            self.rollout_budget.record_usage(usage);
        }
        self.settle_child_reservation(if reserved > 0 { actual_opt } else { None });
        self.budget_exhausted = self.rollout_budget.is_exhausted();
    }

    pub(super) fn release_model_reservation(&mut self) {
        let reserved = std::mem::take(&mut self.pending_budget_reservation);
        if reserved > 0 {
            self.rollout_budget.release(reserved);
        }
        self.release_child_reservation();
    }

    fn try_reserve_child(&mut self, tokens: u64) -> bool {
        let Some(quota) = &self.child_quota else {
            return true;
        };
        if quota.try_reserve(tokens) {
            self.pending_child_reservation = tokens;
            true
        } else {
            false
        }
    }

    fn settle_child_reservation(&mut self, actual: Option<u64>) {
        let reserved = std::mem::take(&mut self.pending_child_reservation);
        if reserved == 0 {
            return;
        }
        if let Some(quota) = &self.child_quota {
            quota.settle(reserved, actual);
        }
    }

    fn release_child_reservation(&mut self) {
        let reserved = std::mem::take(&mut self.pending_child_reservation);
        if reserved == 0 {
            return;
        }
        if let Some(quota) = &self.child_quota {
            quota.release(reserved);
        }
    }

    /// 供应商报上下文溢出时被动压缩再重试；连续两次仍溢出则放弃。
    pub(super) async fn try_reactive_compaction(
        &mut self,
        client: &ModelClient,
        error: &ModelError,
    ) -> Result<bool, String> {
        if error.kind != ModelErrorKind::ContextLimit || self.reactive_compactions >= 2 {
            return Ok(false);
        }
        self.reactive_compactions += 1;
        self.emit(format!(
            "[工具] 模型报上下文溢出，尝试被动压缩后重试（{}/2）：{error}",
            self.reactive_compactions
        ));
        // 被动压缩不再依赖阈值判断，直接做全量摘要。
        let boundary = self
            .run_compaction(Some(client), CompactTrigger::Reactive, None)
            .await?;
        if boundary.is_none() {
            self.emit("[工具] 被动压缩无法再缩减上下文，停止重试");
        }
        Ok(boundary.is_some())
    }

    pub(super) async fn prepare_model_call(
        &mut self,
        client: Option<&ModelClient>,
    ) -> Result<bool, String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        if self.max_turns > 0 && self.turns >= self.max_turns {
            return Err("达到最大模型轮次".to_string());
        }
        self.turns += 1;
        self.sync_context_window();
        // 后台任务完成 / 子 Agent 留言：在模型看到下一次请求前提醒。
        if self.depth == 0 {
            if let Some(notice) = self.background.pending_notice() {
                self.emit(notice.clone());
                append_user_note(&mut self.messages, &notice);
            }
        }
        if let Some(mut request) = self.pending_manual_compact.take() {
            if self
                .run_compaction(client, CompactTrigger::Manual, request.instructions.take())
                .await?
                .is_none()
            {
                self.emit("[工具] 当前上下文太短，无需压缩");
            }
        } else if self.context_window.should_compact(&self.messages) {
            let trigger = if std::mem::take(&mut self.pending_downshift_compact) {
                CompactTrigger::Downshift
            } else {
                CompactTrigger::Auto
            };
            self.run_compaction(client, trigger, None).await?;
        } else {
            self.pending_downshift_compact = false;
        }
        let tool_context_tokens = total_tool_tokens(&self.combined_tools());
        // Tool schemas are part of the request context too. If MCP/schema
        // definitions alone consume the entire configured window, sending
        // them would guarantee an oversized request. Fall back to a final
        // tool-free turn and retain the full window for the answer.
        let tools_fit = tool_context_tokens < self.context_window.token_limit;
        let message_context_limit = if tools_fit {
            self.context_window
                .token_limit
                .saturating_sub(tool_context_tokens)
                .max(1)
        } else {
            self.context_window.token_limit.max(1)
        };
        truncate_messages_tokens(
            &mut self.messages,
            message_context_limit,
            self.tool_result_token_limit,
        );
        // A provider may return a very large assistant message after local
        // compaction. Try a second reset before sending an oversized request.
        if total_message_tokens(&self.messages) > message_context_limit
            && reset_local(&mut self.messages)
        {
            self.context_window.mark_reset();
            truncate_messages_tokens(
                &mut self.messages,
                message_context_limit,
                self.tool_result_token_limit,
            );
            self.emit("[工具] 已重置上下文窗口（超出 token 上限）");
        }
        let budget_stop = self.budget_exhausted || self.rollout_budget.is_exhausted();
        if !tools_fit {
            self.emit("[工具] 工具定义已超过上下文窗口，停止调用工具并直接作答");
        }
        if budget_stop {
            let newly_exhausted = !self.budget_exhausted;
            self.budget_exhausted = true;
            if newly_exhausted {
                self.diagnostics.budget_stops.fetch_add(1, Ordering::AcqRel);
            }
            self.emit("[工具] rollout token 预算已用尽，停止调用工具并直接作答");
        }
        let last_turn =
            !tools_fit || budget_stop || (self.max_turns > 0 && self.turns >= self.max_turns);
        if last_turn {
            if !budget_stop {
                self.emit(format!(
                    "[工具] 第 {}/{} 轮，停止调用工具并直接作答",
                    self.turns, self.max_turns
                ));
            }
            append_last_turn_reminder(&mut self.messages);
        }
        self.emit_context_usage();
        Ok(last_turn)
    }

    async fn compact_with_model(
        &mut self,
        client: &ModelClient,
        instructions: Option<&str>,
    ) -> bool {
        let Some(summary_messages) =
            compaction_prompt_with_instructions(&self.messages, instructions)
        else {
            return false;
        };
        let Some(summary) = self
            .request_compaction_summary(client, &summary_messages)
            .await
        else {
            return false;
        };
        if apply_usable_summary(&mut self.messages, &summary) {
            return true;
        }
        let mut retry_messages = summary_messages;
        retry_messages.push(Message::assistant_text(summary.content.clone()));
        retry_messages.push(Message::user(
            "Previous summary is unusable. Regenerate a factual structured handoff that includes at least two of these headings: User goal, Constraints, Completed work, Verification, Pending work.",
        ));
        if let Some(retry) = self
            .request_compaction_summary(client, &retry_messages)
            .await
        {
            if apply_usable_summary(&mut self.messages, &retry) {
                return true;
            }
            if retry.content.trim().is_empty() {
                self.emit("[工具] 模型摘要为空，改用本地摘要");
            } else {
                self.emit("[工具] 模型摘要不可用，改用本地摘要");
            }
        }
        false
    }

    pub(super) async fn request_compaction_summary(
        &mut self,
        client: &ModelClient,
        messages: &[Message],
    ) -> Option<Message> {
        let summary_limit = 1_024u64;
        let request_tokens = total_message_tokens(messages) as u64;
        let reservation = request_tokens.saturating_add(summary_limit);
        if !self.rollout_budget.try_reserve(reservation) {
            self.emit("[工具] 上下文压缩预算不足，改用本地摘要");
            return None;
        }
        self.pending_budget_reservation = reservation;
        // 配置了轻量模型时用它做摘要，更便宜也更快。
        let main_model = self
            .model_turn
            .as_ref()
            .map(|turn| turn.model.clone())
            .unwrap_or_default();
        let (summary_model, model_role) = match self.lite_model.as_deref() {
            Some(lite) if !lite.trim().is_empty() && lite != main_model => {
                (lite.to_string(), MODEL_ROLE_LITE)
            }
            _ => (main_model, MODEL_ROLE_MAIN),
        };
        let compact_client = match client.call_log_context() {
            Some(context) => client.clone_for_conversation().with_call_log_context(
                context
                    .clone()
                    .with_call_kind(CALL_KIND_COMPACT)
                    .with_operation(OPERATION_COMPACT)
                    .with_model_role(model_role),
            ),
            None => client.clone_for_conversation(),
        };
        let result = compact_client
            .chat(ChatRequest {
                messages,
                tools: &[],
                model: &summary_model,
                effort: None,
                max_output_tokens: Some(summary_limit as u32),
                thinking_enabled: false,
            })
            .await;
        match result {
            Ok(response) => {
                self.settle_model_usage(response.usage, Some(&response.message));
                self.emit_usage(response.usage);
                response.complete_message().ok()
            }
            Err(error) => {
                self.release_model_reservation();
                self.emit(format!("[工具] 模型摘要失败，改用本地摘要：{error}"));
                None
            }
        }
    }

    fn sync_context_window(&mut self) {
        let configured = chars_to_tokens(self.context_char_limit).max(1);
        // Older callers only set `context_char_limit`; newer session wiring
        // sets the token field explicitly. Do not overwrite an explicit token
        // window when both fields are present.
        if self.context_char_limit != DEFAULT_CONTEXT_CHARS
            && self.context_window.token_limit
                == crate::native::settings::DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as usize
        {
            self.context_window.set_token_limit(configured);
        }
    }

    pub(super) fn reserve_model_call(
        &mut self,
        max_output_tokens: Option<u32>,
        tools: &[ToolSpec],
    ) -> Option<ModelCallBudget> {
        if self.pending_budget_reservation > 0 {
            self.release_model_reservation();
        }
        let input_tokens = total_message_tokens(&self.model_request_messages()) as u64;
        let requested_output_tokens =
            u64::from(max_output_tokens.unwrap_or(DEFAULT_FINAL_OUTPUT_RESERVE as u32));
        let tool_tokens = total_tool_tokens(tools) as u64;
        let fixed_tokens = input_tokens.saturating_add(tool_tokens);
        if self.rollout_budget.limit() == 0 {
            // Unlimited budgets track usage for diagnostics but never rewrite
            // the caller's provider settings.
            let estimate = fixed_tokens.saturating_add(requested_output_tokens);
            if !self.try_reserve_child(estimate) {
                self.mark_budget_stop();
                return None;
            }
            self.rollout_budget.try_reserve(estimate);
            self.pending_budget_reservation = estimate;
            return Some(ModelCallBudget { max_output_tokens });
        }
        let mut available = self.rollout_budget.remaining().saturating_sub(fixed_tokens);
        if let Some(quota) = &self.child_quota {
            available = available.min(quota.remaining().saturating_sub(fixed_tokens));
        }
        if available == 0 {
            self.mark_budget_stop();
            return None;
        }
        // The output reserve is only an estimate that `settle_model_usage`
        // replaces with actual usage. A finite budget always sends a hard
        // request cap so a provider that omits usage cannot overrun remaining
        // tokens. Unlimited rollouts still leave the caller's setting alone.
        let requested_output_tokens = match max_output_tokens {
            Some(value) => u64::from(value),
            None => FALLBACK_OUTPUT_TOKEN_GUARD,
        };
        let reserve_output_tokens = requested_output_tokens.min(available);
        let request_cap = Some(reserve_output_tokens as u32);
        let estimate = fixed_tokens.saturating_add(reserve_output_tokens);
        if !self.try_reserve_child(estimate) {
            self.mark_budget_stop();
            return None;
        }
        if self.rollout_budget.try_reserve(estimate) {
            self.pending_budget_reservation = estimate;
            Some(ModelCallBudget {
                max_output_tokens: request_cap,
            })
        } else {
            self.release_child_reservation();
            self.mark_budget_stop();
            None
        }
    }

    fn mark_budget_stop(&mut self) {
        let newly_exhausted = !self.budget_exhausted;
        self.budget_exhausted = true;
        if newly_exhausted {
            self.diagnostics.budget_stops.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(super) fn append_budget_reminder(&mut self) {
        self.emit("[工具] rollout token 预算不足，当前请求仅允许直接作答");
        append_last_turn_reminder(&mut self.messages);
    }

    pub(super) fn finish_without_model(&mut self) -> Result<String, String> {
        self.release_model_reservation();
        if self.output_pending {
            return Ok(self.finish_incomplete_output());
        }
        self.emit(LAST_TURN_FALLBACK.to_string());
        Ok(LAST_TURN_FALLBACK.to_string())
    }
}

fn apply_usable_summary(messages: &mut Vec<Message>, summary: &Message) -> bool {
    summary.tool_calls.is_empty()
        && is_usable_compaction_summary(&summary.content)
        && compact_with_summary(messages, summary.content.trim())
}

fn append_last_turn_reminder(messages: &mut Vec<Message>) {
    append_user_note(messages, LAST_TURN_REMINDER);
}

/// 把提示追加到最后一条用户 / 工具消息末尾（保持角色交替），否则新起一条用户消息。
fn append_user_note(messages: &mut Vec<Message>, note: &str) {
    if let Some(last) = messages.last_mut() {
        if last.role == Role::User || last.role == Role::Tool {
            if !last.content.is_empty() {
                last.content.push_str("\n\n");
            }
            last.content.push_str(note);
            return;
        }
    }
    messages.push(Message::user(note.to_string()));
}

fn apply_history_ids(target: &mut [Message], stamped: &[Message]) {
    if target.len() != stamped.len() {
        return;
    }
    for (target, stamped) in target.iter_mut().zip(stamped.iter()) {
        if target.history_id.is_empty() && !stamped.history_id.is_empty() {
            target.history_id = stamped.history_id.clone();
        }
    }
}
