use super::*;

impl AgentRunner {
    fn call_has_side_effect(&self, name: &str) -> bool {
        self.contract_registry().resolve(name).side_effect_scope
            != crate::native::tools::SideEffectScope::None
    }

    pub(super) async fn bind_recovery_turn(&self) {
        let Some(recovery) = &self.recovery else {
            return;
        };
        if recovery.turn().is_some() {
            return;
        }
        let Some(mailbox) = &self.ctx.user_steer else {
            return;
        };
        if let Some(turn_id) = mailbox.snapshot().await.turn_id {
            recovery.bind_turn(&turn_id);
        }
    }

    pub(super) async fn persist_tool_plan(&self, calls: &[ToolCall]) -> Result<(), String> {
        let Some(recovery) = &self.recovery else {
            return Ok(());
        };
        let planned = calls
            .iter()
            .map(|call| PlannedCall {
                call_id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                side_effect: self.call_has_side_effect(&call.name),
            })
            .collect::<Vec<_>>();
        recovery.commit_plan(&planned).await
    }

    pub(super) async fn ledger_started(&self, call_id: &str) -> Result<(), String> {
        let Some(recovery) = &self.recovery else {
            return Ok(());
        };
        recovery.mark_started(call_id).await
    }

    pub(super) async fn ledger_result(
        &self,
        call: &ToolCall,
        output: &ToolOutput,
    ) -> Result<(), String> {
        let Some(recovery) = &self.recovery else {
            return Ok(());
        };
        recovery
            .commit_result(&call.id, &output.text, !output.ok)
            .await
    }

    fn tool_result_present(&self, call_id: &str) -> bool {
        self.messages
            .iter()
            .any(|message| message.role == Role::Tool && message.tool_call_id == call_id)
    }

    pub async fn apply_tool_recovery(&mut self) -> Result<(), String> {
        let Some(recovery) = &self.recovery else {
            return Ok(());
        };
        let runs = recovery.settle_interrupted().await?;
        let assembly = RecoveryState::assemble(&self.messages, &runs);
        if let Some(assistant) = assembly.assistant {
            self.messages.push(assistant);
        }
        self.pending_recovery = assembly
            .steps
            .into_iter()
            .filter(|step| {
                let call_id = match step {
                    RecoveryStep::Reuse { call_id, .. }
                    | RecoveryStep::Execute { call_id, .. }
                    | RecoveryStep::Unknown { call_id, .. } => call_id,
                };
                !self.tool_result_present(call_id)
            })
            .collect();
        Ok(())
    }

    pub(super) async fn drain_pending_recovery(&mut self) -> Result<(), String> {
        if self.pending_recovery.is_empty() {
            return Ok(());
        }
        let steps = std::mem::take(&mut self.pending_recovery);
        for step in steps {
            if self.ctx.cancel.is_cancelled() {
                return Err("已取消".to_string());
            }
            match step {
                RecoveryStep::Reuse {
                    call_id,
                    name,
                    text,
                    is_error,
                } => {
                    let call = ToolCall {
                        id: call_id,
                        name,
                        arguments: String::new(),
                    };
                    let output = if is_error {
                        ToolOutput::error(text)
                    } else {
                        ToolOutput::text(text)
                    };
                    self.append_tool_message(&call, output);
                }
                RecoveryStep::Unknown { call_id, name } => {
                    let call = ToolCall {
                        id: call_id,
                        name,
                        arguments: String::new(),
                    };
                    self.append_tool_message(&call, ToolOutput::error(UNKNOWN_RESULT));
                }
                RecoveryStep::Execute {
                    call_id,
                    name,
                    arguments,
                } => {
                    let call = ToolCall {
                        id: call_id,
                        name,
                        arguments,
                    };
                    if !self.ctx.execution_current() {
                        let output = ToolOutput::error(crate::native::steer::SUPERSEDED);
                        self.ledger_result(&call, &output).await?;
                        self.append_tool_message(&call, output);
                        continue;
                    }
                    self.ledger_started(&call.id).await?;
                    self.emit_tool_start(&call).await?;
                    let output = self.execute_logged_tool(&call).await;
                    self.ledger_result(&call, &output).await?;
                    self.emit_tool_result(&call, &output).await?;
                    self.append_tool_message(&call, output);
                }
            }
        }
        self.checkpoint_transcript().await?;
        Ok(())
    }
}
