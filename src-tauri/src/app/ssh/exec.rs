use std::time::Duration;

use russh::client::Msg;
use russh::{Channel, ChannelMsg};
use tauri::{AppHandle, Manager, Runtime};
use tokio::io::AsyncWriteExt;

use crate::db::models::SshConfigRecord;

use super::client::ConnectParams;
use super::error::SshError;
use super::pool::SshPool;

#[derive(Debug, Clone)]
pub(crate) struct SshCommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
}

impl SshCommandOutput {
    pub(crate) fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    pub(crate) fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub(crate) fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExecOptions {
    pub stdin: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            stdin: None,
            timeout: Some(Duration::from_secs(120)),
        }
    }
}

#[derive(Debug)]
pub(crate) enum SshStreamEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(i32),
    Closed,
}

pub(crate) struct SshCommandStream {
    channel: Channel<Msg>,
}

impl SshCommandStream {
    pub(crate) async fn next(&mut self) -> Option<SshStreamEvent> {
        loop {
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    return Some(SshStreamEvent::Stdout(data.to_vec()));
                }
                Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                    return Some(SshStreamEvent::Stderr(data.to_vec()));
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return Some(SshStreamEvent::Exit(exit_status as i32));
                }
                Some(ChannelMsg::Close) | None => return Some(SshStreamEvent::Closed),
                Some(_) => continue,
            }
        }
    }

    pub(crate) async fn write_stdin(&self, bytes: &[u8]) -> Result<(), SshError> {
        let mut writer = self.channel.make_writer();
        writer.write_all(bytes).await?;
        Ok(())
    }

    pub(crate) async fn eof(&self) -> Result<(), SshError> {
        let mut writer = self.channel.make_writer();
        writer.shutdown().await?;
        Ok(())
    }

    pub(crate) async fn close(&self) -> Result<(), SshError> {
        self.channel.close().await?;
        Ok(())
    }
}

impl SshPool {
    pub(crate) async fn exec(
        &self,
        params: &ConnectParams,
        cmd: &str,
        options: ExecOptions,
    ) -> Result<SshCommandOutput, SshError> {
        match self.exec_once(params, cmd, &options).await {
            Err(SshError::ConnectionLost) => {
                self.invalidate(&params.ssh_config_id).await;
                self.exec_once(params, cmd, &options).await
            }
            other => other,
        }
    }

    async fn exec_once(
        &self,
        params: &ConnectParams,
        cmd: &str,
        options: &ExecOptions,
    ) -> Result<SshCommandOutput, SshError> {
        let channel = self.open_session(params).await?;
        run_exec_channel(channel, cmd, options).await
    }

    pub(crate) async fn spawn(
        &self,
        params: &ConnectParams,
        cmd: &str,
    ) -> Result<SshCommandStream, SshError> {
        match self.spawn_once(params, cmd).await {
            Err(SshError::ConnectionLost) => {
                self.invalidate(&params.ssh_config_id).await;
                self.spawn_once(params, cmd).await
            }
            other => other,
        }
    }

    async fn spawn_once(
        &self,
        params: &ConnectParams,
        cmd: &str,
    ) -> Result<SshCommandStream, SshError> {
        let channel = self.open_session(params).await?;
        channel.exec(true, cmd).await?;
        Ok(SshCommandStream { channel })
    }
}

async fn run_exec_channel(
    channel: Channel<Msg>,
    cmd: &str,
    options: &ExecOptions,
) -> Result<SshCommandOutput, SshError> {
    channel.exec(true, cmd).await?;

    if let Some(stdin) = options.stdin.clone() {
        let mut writer = channel.make_writer();
        tokio::spawn(async move {
            let _ = writer.write_all(&stdin).await;
            let _ = writer.shutdown().await;
        });
    }

    let timeout = options.timeout.unwrap_or(Duration::from_secs(120));
    collect_command_output(channel, timeout).await
}

async fn collect_command_output(
    mut channel: Channel<Msg>,
    timeout: Duration,
) -> Result<SshCommandOutput, SshError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;
    let mut failed = false;
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => stdout.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExtendedData { data, ext: 1 })) => {
                stderr.extend_from_slice(&data);
            }
            Ok(Some(ChannelMsg::ExitStatus { exit_status })) => {
                exit_code = Some(exit_status as i32);
            }
            Ok(Some(ChannelMsg::Success)) => {}
            Ok(Some(ChannelMsg::Failure)) => failed = true,
            Ok(Some(ChannelMsg::Eof)) => {}
            Ok(Some(ChannelMsg::Close)) | Ok(None) => break,
            Ok(Some(_)) => {}
            Err(_) => {
                let _ = channel.close().await;
                return Err(SshError::CommandTimeout);
            }
        }
    }

    if failed && exit_code.is_none() {
        return Err(SshError::ChannelFailure);
    }

    Ok(SshCommandOutput {
        stdout,
        stderr,
        exit_code,
    })
}

pub(crate) async fn execute_ssh_command<R: Runtime>(
    app: &AppHandle<R>,
    ssh_config: &SshConfigRecord,
    remote_command: &str,
    require_password_probe: bool,
) -> Result<SshCommandOutput, String> {
    let pool = app.state::<SshPool>().inner().clone();
    let params = super::resolve_connect_params(app, ssh_config, require_password_probe)?;
    pool.exec(&params, remote_command, ExecOptions::default())
        .await
        .map_err(Into::into)
}

pub(crate) async fn execute_ssh_command_with_input<R: Runtime>(
    app: &AppHandle<R>,
    ssh_config: &SshConfigRecord,
    remote_command: &str,
    stdin_bytes: Vec<u8>,
    require_password_probe: bool,
) -> Result<SshCommandOutput, String> {
    let pool = app.state::<SshPool>().inner().clone();
    let params = super::resolve_connect_params(app, ssh_config, require_password_probe)?;
    pool.exec(
        &params,
        remote_command,
        ExecOptions {
            stdin: Some(stdin_bytes),
            timeout: Some(Duration::from_secs(120)),
        },
    )
    .await
    .map_err(Into::into)
}

pub(crate) async fn spawn_ssh_command<R: Runtime>(
    app: &AppHandle<R>,
    ssh_config: &SshConfigRecord,
    remote_command: &str,
    require_password_probe: bool,
) -> Result<SshCommandStream, String> {
    let pool = app.state::<SshPool>().inner().clone();
    let params = super::resolve_connect_params(app, ssh_config, require_password_probe)?;
    pool.spawn(&params, remote_command)
        .await
        .map_err(Into::into)
}
