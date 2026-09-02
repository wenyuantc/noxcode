use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime};

use crate::app::shared::{
    sqlite_pool, EXECUTION_TARGET_LOCAL, EXECUTION_TARGET_SSH, WORKSPACE_TYPE_LOCAL,
    WORKSPACE_TYPE_SSH,
};
use crate::app::ssh::configs::{fetch_ssh_config_record_by_id, ssh_config_target_host_label};
use crate::app::workspaces::fetch_workspace_by_id;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ExecutionContext {
    pub execution_target: String,
    pub working_dir: Option<String>,
    pub ssh_config_id: Option<String>,
    pub target_host_label: Option<String>,
}

#[allow(dead_code)]
impl ExecutionContext {
    pub(crate) fn local_default() -> Self {
        Self {
            execution_target: EXECUTION_TARGET_LOCAL.to_string(),
            working_dir: None,
            ssh_config_id: None,
            target_host_label: None,
        }
    }

    pub(crate) fn is_ssh(&self) -> bool {
        self.execution_target == EXECUTION_TARGET_SSH
    }
}

pub(crate) async fn resolve_workspace_execution_context_with_pool(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<ExecutionContext, String> {
    let workspace = fetch_workspace_by_id(pool, workspace_id).await?;
    match workspace.workspace_type.as_str() {
        value if value == WORKSPACE_TYPE_LOCAL => {
            let repo_path = workspace
                .repo_path
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "本地工作区缺少 repo_path".to_string())?;
            if !Path::new(&repo_path).is_dir() {
                return Err(format!("本地仓库路径不存在或不是目录: {repo_path}"));
            }
            Ok(ExecutionContext {
                execution_target: EXECUTION_TARGET_LOCAL.to_string(),
                working_dir: Some(repo_path),
                ssh_config_id: None,
                target_host_label: None,
            })
        }
        value if value == WORKSPACE_TYPE_SSH => {
            let ssh_config_id = workspace
                .ssh_config_id
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "SSH 工作区缺少 ssh_config_id".to_string())?;
            let remote_repo_path = workspace
                .remote_repo_path
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "SSH 工作区缺少 remote_repo_path".to_string())?;
            let record = fetch_ssh_config_record_by_id(pool, &ssh_config_id).await?;
            Ok(ExecutionContext {
                execution_target: EXECUTION_TARGET_SSH.to_string(),
                working_dir: Some(remote_repo_path),
                ssh_config_id: Some(ssh_config_id),
                target_host_label: Some(ssh_config_target_host_label(&record)),
            })
        }
        other => Err(format!("不支持的工作区类型: {other}")),
    }
}

#[allow(dead_code)]
pub(crate) async fn resolve_workspace_execution_context<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: &str,
) -> Result<ExecutionContext, String> {
    let pool = sqlite_pool(app).await?;
    resolve_workspace_execution_context_with_pool(&pool, workspace_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::secret_store::SecretStore;
    use crate::app::ssh::configs::create_ssh_config_with;
    use crate::db::models::CreateSshConfig;
    use crate::db::test_support::setup_migrated_pool;

    #[test]
    fn local_default_is_not_ssh() {
        let ctx = ExecutionContext::local_default();
        assert!(!ctx.is_ssh());
        assert_eq!(ctx.execution_target, EXECUTION_TARGET_LOCAL);
    }

    #[test]
    fn resolve_local_workspace_requires_existing_dir() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let dir = tempfile::tempdir().expect("tempdir");
            sqlx::query(
                "INSERT INTO workspaces (id, name, workspace_type, repo_path) VALUES ('ws-local', 'local', 'local', $1)",
            )
            .bind(dir.path().to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .expect("insert local workspace");

            let ctx = resolve_workspace_execution_context_with_pool(&pool, "ws-local")
                .await
                .expect("resolve local");
            assert!(!ctx.is_ssh());
            assert_eq!(
                ctx.working_dir.as_deref(),
                Some(dir.path().to_string_lossy().as_ref())
            );

            sqlx::query(
                "INSERT INTO workspaces (id, name, workspace_type, repo_path) VALUES ('ws-missing', 'missing', 'local', '/definitely/not/here')",
            )
            .execute(&pool)
            .await
            .expect("insert missing");
            let error = resolve_workspace_execution_context_with_pool(&pool, "ws-missing")
                .await
                .expect_err("missing path");
            assert!(error.contains("不是目录") || error.contains("不存在"));
        });
    }

    #[test]
    fn resolve_ssh_workspace_requires_remote_path_and_builds_label() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let dir = tempfile::tempdir().expect("secrets");
            let secrets = SecretStore::in_memory(dir.path().to_path_buf());
            let ssh = create_ssh_config_with(
                &pool,
                &secrets,
                CreateSshConfig {
                    name: "prod".to_string(),
                    host: "ssh.example.test".to_string(),
                    port: Some(22),
                    username: "deploy".to_string(),
                    auth_type: "key".to_string(),
                    private_key_path: Some("~/.ssh/id_ed25519".to_string()),
                    password: None,
                    passphrase: None,
                    known_hosts_mode: Some("accept-new".to_string()),
                },
            )
            .await
            .expect("create ssh");

            sqlx::query(
                "INSERT INTO workspaces (id, name, workspace_type, ssh_config_id) VALUES ('ws-bad', 'bad', 'ssh', $1)",
            )
            .bind(&ssh.id)
            .execute(&pool)
            .await
            .expect("insert bad ssh workspace");
            let error = resolve_workspace_execution_context_with_pool(&pool, "ws-bad")
                .await
                .expect_err("missing remote path");
            assert!(error.contains("remote_repo_path"));

            sqlx::query(
                "INSERT INTO workspaces (id, name, workspace_type, ssh_config_id, remote_repo_path) VALUES ('ws-ssh', 'ssh', 'ssh', $1, '/opt/app')",
            )
            .bind(&ssh.id)
            .execute(&pool)
            .await
            .expect("insert ssh workspace");
            let ctx = resolve_workspace_execution_context_with_pool(&pool, "ws-ssh")
                .await
                .expect("resolve ssh");
            assert!(ctx.is_ssh());
            assert_eq!(ctx.working_dir.as_deref(), Some("/opt/app"));
            assert_eq!(ctx.ssh_config_id.as_deref(), Some(ssh.id.as_str()));
            assert_eq!(
                ctx.target_host_label.as_deref(),
                Some("deploy@ssh.example.test:22")
            );
        });
    }
}
