use serde::{Deserialize, Serialize};

use super::contract::{PatternSource, PermissionCapability};
use super::dispatch::ToolCtx;
use super::patch::{extract_patch_text, parse_patch, PatchAction};
use super::paths::path_is_within;
use super::paths::{resolve_local_path, resolve_posix_path, resolve_under_workspace_posix};
use super::permission::{PermissionRule, RuleScope};

pub fn is_file_tool(name: &str) -> bool {
    matches!(
        name,
        "Read" | "SQLiteQuery" | "Glob" | "Grep" | "Write" | "Edit" | "ApplyPatch"
    )
}

pub async fn collect_file_access(
    ctx: &ToolCtx,
    name: &str,
    arguments: &str,
) -> Result<FileAccessPrompt, String> {
    let args: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("工具参数不是合法 JSON: {error}"))?;
    let mut inputs = Vec::new();
    if name == "ApplyPatch" {
        for action in parse_patch(&extract_patch_text(arguments)?)? {
            match action {
                PatchAction::Add { path, .. } => inputs.push((path, "create")),
                PatchAction::Delete { path } => inputs.push((path, "delete")),
                PatchAction::Update { path, move_to, .. } => {
                    inputs.push((
                        path,
                        if move_to.is_some() {
                            "move_source"
                        } else {
                            "edit"
                        },
                    ));
                    if let Some(dest) = move_to {
                        inputs.push((dest, "move_destination"));
                    }
                }
            }
        }
    } else {
        let key = if matches!(name, "Glob" | "Grep") {
            "path"
        } else {
            "file_path"
        };
        let value = args
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let input = match value {
            Some(value) => value,
            None if key == "path" => ".",
            None => return Err(format!("{key} 不能为空")),
        };
        inputs.push((
            input.to_string(),
            match name {
                "Read" | "SQLiteQuery" => "read",
                "Glob" | "Grep" => "search",
                "Write" => "write",
                _ => "edit",
            },
        ));
    }
    let capability = if matches!(name, "Read" | "SQLiteQuery" | "Glob" | "Grep") {
        PermissionCapability::Read
    } else {
        PermissionCapability::Edit
    };
    let workspace = ctx.workspace_for_read();
    let mut paths = Vec::new();
    for (input, operation) in inputs {
        let (path, outside_workspace, directory) = if let Some(ssh) = ctx.ssh_for_exec() {
            let path = resolve_posix_path(&ssh.root, &input)?;
            let outside = resolve_under_workspace_posix(&ssh.root, &input).is_err();
            let directory = ssh.validate_path(&path).await?;
            (path, outside, directory)
        } else {
            let path = resolve_local_path(&workspace.root, &input)?;
            let outside = if capability == PermissionCapability::Read {
                workspace.resolve_for_read(&input).is_err()
            } else {
                workspace.resolve_for_write(&input).is_err()
            };
            (path.to_string_lossy().into_owned(), outside, path.is_dir())
        };
        let scope = if operation == "search" && directory {
            PathAccessScope::Subtree
        } else {
            PathAccessScope::Exact
        };
        paths.push(FileAccessPath {
            path,
            requested_path: input,
            capability,
            scope,
            operation: operation.to_string(),
            outside_workspace,
        });
    }
    Ok(FileAccessPrompt {
        target: ctx.permission_target(),
        paths,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionTarget {
    Local,
    Ssh {
        config_id: String,
        host: String,
        port: i64,
        username: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathAccessScope {
    Exact,
    Subtree,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalPathRule {
    pub target: PermissionTarget,
    pub scope: PathAccessScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAccessPath {
    pub path: String,
    pub requested_path: String,
    pub capability: PermissionCapability,
    pub scope: PathAccessScope,
    pub operation: String,
    pub outside_workspace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAccessPrompt {
    pub target: PermissionTarget,
    pub paths: Vec<FileAccessPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAccessSelection {
    pub path: String,
    pub directory: bool,
}

impl FileAccessPrompt {
    /// The UI selects a presented target or its parent; it never supplies a new root.
    pub fn rules_for_selection(
        &self,
        selections: &[FileAccessSelection],
        scope: RuleScope,
    ) -> Result<Vec<PermissionRule>, String> {
        if selections.len() != self.paths.len() {
            return Err("授权目标与待确认请求不一致".to_string());
        }
        self.paths
            .iter()
            .zip(selections)
            .map(|(entry, selection)| {
                if entry.path != selection.path {
                    return Err("授权目标与待确认请求不一致".to_string());
                }
                let (pattern, path_scope) = if selection.directory {
                    let directory = if entry.scope == PathAccessScope::Subtree {
                        entry.path.clone()
                    } else {
                        std::path::Path::new(&entry.path)
                            .parent()
                            .ok_or_else(|| "目标路径没有父目录".to_string())?
                            .to_string_lossy()
                            .into_owned()
                    };
                    (directory, PathAccessScope::Subtree)
                } else {
                    (entry.path.clone(), entry.scope)
                };
                Ok(PermissionRule {
                    id: String::new(),
                    capability: entry.capability,
                    pattern,
                    source: PatternSource::Path,
                    scope,
                    note: "由权限确认对话框保存".to_string(),
                    plan_bash: None,
                    external_path: Some(ExternalPathRule {
                        target: self.target.clone(),
                        scope: path_scope,
                    }),
                })
            })
            .collect()
    }
}

pub fn external_rule_matches(
    rule: &PermissionRule,
    target: &PermissionTarget,
    path: &FileAccessPath,
) -> bool {
    let Some(external) = &rule.external_path else {
        return false;
    };
    external.target == *target
        && rule.capability == path.capability
        && rule.source == PatternSource::Path
        && match external.scope {
            PathAccessScope::Exact => {
                path.scope == PathAccessScope::Exact && path.path == rule.pattern
            }
            PathAccessScope::Subtree => path_is_within(&rule.pattern, &path.path),
        }
}

/// Grants live only on the cloned execution context for one prepared call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedPath {
    pub path: String,
    pub scope: PathAccessScope,
    pub capability: PermissionCapability,
}

impl AuthorizedPath {
    pub fn permits(&self, path: &str, write: bool) -> bool {
        (!write || self.capability == PermissionCapability::Edit)
            && match self.scope {
                PathAccessScope::Exact => self.path == path,
                PathAccessScope::Subtree => path_is_within(&self.path, path),
            }
    }
}
