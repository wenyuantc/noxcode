use super::*;

impl AgentRunner {
    pub(super) fn emit(&self, line: impl Into<String>) {
        Self::send_prefixed_event(&self.on_event, &self.event_prefix, line);
    }

    pub(super) fn send_prefixed_event(
        on_event: &Option<mpsc::UnboundedSender<NativeEvent>>,
        prefix: &str,
        line: impl Into<String>,
    ) {
        if let Some(tx) = on_event {
            let line = line.into();
            let _ = tx.send(NativeEvent::Line(if prefix.is_empty() {
                line
            } else {
                format!("{prefix}{line}")
            }));
        }
    }

    fn subagent_tag(&self) -> Option<String> {
        let tag = self.event_prefix.trim();
        if tag.is_empty() {
            None
        } else {
            Some(tag.to_string())
        }
    }

    pub(super) fn assign_call_id(&mut self, call: &mut ToolCall) {
        if call.id.trim().is_empty() {
            self.tool_seq = self.tool_seq.saturating_add(1);
            call.id = format!("anon-{}", self.tool_seq);
        }
    }

    fn send_tool(&self, line: String, event: NativeToolEvent, images: Vec<NativeImage>) {
        self.send_tool_prefixed(&self.event_prefix, line, event, images);
    }

    fn send_tool_prefixed(
        &self,
        prefix: &str,
        line: String,
        event: NativeToolEvent,
        images: Vec<NativeImage>,
    ) {
        if let Some(tx) = &self.on_event {
            let line = if prefix.is_empty() {
                line
            } else {
                format!("{prefix}{line}")
            };
            let _ = tx.send(NativeEvent::Tool {
                line,
                event,
                images,
            });
        }
    }

    fn tool_event_prefix(&self, tag_override: Option<&str>) -> (String, Option<String>) {
        match tag_override.map(str::trim).filter(|tag| !tag.is_empty()) {
            Some(tag) => (format!("{tag} "), Some(tag.to_string())),
            None => (self.event_prefix.clone(), self.subagent_tag()),
        }
    }

    async fn mcp_display(&self, name: &str) -> (Option<String>, Option<String>) {
        match self.ctx.mcp.display_for_tool(name).await {
            Some((server, tool)) => (Some(server), Some(tool)),
            None => (None, None),
        }
    }

    pub(super) async fn emit_tool_start(&mut self, call: &ToolCall) -> Result<(), String> {
        self.emit_tool_start_with_tag(call, None).await
    }

    pub(super) async fn emit_tool_start_with_tag(
        &mut self,
        call: &ToolCall,
        tag_override: Option<&str>,
    ) -> Result<(), String> {
        self.ledger_started(&call.id).await?;
        if self.started_tool_ids.contains(&call.id) {
            return Ok(());
        }
        let (mcp_server, mcp_tool) = self.mcp_display(&call.name).await;
        let line = tool_start_line_ex(
            &call.name,
            &call.arguments,
            mcp_server.as_deref(),
            mcp_tool.as_deref(),
        );
        let title = tool_event_title(&line);
        let args_summary = tool_args_summary(&call.name, &call.arguments);
        let (prefix, subagent_tag) = self.tool_event_prefix(tag_override);
        self.started_tool_ids.insert(call.id.clone());
        self.tool_started_ms.insert(call.id.clone(), unix_now_ms());
        self.send_tool_prefixed(
            &prefix,
            line,
            NativeToolEvent {
                phase: NativeToolPhase::Start,
                call_id: call.id.clone(),
                name: call.name.clone(),
                title,
                args_summary,
                ok: None,
                duration_ms: None,
                result_preview: None,
                subagent_tag,
                mcp_server,
                mcp_tool,
                image_names: Vec::new(),
            },
            Vec::new(),
        );
        Ok(())
    }

    pub(super) async fn emit_tool_result(
        &mut self,
        call: &ToolCall,
        output: &ToolOutput,
    ) -> Result<(), String> {
        self.emit_tool_result_with_tag(call, output, None).await
    }

    pub(super) async fn emit_tool_result_with_tag(
        &mut self,
        call: &ToolCall,
        output: &ToolOutput,
        tag_override: Option<&str>,
    ) -> Result<(), String> {
        if !self.started_tool_ids.contains(&call.id) {
            self.emit_tool_start_with_tag(call, tag_override).await?;
        }
        let duration_ms = self
            .tool_started_ms
            .get(&call.id)
            .map(|start| unix_now_ms().saturating_sub(*start));
        let (mcp_server, mcp_tool) = self.mcp_display(&call.name).await;
        let start_line = tool_start_line_ex(
            &call.name,
            &call.arguments,
            mcp_server.as_deref(),
            mcp_tool.as_deref(),
        );
        let line = tool_result_line(&call.name, &output.text);
        let (prefix, subagent_tag) = self.tool_event_prefix(tag_override);
        self.send_tool_prefixed(
            &prefix,
            line,
            NativeToolEvent {
                phase: NativeToolPhase::Result,
                call_id: call.id.clone(),
                name: call.name.clone(),
                title: tool_event_title(&start_line),
                args_summary: tool_args_summary(&call.name, &call.arguments),
                ok: Some(output.ok),
                duration_ms,
                result_preview: Some(cap_tool_result_display(&output.text)),
                subagent_tag,
                mcp_server,
                mcp_tool,
                image_names: output
                    .images
                    .iter()
                    .map(|image| image.name.clone())
                    .collect(),
            },
            output.images.clone(),
        );
        Ok(())
    }

    /// Drop the fragments shown so far: either the complete line is about to
    /// arrive, or a retry is going to regenerate the answer.
    pub(super) fn emit_delta_clear(&self) {
        if !self.streaming {
            return;
        }
        if let Some(tx) = &self.on_event {
            let _ = tx.send(NativeEvent::Delta(StreamDelta::Reset));
        }
    }

    pub(super) fn begin_model_call(&mut self) {
        let fragment = if self.output_pending {
            let mut fragment = self
                .output_fragment
                .clone()
                .expect("pending output has a fragment");
            fragment.part += 1;
            fragment
        } else {
            NativeAssistantFragment {
                chain_id: crate::app::shared::new_id(),
                part: 0,
                subagent_tag: self.subagent_tag(),
            }
        };
        self.output_fragment = Some(fragment.clone());
        if self.streaming {
            if let Some(tx) = &self.on_event {
                let _ = tx.send(NativeEvent::ModelCall(fragment));
            }
        }
        self.call_started_ms.store(unix_now_ms(), Ordering::Relaxed);
        self.reasoning_started_ms.store(0, Ordering::Relaxed);
    }

    pub(super) fn thinking_elapsed_seconds(&self) -> u32 {
        let now = unix_now_ms();
        let reasoning = self.reasoning_started_ms.swap(0, Ordering::Relaxed);
        let call = self.call_started_ms.swap(0, Ordering::Relaxed);
        let start = if reasoning > 0 { reasoning } else { call };
        thinking_duration_seconds(now.saturating_sub(start))
    }

    pub(super) fn observe_client(&self, client: &ModelClient) -> ModelClient {
        let on_event = self.on_event.clone();
        let prefix = self.event_prefix.clone();
        let delta_events = self.on_event.clone();
        let reasoning_started_ms = self.reasoning_started_ms.clone();
        client
            .clone_for_conversation()
            .with_cancel(self.ctx.cancel.clone())
            .with_retry_hook(Arc::new(move |line: &str| {
                Self::send_prefixed_event(&on_event, &prefix, line);
            }))
            // Only the top-level runner streams: concurrent child agents would
            // interleave their fragments into one unreadable line.
            .with_delta_hook(Arc::new(move |delta: StreamDelta| {
                if matches!(&delta, StreamDelta::Reasoning(text) if !text.is_empty()) {
                    let _ = reasoning_started_ms.compare_exchange(
                        0,
                        unix_now_ms(),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );
                }
                if let Some(tx) = &delta_events {
                    let _ = tx.send(NativeEvent::Delta(delta));
                }
            }))
    }

    pub(super) fn observe_child_client(
        &self,
        parent: Option<&ModelClient>,
        client: &ModelClient,
        spec: Option<&SubagentSpec>,
    ) -> ModelClient {
        let on_event = self.on_event.clone();
        let prefix = self.event_prefix.clone();
        // Child agents must begin an independent Responses conversation even
        // when they inherit the parent's transport client.
        let mut observed = client
            .clone()
            .with_cancel(self.ctx.cancel.clone())
            .with_retry_hook(Arc::new(move |line: &str| {
                Self::send_prefixed_event(&on_event, &prefix, line);
            }));
        let parent_context = parent.and_then(ModelClient::call_log_context);
        let mut context = client
            .call_log_context()
            .cloned()
            .or_else(|| parent_context.cloned())
            .unwrap_or_default()
            .with_call_kind(CALL_KIND_SUBAGENT)
            .with_operation(OPERATION_SUBAGENT);
        if context.session_id.is_none() {
            context.session_id = parent_context.and_then(|item| item.session_id.clone());
        }
        if context.profile_id.is_none() {
            context.profile_id = parent_context.and_then(|item| item.profile_id.clone());
        }
        if context.workspace_id.is_none() {
            context.workspace_id = parent_context.and_then(|item| item.workspace_id.clone());
        }
        if context.execution_target.is_none() {
            context.execution_target =
                parent_context.and_then(|item| item.execution_target.clone());
        }
        if let Some(spec) = spec {
            context = context.with_subagent_id(spec.kind.as_str());
        }
        if let Some(sink) = client
            .call_log_sink()
            .or_else(|| parent.and_then(ModelClient::call_log_sink))
        {
            observed = observed.with_call_log(context, sink);
        } else {
            observed = observed.with_call_log_context(context);
        }
        observed
    }

    pub(super) fn emit_activity(&self, action: &str, details: &str) {
        if let Some(tx) = &self.on_activity {
            let _ = tx.send((action.to_string(), details.to_string()));
        }
    }

    pub(super) fn emit_context_usage(&self) {
        if self.depth != 0 {
            return;
        }
        if let Some(tx) = &self.on_event {
            let tools = self.combined_tools();
            let breakdown = context_usage_breakdown(&self.messages, &tools, &self.skills_prompt);
            let (estimated_tokens, from_provider) =
                self.estimated_context_tokens(total_tool_tokens(&tools));
            let (prompt_tokens, cached_tokens) = self
                .last_usage
                .map(|usage| (usage.prompt_tokens as usize, usage.cached_tokens as usize))
                .unwrap_or((0, 0));
            let _ = tx.send(NativeEvent::ContextUsage(ContextUsageSnapshot {
                used_tokens: breakdown.used_tokens,
                limit_tokens: self.context_window.token_limit,
                generation: self.context_window.generation,
                compactions: self.context_window.compactions,
                mcp_tokens: breakdown.mcp_tokens,
                system_tool_tokens: breakdown.system_tool_tokens,
                skill_tokens: breakdown.skill_tokens,
                system_prompt_tokens: breakdown.system_prompt_tokens,
                other_tokens: breakdown.other_tokens,
                message_tokens: breakdown.message_tokens,
                prompt_tokens,
                cached_tokens,
                estimated_tokens,
                estimate_source: if from_provider {
                    "provider"
                } else {
                    "estimate"
                },
                output_reserve_tokens: self.context_window.output_reserve,
            }));
        }
    }
}
