use super::*;

impl AgentRunner {
    pub fn take_steer_finish(&mut self) -> bool {
        std::mem::take(&mut self.pending_steer_finish)
    }

    pub(super) fn inject_steer_messages(&mut self) -> bool {
        let Some(rx) = self.steer_rx.clone() else {
            return false;
        };
        let Ok(mut guard) = rx.try_lock() else {
            return false;
        };
        let mut injected = false;
        while let Ok(item) = guard.try_recv() {
            match item {
                NativeFollowup::Input { text, images } => {
                    self.clear_output_recovery();
                    if let Some(tx) = &self.on_event {
                        let _ = tx.send(NativeEvent::UserInput {
                            text: text.clone(),
                            images: images.clone(),
                        });
                    }
                    self.messages.push(Message::user_with_images(text, images));
                    injected = true;
                }
                NativeFollowup::Compact(request) => {
                    self.pending_manual_compact = Some(request);
                }
                NativeFollowup::Finish => {
                    self.clear_output_recovery();
                    self.pending_steer_finish = true;
                }
            }
        }
        injected
    }

    /// Claim once before awaiting hooks; a second accepted input stays in the mailbox.
    pub(super) async fn inject_user_steer(&mut self) -> Result<bool, String> {
        let Some(mailbox) = self.ctx.user_steer.clone() else {
            return Ok(false);
        };
        let mut consumed = false;
        while let Some(input) = mailbox.take().await {
            consumed = true;
            self.clear_output_recovery();
            self.ctx.main_origin = Some(crate::native::steer::MainOrigin {
                instance_id: input.receipt.instance_id.clone(),
                generation: input.receipt.generation,
                child: false,
            });
            if self.ctx.cancel.is_cancelled() {
                mailbox
                    .finish_input(
                        &input.receipt.input_id,
                        crate::native::steer::SteerStatus::Cancelled,
                        Some("会话已停止".into()),
                    )
                    .await?;
                continue;
            }
            let mut text = input.receipt.text.clone();
            match run_user_prompt_submit_hooks(&self.ctx.hook_runtime(), &text).await {
                Ok(context) => {
                    if self.ctx.cancel.is_cancelled() {
                        mailbox
                            .finish_input(
                                &input.receipt.input_id,
                                crate::native::steer::SteerStatus::Cancelled,
                                Some("会话已停止".into()),
                            )
                            .await?;
                        continue;
                    }
                    if !context.is_empty() {
                        text = format!("{text}\n\n[钩子上下文]\n{}", context.join("\n"));
                    }
                    self.messages
                        .push(Message::user_with_images(text, input.images));
                    self.checkpoint_transcript().await?;
                    mailbox
                        .finish_input(
                            &input.receipt.input_id,
                            crate::native::steer::SteerStatus::Applied,
                            None,
                        )
                        .await?;
                }
                Err(reason) => {
                    mailbox
                        .finish_input(
                            &input.receipt.input_id,
                            crate::native::steer::SteerStatus::Rejected,
                            Some(format!("输入被钩子阻断：{reason}")),
                        )
                        .await?
                }
            }
        }
        Ok(consumed)
    }

    pub(super) async fn seal_user_turn(&mut self) -> Result<bool, String> {
        if self.inject_user_steer().await? {
            return Ok(false);
        }
        match &self.ctx.user_steer {
            Some(mailbox) => Ok(mailbox.seal().await),
            None => Ok(true),
        }
    }
}
