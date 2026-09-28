use super::*;

impl AgentRunner {
    pub(super) fn combined_tools(&self) -> Vec<ToolSpec> {
        let mut tools = tool_specs();
        if self.ctx.ssh.is_some() {
            tools.retain(|tool| tool.name != "SQLiteQuery");
        }
        let advertise_computer = self.ctx.computer_control_enabled
            && self.ctx.ssh.is_none()
            && !self.ctx.is_read_only()
            && !self.ctx.is_plan_mode()
            && self.depth == 0;
        if !advertise_computer {
            tools.retain(|tool| tool.name != "Computer");
        }
        let read_only = self.ctx.is_read_only();
        let plan_mode = self.ctx.is_plan_mode();
        if self.depth > 0 || (read_only && !plan_mode) {
            tools.retain(|tool| tool.name != "Agent");
        }
        // 子 Agent 没有用户交互通道：不给提问与计划模式工具；后台任务管理只给主 Agent。
        if self.depth > 0 {
            tools.retain(|tool| {
                !matches!(
                    tool.name.as_str(),
                    "AskUserQuestion"
                        | "EnterPlanMode"
                        | "ExitPlanMode"
                        | "TaskOutput"
                        | "TaskStop"
                        | "SendMessage"
                        | "CronCreate"
                        | "CronUpdate"
                        | "CronList"
                        | "CronDelete"
                        | "Goal"
                        | "GoalRead"
                )
            });
        }
        // RespondToCoordinator 只对后台子 Agent 可见。
        let is_background_child = self.ctx.coordinator.is_some();
        tools.retain(|tool| tool.name != "RespondToCoordinator" || is_background_child);
        if read_only {
            tools.retain(|tool| {
                !matches!(
                    tool.name.as_str(),
                    "TaskOutput" | "TaskStop" | "SendMessage"
                )
            });
        }
        // EnterPlanMode 只在执行模式可见，ExitPlanMode 只在计划模式可见。
        tools.retain(|tool| match tool.name.as_str() {
            "EnterPlanMode" => !plan_mode && !read_only,
            "ExitPlanMode" => plan_mode,
            _ => true,
        });
        let cap = self.max_concurrent_subagents.max(1);
        for tool in &mut tools {
            if tool.name == "Agent" {
                tool.description = crate::native::prompt::agent_tool_description(
                    cap,
                    &self.subagent_policy,
                    &self.custom_subagents,
                    self.required_subagent_type.as_deref(),
                    plan_mode,
                );
            }
        }
        tools.extend(self.extra_tools.clone());
        if let Some(allowed) = &self.allowed_tools {
            tools.retain(|tool| allowed.contains(&tool.name));
        }
        if let Some(disallowed) = &self.disallowed_tools {
            tools.retain(|tool| !disallowed.contains(&tool.name));
        }
        if read_only {
            tools.retain(|tool| {
                crate::native::tools::is_read_only_native_tool(&tool.name)
                    || tool.name == "Bash"
                    || (plan_mode && tool.name == "Agent")
            });
        }
        tools
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.combined_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }

    /// 连续相同参数的调用计数；达到上限时返回拒绝文案。
    pub(super) fn repeat_guard(&mut self, call: &ToolCall) -> Option<String> {
        let key = format!("{}\n{}", call.name, call.arguments);
        if self.last_tool_key.as_deref() == Some(key.as_str()) {
            self.last_tool_repeat = self.last_tool_repeat.saturating_add(1);
        } else {
            self.last_tool_key = Some(key);
            self.last_tool_repeat = 1;
        }
        if self.last_tool_repeat >= REPEAT_TOOL_LIMIT {
            return Some(format!(
                "重复调用被拒绝：你已用相同参数连续调用 {} {} 次。请改用其他工具或直接给出最终结论。",
                call.name, self.last_tool_repeat
            ));
        }
        None
    }

    pub(super) fn install_goal_reviewer(&mut self, client: Option<&ModelClient>) {
        let Some(client) = client else {
            self.ctx.goal_reviewer = None;
            return;
        };
        let Some(model) = self.model_turn.as_ref().map(|cfg| cfg.model.clone()) else {
            self.ctx.goal_reviewer = None;
            return;
        };
        let client = client.clone();
        let lite = self.lite_model.clone();
        self.ctx.goal_reviewer = Some(std::sync::Arc::new(move |material: String| {
            let client = client.clone();
            let lite = lite.clone();
            let model = model.clone();
            let review: std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = crate::native::goals::GoalReviewDecision>
                        + Send,
                >,
            > = Box::pin(async move {
                crate::native::goals::review_goal_with_client(
                    &client,
                    &model,
                    lite.as_deref(),
                    &material,
                )
                .await
            });
            review
        }));
    }

    pub(super) async fn goal_verification_paused(&self) -> bool {
        let Some(scope) = &self.ctx.session_scope else {
            return false;
        };
        crate::native::goals::verification_paused(&scope.pool, &self.ctx.session_record_id)
            .await
            .unwrap_or(false)
    }

    pub(super) async fn execute_logged_tool(&mut self, call: &ToolCall) -> ToolOutput {
        if let Some(rejection) = self.repeat_guard(call) {
            return ToolOutput::error(rejection);
        }
        match execute_tool_call(&self.ctx, call).await {
            Ok(value) => value,
            Err(error) => ToolOutput::error(error),
        }
    }

    pub(super) async fn reject_nested_agent(&mut self, call: &ToolCall) -> ToolOutput {
        if let Some(rejection) = self.repeat_guard(call) {
            return ToolOutput::error(rejection);
        }
        match preflight_tool(&self.ctx, call).await {
            Ok(prepared) => finalize_tool(
                &self.ctx,
                prepared,
                Err("子 Agent 不能再委派子 Agent".to_string()),
            )
            .await
            .unwrap_or_else(ToolOutput::error),
            Err(error) => ToolOutput::error(error),
        }
    }

    /// 同一轮工具调用的调度：连续的 `Agent` 调用成批并行；连续的
    /// `concurrent_safe` 只读工具并行（上限 [`MAX_PARALLEL_TOOL_CALLS`]）；
    /// 其余按顺序执行。结果始终按模型给出的顺序回填。
    pub(super) async fn execute_tool_calls(
        &mut self,
        mut calls: Vec<ToolCall>,
        client: Option<&ModelClient>,
    ) -> Result<(), String> {
        self.install_goal_reviewer(client);
        for call in &mut calls {
            self.assign_call_id(call);
        }
        self.persist_tool_plan(&calls).await?;
        let registry = self.contract_registry();
        let mut index = 0;
        while index < calls.len() {
            if !self.ctx.execution_current() {
                for call in &calls[index..] {
                    self.push_tool_output(
                        call,
                        ToolOutput::error(crate::native::steer::SUPERSEDED),
                    )
                    .await?;
                }
                return Ok(());
            }
            if self.ctx.cancel.is_cancelled() {
                return Err("已取消".to_string());
            }
            if calls[index].name == "Agent" {
                let mut end = index + 1;
                while end < calls.len() && calls[end].name == "Agent" {
                    end += 1;
                }
                self.run_agent_batch(&calls[index..end], client).await?;
                index = end;
                continue;
            }
            let parallel_ok = |call: &ToolCall| {
                call.name != "Agent" && registry.resolve(&call.name).can_run_concurrently()
            };
            if parallel_ok(&calls[index]) {
                let mut end = index + 1;
                while end < calls.len()
                    && end - index < MAX_PARALLEL_TOOL_CALLS
                    && parallel_ok(&calls[end])
                {
                    end += 1;
                }
                if end - index > 1 {
                    self.run_parallel_batch(&calls[index..end]).await?;
                    index = end;
                    continue;
                }
            }
            let call = &calls[index];
            self.emit_tool_start(call).await?;
            let output = self.execute_logged_tool(call).await;
            self.record_tool_result(call, output).await?;
            index += 1;
        }
        Ok(())
    }

    /// 并行执行一批只读工具；`ToolCtx` 克隆共享已读文件与待办状态。
    async fn run_parallel_batch(&mut self, calls: &[ToolCall]) -> Result<(), String> {
        let mut slot: Vec<Option<ToolOutput>> = vec![None; calls.len()];
        let mut join_set = JoinSet::new();
        for (pos, call) in calls.iter().enumerate() {
            self.emit_tool_start(call).await?;
            if let Some(rejection) = self.repeat_guard(call) {
                let output = ToolOutput::error(rejection);
                self.ledger_result(call, &output).await?;
                slot[pos] = Some(output);
                continue;
            }
            let ctx = self.ctx.clone();
            let call = call.clone();
            join_set.spawn(async move {
                let output = match execute_tool_call(&ctx, &call).await {
                    Ok(value) => value,
                    Err(error) => ToolOutput::error(error),
                };
                (pos, output)
            });
        }
        while let Some(joined) = join_set.join_next().await {
            let (pos, output) = joined.map_err(|error| format!("并行工具任务失败: {error}"))?;
            self.ledger_result(&calls[pos], &output).await?;
            slot[pos] = Some(output);
        }
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        for (call, output) in calls.iter().zip(slot) {
            let output = output.unwrap_or_else(|| ToolOutput::error("工具未返回结果"));
            self.record_tool_result(call, output).await?;
        }
        Ok(())
    }

    pub(super) async fn push_tool_output(
        &mut self,
        call: &ToolCall,
        output: ToolOutput,
    ) -> Result<(), String> {
        self.push_tool_output_with_tag(call, output, None).await
    }

    pub(super) async fn push_tool_output_with_tag(
        &mut self,
        call: &ToolCall,
        output: ToolOutput,
        tag: Option<&str>,
    ) -> Result<(), String> {
        self.ledger_result(call, &output).await?;
        self.emit_tool_result_with_tag(call, &output, tag).await?;
        self.append_tool_message(call, output);
        Ok(())
    }

    pub(super) async fn record_tool_result(
        &mut self,
        call: &ToolCall,
        mut output: ToolOutput,
    ) -> Result<(), String> {
        if !output.images.is_empty() {
            if let (Some(scope), Some(config_dir)) =
                (&self.ctx.session_scope, &self.ctx.app_config_dir)
            {
                crate::native::images::remember_tool_images(
                    &scope.pool,
                    &config_dir.join(crate::native::images::ATTACHMENTS_DIR_NAME),
                    &mut output.images,
                )
                .await?;
            }
        }
        self.ledger_result(call, &output).await?;
        self.emit_tool_result(call, &output).await?;
        self.append_tool_message(call, output);
        Ok(())
    }

    /// 先按契约结果预算裁决（超预算的 Artifact 策略落盘、只留预览），再按会话的
    /// token 上限截断；图片附件以紧随其后的用户消息形式交给模型。
    pub(super) fn append_tool_message(&mut self, call: &ToolCall, output: ToolOutput) {
        let contract = self.contract_registry().resolve(&call.name);
        let budgeted =
            bound_with_artifact(self.artifacts.as_deref(), &contract, call, &output.text);
        let bounded = truncate_tool_result(
            &call.name,
            &call.arguments,
            &budgeted,
            self.tool_result_token_limit,
        );
        if bounded != output.text {
            self.diagnostics
                .tool_results_truncated
                .fetch_add(1, Ordering::AcqRel);
        }
        let mut message = Message::tool_result(&call.id, bounded);
        message.name = call.name.clone();
        self.messages.push(message);
        if !output.images.is_empty() {
            let names: Vec<&str> = output
                .images
                .iter()
                .map(|image| image.name.as_str())
                .collect();
            self.messages.push(Message::user_with_images(
                format!("（{} 工具返回的图片：{}）", call.name, names.join("、")),
                output.images,
            ));
        }
    }
}
