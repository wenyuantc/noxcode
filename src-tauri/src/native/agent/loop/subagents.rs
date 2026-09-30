use super::*;

impl AgentRunner {
    pub(super) fn spawn_child_runner(&self, spec: &SubagentSpec, index: u32) -> AgentRunner {
        self.spawn_child_with_quota(spec, index, self.child_quota_for_share())
    }

    pub(super) fn child_quota_for_share(&self) -> Option<Arc<ChildQuota>> {
        if self.rollout_budget.limit() == 0 {
            None
        } else {
            let share = u64::from(self.subagent_budget_share_percent.clamp(5, 100));
            let limit = self.rollout_budget.remaining().saturating_mul(share) / 100;
            Some(ChildQuota::shared(limit))
        }
    }

    pub(super) fn spawn_child_with_quota(
        &self,
        spec: &SubagentSpec,
        index: u32,
        child_quota: Option<Arc<ChildQuota>>,
    ) -> AgentRunner {
        let mut child = AgentRunner::new(self.ctx.workspace.clone());
        // 共享取消 / 权限放行 / MCP 服务器放行 / 代理环境；已读文件与待办独立。
        child.ctx = self.ctx.fork_for_child();
        child.artifacts = self.artifacts.clone();
        child.lite_model = self.lite_model.clone();
        child.extra_tool_contracts = self.extra_tool_contracts.clone();
        child.skills_prompt = self.skills_prompt.clone();
        child.depth = self.depth.saturating_add(1);
        child.event_prefix = format!(
            "{} ",
            format_subagent_log_tag(index, &spec.kind, &spec.description)
        );
        child.max_turns = self.max_subagent_turns;
        child.max_subagent_turns = self.max_subagent_turns;
        child.max_concurrent_subagents = self.max_concurrent_subagents;
        child.subagent_policy = self.subagent_policy.clone();
        child.context_char_limit = self.context_char_limit;
        child.rollout_budget = self.rollout_budget.clone();
        child.subagent_budget_share_percent = self.subagent_budget_share_percent;
        child.child_quota = child_quota;
        child.context_window = ContextWindow::new(self.context_window.token_limit);
        child.tool_result_token_limit = self.tool_result_token_limit;
        child.diagnostics = self.diagnostics.clone();
        child.on_event = self.on_event.clone();
        child.on_usage = self.on_usage.clone();
        child.workspace_context = self.workspace_context.clone();
        child.project_agents = self.project_agents.clone();
        let custom_def = match &spec.kind {
            SubagentKind::Custom(name) => {
                find_native_subagent(&self.custom_subagents, name).cloned()
            }
            _ => None,
        };
        // 父会话的记忆目录只留给 general 子 Agent；其余子 Agent 不能写它。
        if !matches!(spec.kind, SubagentKind::General) {
            child.ctx.workspace.extra_write_roots.clear();
        }
        match &spec.kind {
            SubagentKind::Explore => {
                child.ctx.set_read_only(true);
                child.set_allowed_tools(&read_only_tool_names_with_bash());
            }
            SubagentKind::General => {
                child.ctx.mcp = self.ctx.mcp.clone();
                child.set_extra_tools(self.extra_tools.clone());
            }
            SubagentKind::Custom(_) => {
                if let Some(def) = custom_def.as_ref() {
                    if def.tool_mode == TOOL_MODE_ALL {
                        child.ctx.mcp = self.ctx.mcp.clone();
                        child.set_extra_tools(self.extra_tools.clone());
                    } else {
                        let tools = effective_custom_tools(&def.tools);
                        let names: Vec<&str> = tools.iter().map(String::as_str).collect();
                        child.set_allowed_tools(&names);
                        if custom_tools_are_read_only(&tools) {
                            child.ctx.set_read_only(true);
                        }
                    }
                    if !def.disallowed_tools.is_empty() {
                        child.set_disallowed_tools(&def.disallowed_tools);
                    }
                    if let Some(max_turns) = def.max_turns {
                        child.max_turns = max_turns.max(1) as u32;
                    }
                    // 档案限定了技能时，子 Agent 只看到这些技能。
                    if !def.skills.is_empty() {
                        child.ctx.skills.retain(|skill| {
                            def.skills
                                .iter()
                                .any(|name| name.eq_ignore_ascii_case(&skill.name))
                        });
                        child.skills_prompt =
                            crate::native::skills::format_skills_prompt(&child.ctx.skills);
                    }
                    // 档案自带权限模式时，子 Agent 用自己的放行开关，不再共享父会话的。
                    if let Some(mode) = def.permission_mode.as_deref() {
                        use crate::native::settings::{
                            permission_mode_auto_approves_build,
                            permission_mode_auto_approves_edits, permission_mode_is_yolo,
                        };
                        child.ctx.allow_all_high_risk = Arc::new(
                            std::sync::atomic::AtomicBool::new(permission_mode_is_yolo(mode)),
                        );
                        child.ctx.auto_approve_overwrite =
                            permission_mode_auto_approves_edits(mode);
                        child.ctx.auto_approve_opaque_bash =
                            permission_mode_auto_approves_build(mode);
                        child.ctx.auto_approve_readonly_mcp =
                            permission_mode_auto_approves_build(mode);
                    }
                }
            }
        }
        let parent_system = self
            .messages
            .iter()
            .find(|message| message.role == Role::System)
            .map(|message| message.content.clone());
        let mut system = if let Some(def) = custom_def.as_ref() {
            custom_child_system_prompt(spec, def, &self.workspace_context, &self.project_agents)
        } else {
            child_system_prompt(parent_system.as_deref(), spec)
        };
        if let Some(def) = custom_def.as_ref() {
            if let Some(block) = self.bind_agent_memory(&mut child, def) {
                system.push_str("\n\n");
                system.push_str(&block);
            }
        }
        child.messages.push(Message::system(system));
        child
    }

    /// 档案开启持久记忆时，只通过 `Memory` 工具开放该子 Agent 自己的记忆目录。
    /// 不改变只读状态，也不增加任何工作区写入能力。返回注入系统提示的记忆块。
    fn bind_agent_memory(&self, child: &mut AgentRunner, def: &NativeSubagent) -> Option<String> {
        let scope = def.memory.as_deref()?;
        let roots = self.agent_memory_roots.as_ref()?;
        let dir = match crate::native::subagents::prepare_agent_memory_dir(roots, scope, &def.name)
        {
            Ok(dir) => dir,
            Err(error) => {
                self.emit(format!("[子 Agent] {} 的记忆不可用：{error}", def.name));
                return None;
            }
        };
        if let Some(allowed) = child.allowed_tools.as_mut() {
            allowed.insert("Memory".to_string());
        }
        let index = crate::native::memory::load_index(&dir);
        child.ctx.memory = Some(crate::native::tools::memory_tool::MemoryBinding {
            dir,
            writable: true,
        });
        Some(format!(
            "# 你的持久记忆（{scope}）\n这是只属于 {} 的记忆，跨会话保留。用 Memory 工具 list / search / read 查看；发现值得长期保留的信息（用户偏好、纠正过的做法、项目决策、参考资料）时用 Memory 工具 write，过时的条目要更新或删除。不要用 Write / Edit 修改记忆。\n\n{}",
            def.name,
            if index.trim().is_empty() {
                "（当前还没有记忆条目）".to_string()
            } else {
                index.trim().to_string()
            }
        ))
    }

    pub(super) async fn run_agent_batch(
        &mut self,
        calls: &[ToolCall],
        client: Option<&ModelClient>,
    ) -> Result<(), String> {
        if self.depth > 0 {
            for call in calls {
                let output = self.reject_nested_agent(call).await;
                self.push_tool_output(call, output).await?;
            }
            return Ok(());
        }
        if let Some(reload) = &self.reload_custom_subagents {
            self.custom_subagents = reload();
        }
        let mut slot: Vec<Option<(ToolCall, ToolOutput)>> = vec![None; calls.len()];
        struct Job {
            call: ToolCall,
            spec: SubagentSpec,
            index: u32,
            prepared: PreparedTool,
        }
        let mut jobs = Vec::new();
        for (pos, call) in calls.iter().enumerate() {
            if let Some(rejection) = self.call_guard(call) {
                slot[pos] = Some((call.clone(), ToolOutput::error(rejection)));
                continue;
            }
            let prepared = match preflight_tool(&self.ctx, call).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    slot[pos] = Some((call.clone(), ToolOutput::error(error)));
                    continue;
                }
            };
            let mut call = call.clone();
            call.arguments = prepared.arguments.clone();
            match parse_subagent_args_with(&call.arguments, &self.custom_subagents) {
                Ok(spec) => {
                    if self.ctx.is_plan_mode() && !matches!(&spec.kind, SubagentKind::Explore) {
                        let output = finalize_tool(
                            &self.ctx,
                            prepared,
                            Err("计划模式只能使用内置 explore 子智能体".to_string()),
                        )
                        .await
                        .unwrap_or_else(ToolOutput::error);
                        slot[pos] = Some((call, output));
                        continue;
                    }
                    self.subagent_seq = self.subagent_seq.saturating_add(1);
                    jobs.push((
                        pos,
                        Job {
                            call,
                            spec,
                            index: self.subagent_seq,
                            prepared,
                        },
                    ));
                }
                Err(error) => {
                    let output = finalize_tool(&self.ctx, prepared, Err(error))
                        .await
                        .unwrap_or_else(ToolOutput::error);
                    slot[pos] = Some((call, output));
                }
            }
        }
        let semaphore = self
            .subagent_semaphore
            .get_or_insert_with(|| {
                Arc::new(Semaphore::new(self.max_concurrent_subagents.max(1) as usize))
            })
            .clone();
        let mut join_set = JoinSet::new();
        let stub = self.subagent_stub.clone();
        let model_turn = self.model_turn.clone();
        let client_owned = client.cloned();
        let batch_quota = self.child_quota_for_share();
        let mut tags: HashMap<String, String> = HashMap::new();
        for (pos, job) in jobs {
            if !self.ctx.execution_current() {
                slot[pos] = Some((
                    job.call,
                    ToolOutput::error(crate::native::steer::SUPERSEDED),
                ));
                continue;
            }
            let tag = format_subagent_log_tag(job.index, &job.spec.kind, &job.spec.description);
            tags.insert(job.call.id.clone(), tag.clone());
            self.emit_tool_start_with_tag(&job.call, Some(&tag)).await?;
            if !self.ctx.execution_current() {
                slot[pos] = Some((
                    job.call,
                    ToolOutput::error(crate::native::steer::SUPERSEDED),
                ));
                continue;
            }
            self.emit(format!("{} 启动（{}）", tag, job.spec.kind.as_str()));
            self.emit_activity(
                "native_subagent_started",
                &format!("{}（{}）", job.spec.description, job.spec.kind.as_str()),
            );
            let permit = semaphore.clone();
            let stub = stub.clone();
            let client_owned = client_owned.clone();
            let model_turn = model_turn.clone();
            let child_model_loader = self.child_model_loader.clone();
            let custom_override = match &job.spec.kind {
                SubagentKind::Custom(name) => find_native_subagent(&self.custom_subagents, name)
                    .filter(|item| item.model_mode == MODEL_MODE_CHANNEL)
                    .and_then(|item| {
                        Some((
                            item.channel_id.clone()?,
                            item.model.clone()?,
                            item.reasoning_effort.clone(),
                        ))
                    }),
                _ => None,
            };
            let mut child = self.spawn_child_with_quota(&job.spec, job.index, batch_quota.clone());
            if !job.call.id.is_empty() {
                child.ctx.file_attribution_call_id = Some(job.call.id.clone());
            }
            self.diagnostics
                .subagents_started
                .fetch_add(1, Ordering::AcqRel);
            if job.spec.run_in_background {
                // 后台任务：立刻返回 task_id，子 Agent 在独立任务里跑，结果进注册表。
                let (task, steer_rx) = self
                    .background
                    .register(&job.spec.description, job.spec.kind.as_str());
                let parent_cancel = self.ctx.cancel.clone();
                child.ctx.cancel = task.cancel.clone();
                child.steer_rx = Some(Arc::new(Mutex::new(steer_rx)));
                child.ctx.coordinator = Some((self.background.clone(), task.id.clone()));
                let registry = self.background.clone();
                let task_id = task.id.clone();
                let on_event = self.on_event.clone();
                let prefix = self.event_prefix.clone();
                let spec = job.spec.clone();
                let run = run_child_job(
                    child,
                    spec.clone(),
                    permit,
                    stub,
                    custom_override,
                    child_model_loader,
                    client_owned,
                    model_turn,
                );
                let cancel_flag = task.cancel.clone();
                tokio::spawn(async move {
                    tokio::pin!(run);
                    let outcome = tokio::select! {
                        result = &mut run => result,
                        _ = async {
                            loop {
                                if parent_cancel.is_cancelled() {
                                    cancel_flag.cancel();
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        } => run.await,
                    };
                    let status = if outcome.is_ok() { "成功" } else { "失败" };
                    Self::send_prefixed_event(
                        &on_event,
                        &prefix,
                        format!("{tag} 后台任务 {task_id} 结束 {status}"),
                    );
                    registry.finish(&task_id, outcome.map(|report| truncate_report(&report)));
                });
                let output = finalize_tool(
                    &self.ctx,
                    job.prepared,
                    Ok(ToolOutput::text(format!(
                        "后台任务已启动：task_id={}（{} / {}）。用 TaskOutput 读取结果、SendMessage 追加指令、TaskStop 停止；任务完成时会收到提醒。",
                        task.id,
                        spec.kind.as_str(),
                        spec.description
                    ))),
                )
                .await
                .unwrap_or_else(ToolOutput::error);
                slot[pos] = Some((job.call, output));
                continue;
            }
            let run = run_child_job(
                child,
                job.spec.clone(),
                permit,
                stub,
                custom_override,
                child_model_loader,
                client_owned,
                model_turn,
            );
            join_set.spawn(async move {
                let outcome = run.await;
                (pos, job, outcome)
            });
        }
        while let Some(joined) = join_set.join_next().await {
            let (pos, job, outcome) =
                joined.map_err(|error| format!("子 Agent 任务失败: {error}"))?;
            let result = match &outcome {
                Ok(report) => Ok(ToolOutput::text(format_subagent_result(
                    &job.spec,
                    Ok(report),
                ))),
                Err(error) => Err(format_subagent_result(&job.spec, Err(error))),
            };
            let output = finalize_tool(&self.ctx, job.prepared, result)
                .await
                .unwrap_or_else(ToolOutput::error);
            let status = if outcome.is_ok() { "成功" } else { "失败" };
            self.emit(format!(
                "{} 结束 {status}",
                format_subagent_log_tag(job.index, &job.spec.kind, &job.spec.description)
            ));
            self.emit_activity(
                "native_subagent_finished",
                &format!(
                    "{}（{}）{status}",
                    job.spec.description,
                    job.spec.kind.as_str()
                ),
            );
            slot[pos] = Some((job.call, output));
        }
        for item in slot.into_iter().flatten() {
            let tag = tags.get(&item.0.id).map(String::as_str);
            self.push_tool_output_with_tag(&item.0, item.1, tag).await?;
        }
        Ok(())
    }
}

/// 前台和后台任务共用 runner 的并发许可；等待许可与执行过程均响应取消。
#[allow(clippy::too_many_arguments)]
fn run_child_job(
    mut child: AgentRunner,
    spec: SubagentSpec,
    permit: Arc<Semaphore>,
    stub: Option<SubagentStub>,
    custom_override: Option<(String, String, Option<String>)>,
    child_model_loader: Option<ChildModelLoader>,
    client_owned: Option<ModelClient>,
    model_turn: Option<ModelTurnCfg>,
) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
    Box::pin(async move {
        let cancel = child.ctx.cancel.clone();
        let _permit = tokio::select! {
            biased;
            _ = wait_for_cancel(&cancel) => return Err("已取消".to_string()),
            permit = permit.acquire_owned() => {
                permit.map_err(|_| "子 Agent 并发许可已关闭".to_string())?
            }
        };
        if let Some((registry, task_id)) = &child.ctx.coordinator {
            registry.mark_running(task_id);
        }
        let run = async {
            if let Some(stub) = stub {
                Ok(stub(&spec))
            } else if let Some((channel_id, model, effort)) = custom_override {
                let Some(loader) = child_model_loader else {
                    return Err("子 Agent 需要模型客户端".to_string());
                };
                let settings = loader(channel_id, model, effort).await?;
                child
                    .run_child_with_client(
                        client_owned.as_ref(),
                        &settings.client,
                        &spec.prompt,
                        &settings.model,
                        settings.effort.as_deref(),
                        settings.max_output_tokens,
                        settings.thinking_enabled,
                        Some(&spec),
                    )
                    .await
            } else if let (Some(client), Some(cfg)) = (client_owned.as_ref(), model_turn) {
                child
                    .run_child_with_client(
                        Some(client),
                        client,
                        &spec.prompt,
                        &cfg.model,
                        cfg.effort.as_deref(),
                        cfg.max_output_tokens,
                        cfg.thinking_enabled,
                        Some(&spec),
                    )
                    .await
            } else {
                Err("子 Agent 需要模型客户端".to_string())
            }
        };
        tokio::pin!(run);
        tokio::select! {
            biased;
            _ = wait_for_cancel(&cancel) => {
                // 让正在运行的本地 / SSH 工具先观察取消并终止子进程，再中断模型请求。
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), &mut run).await;
                Err("已取消".to_string())
            },
            result = &mut run => result,
        }
    })
}

async fn wait_for_cancel(cancel: &crate::native::tools::CancelFlag) {
    while !cancel.is_cancelled() {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}
