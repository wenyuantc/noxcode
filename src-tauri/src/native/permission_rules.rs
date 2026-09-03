//! 权限规则持久化与命令。
//!
//! 全局规则放 `$APPCONFIG/native-permissions.json`，工作区规则放
//! `<workspace>/.noxcode/permissions.json`（只对本地工作区生效）。会话启动时合并
//! 两份规则注入 `ToolCtx`；用户在确认对话框选择「总是允许」时追加一条并即时生效。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::app::shared::{new_id, sqlite_pool, EXECUTION_TARGET_LOCAL};
use crate::engine::context::resolve_workspace_execution_context_with_pool;
use crate::native::tools::permission::{PermissionRule, PermissionRules, RuleEffect, RuleScope};

pub const GLOBAL_RULES_FILE: &str = "native-permissions.json";
pub const WORKSPACE_RULES_DIR: &str = ".noxcode";
pub const WORKSPACE_RULES_FILE: &str = "permissions.json";
const RULES_VERSION: u32 = 1;
const MAX_RULES_PER_FILE: usize = 500;

/// 会话内共享的规则句柄：命令层追加规则后所有工具上下文立刻看到。
pub type SharedPermissionRules = Arc<RwLock<PermissionRules>>;

pub fn shared_rules(rules: PermissionRules) -> SharedPermissionRules {
    Arc::new(RwLock::new(rules))
}

#[derive(Debug, Serialize, Deserialize)]
struct RulesDocument {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    allow: Vec<PermissionRule>,
    #[serde(default)]
    deny: Vec<PermissionRule>,
    #[serde(default)]
    ask: Vec<PermissionRule>,
}

fn default_version() -> u32 {
    RULES_VERSION
}

impl RulesDocument {
    fn into_rules(self, scope: RuleScope) -> PermissionRules {
        let fix = |mut rule: PermissionRule| {
            rule.scope = scope;
            if rule.id.trim().is_empty() {
                rule.id = new_id();
            }
            rule
        };
        PermissionRules {
            allow: self.allow.into_iter().map(fix).collect(),
            deny: self.deny.into_iter().map(fix).collect(),
            ask: self.ask.into_iter().map(fix).collect(),
        }
    }

    fn from_rules(rules: &PermissionRules) -> Self {
        Self {
            version: RULES_VERSION,
            allow: rules.allow.clone(),
            deny: rules.deny.clone(),
            ask: rules.ask.clone(),
        }
    }
}

pub fn global_rules_path(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join(GLOBAL_RULES_FILE)
}

pub fn workspace_rules_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(WORKSPACE_RULES_DIR)
        .join(WORKSPACE_RULES_FILE)
}

fn read_rules_file(path: &Path, scope: RuleScope) -> Result<PermissionRules, String> {
    if !path.exists() {
        return Ok(PermissionRules::default());
    }
    let text = fs::read_to_string(path).map_err(|error| format!("读取权限规则失败: {error}"))?;
    if text.trim().is_empty() {
        return Ok(PermissionRules::default());
    }
    let document: RulesDocument =
        serde_json::from_str(&text).map_err(|error| format!("权限规则文件格式错误: {error}"))?;
    Ok(document.into_rules(scope))
}

fn write_rules_file(path: &Path, rules: &PermissionRules) -> Result<(), String> {
    if rules.len() > MAX_RULES_PER_FILE {
        return Err(format!("权限规则最多 {MAX_RULES_PER_FILE} 条"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建权限规则目录失败: {error}"))?;
    }
    let json = serde_json::to_string_pretty(&RulesDocument::from_rules(rules))
        .map_err(|error| format!("序列化权限规则失败: {error}"))?;
    fs::write(path, json).map_err(|error| format!("写入权限规则失败: {error}"))
}

pub fn load_global_rules(app_config_dir: &Path) -> Result<PermissionRules, String> {
    read_rules_file(&global_rules_path(app_config_dir), RuleScope::Global)
}

pub fn load_workspace_rules(workspace_root: &Path) -> Result<PermissionRules, String> {
    read_rules_file(&workspace_rules_path(workspace_root), RuleScope::Workspace)
}

/// 工作区规则优先于全局规则。读失败只打日志，不阻断会话。
pub fn load_effective_rules(
    app_config_dir: &Path,
    workspace_root: Option<&Path>,
) -> PermissionRules {
    let global = load_global_rules(app_config_dir).unwrap_or_else(|error| {
        eprintln!("[native] 读取全局权限规则失败: {error}");
        PermissionRules::default()
    });
    let workspace = workspace_root
        .map(|root| {
            load_workspace_rules(root).unwrap_or_else(|error| {
                eprintln!("[native] 读取工作区权限规则失败: {error}");
                PermissionRules::default()
            })
        })
        .unwrap_or_default();
    PermissionRules::merged(&global, &workspace)
}

fn normalize_rule(mut rule: PermissionRule) -> Result<PermissionRule, String> {
    rule.pattern = rule.pattern.trim().to_string();
    if rule.pattern.is_empty() {
        return Err("规则模式不能为空".to_string());
    }
    if rule.pattern.chars().count() > 512 {
        return Err("规则模式过长".to_string());
    }
    rule.note = rule.note.trim().chars().take(200).collect();
    if rule.id.trim().is_empty() {
        rule.id = new_id();
    }
    Ok(rule)
}

/// 追加一条规则到其 `scope` 对应的文件；返回带 id 的规则。
pub fn add_rule(
    app_config_dir: &Path,
    workspace_root: Option<&Path>,
    effect: RuleEffect,
    rule: PermissionRule,
) -> Result<PermissionRule, String> {
    let rule = normalize_rule(rule)?;
    match rule.scope {
        RuleScope::Global => {
            let path = global_rules_path(app_config_dir);
            let mut rules = read_rules_file(&path, RuleScope::Global)?;
            rules.push(effect, rule.clone());
            write_rules_file(&path, &rules)?;
        }
        RuleScope::Workspace => {
            let root = workspace_root
                .ok_or_else(|| "当前工作区不是本地目录，只能保存全局规则".to_string())?;
            let path = workspace_rules_path(root);
            let mut rules = read_rules_file(&path, RuleScope::Workspace)?;
            rules.push(effect, rule.clone());
            write_rules_file(&path, &rules)?;
        }
    }
    Ok(rule)
}

/// 从两份文件里删除 id 对应的规则。
pub fn delete_rule(
    app_config_dir: &Path,
    workspace_root: Option<&Path>,
    id: &str,
) -> Result<bool, String> {
    let mut removed = false;
    let global_path = global_rules_path(app_config_dir);
    let mut global = read_rules_file(&global_path, RuleScope::Global)?;
    if global.remove(id) {
        write_rules_file(&global_path, &global)?;
        removed = true;
    }
    if let Some(root) = workspace_root {
        let path = workspace_rules_path(root);
        let mut workspace = read_rules_file(&path, RuleScope::Workspace)?;
        if workspace.remove(id) {
            write_rules_file(&path, &workspace)?;
            removed = true;
        }
    }
    Ok(removed)
}

/// 整体覆盖某一作用域的规则文件（设置页保存）。
pub fn replace_rules(
    app_config_dir: &Path,
    workspace_root: Option<&Path>,
    scope: RuleScope,
    mut rules: PermissionRules,
) -> Result<PermissionRules, String> {
    let normalize_all = |list: Vec<PermissionRule>| -> Result<Vec<PermissionRule>, String> {
        list.into_iter()
            .map(|mut rule| {
                rule.scope = scope;
                normalize_rule(rule)
            })
            .collect()
    };
    rules.allow = normalize_all(rules.allow)?;
    rules.deny = normalize_all(rules.deny)?;
    rules.ask = normalize_all(rules.ask)?;
    let path = match scope {
        RuleScope::Global => global_rules_path(app_config_dir),
        RuleScope::Workspace => workspace_rules_path(
            workspace_root.ok_or_else(|| "当前工作区不是本地目录，只能保存全局规则".to_string())?,
        ),
    };
    write_rules_file(&path, &rules)?;
    Ok(rules)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativePermissionRulesView {
    pub global: PermissionRules,
    pub workspace: Option<PermissionRules>,
    pub workspace_root: Option<String>,
}

fn app_config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))
}

/// 本地工作区返回目录；SSH 工作区或未指定返回 `None`。
pub(crate) async fn local_workspace_root<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let Some(workspace_id) = workspace_id.map(str::trim).filter(|item| !item.is_empty()) else {
        return Ok(None);
    };
    let pool = sqlite_pool(app).await?;
    let context = resolve_workspace_execution_context_with_pool(&pool, workspace_id).await?;
    if context.execution_target != EXECUTION_TARGET_LOCAL {
        return Ok(None);
    }
    Ok(context.working_dir.map(PathBuf::from))
}

#[tauri::command]
pub async fn get_native_permission_rules<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
) -> Result<NativePermissionRulesView, String> {
    let config_dir = app_config_dir(&app)?;
    let root = local_workspace_root(&app, workspace_id.as_deref()).await?;
    Ok(NativePermissionRulesView {
        global: load_global_rules(&config_dir)?,
        workspace: match root.as_deref() {
            Some(root) => Some(load_workspace_rules(root)?),
            None => None,
        },
        workspace_root: root.map(|item| item.to_string_lossy().into_owned()),
    })
}

#[tauri::command]
pub async fn update_native_permission_rules<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
    scope: RuleScope,
    rules: PermissionRules,
) -> Result<NativePermissionRulesView, String> {
    let config_dir = app_config_dir(&app)?;
    let root = local_workspace_root(&app, workspace_id.as_deref()).await?;
    replace_rules(&config_dir, root.as_deref(), scope, rules)?;
    get_native_permission_rules(app, workspace_id).await
}

#[tauri::command]
pub async fn add_native_permission_rule<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
    effect: RuleEffect,
    rule: PermissionRule,
) -> Result<PermissionRule, String> {
    let config_dir = app_config_dir(&app)?;
    let root = local_workspace_root(&app, workspace_id.as_deref()).await?;
    add_rule(&config_dir, root.as_deref(), effect, rule)
}

#[tauri::command]
pub async fn delete_native_permission_rule<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
    id: String,
) -> Result<bool, String> {
    let config_dir = app_config_dir(&app)?;
    let root = local_workspace_root(&app, workspace_id.as_deref()).await?;
    delete_rule(&config_dir, root.as_deref(), &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::tools::contract::{PatternSource, PermissionCapability};

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}",
            crate::native::artifacts::unique_suffix()
        ));
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn rule(pattern: &str, scope: RuleScope) -> PermissionRule {
        PermissionRule {
            id: String::new(),
            capability: PermissionCapability::Bash,
            pattern: pattern.to_string(),
            source: PatternSource::Command,
            scope,
            note: String::new(),
        }
    }

    #[test]
    fn rules_round_trip_through_files_and_merge_workspace_first() {
        let config = temp_dir("noxcode-rules-config");
        let workspace = temp_dir("noxcode-rules-ws");
        let saved = add_rule(
            &config,
            Some(&workspace),
            RuleEffect::Allow,
            rule("git *", RuleScope::Global),
        )
        .expect("add global");
        assert!(!saved.id.is_empty());
        add_rule(
            &config,
            Some(&workspace),
            RuleEffect::Deny,
            rule("git push*", RuleScope::Workspace),
        )
        .expect("add workspace");
        assert!(workspace_rules_path(&workspace).exists());
        let effective = load_effective_rules(&config, Some(&workspace));
        assert_eq!(effective.allow.len(), 1);
        assert_eq!(effective.deny.len(), 1);
        assert_eq!(effective.deny[0].scope, RuleScope::Workspace);
        assert!(delete_rule(&config, Some(&workspace), &saved.id).expect("delete"));
        assert!(!delete_rule(&config, Some(&workspace), "missing").expect("delete"));
        let after = load_effective_rules(&config, Some(&workspace));
        assert!(after.allow.is_empty());
        let empty_pattern = add_rule(
            &config,
            None,
            RuleEffect::Allow,
            rule("  ", RuleScope::Global),
        );
        assert!(empty_pattern.is_err());
        let no_workspace = add_rule(
            &config,
            None,
            RuleEffect::Allow,
            rule("ls*", RuleScope::Workspace),
        );
        assert!(no_workspace.is_err());
        let _ = fs::remove_dir_all(config);
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn replace_rules_overwrites_scope_file() {
        let config = temp_dir("noxcode-rules-replace");
        let rules = PermissionRules {
            allow: vec![rule("npm*", RuleScope::Global)],
            deny: Vec::new(),
            ask: vec![rule("docker*", RuleScope::Workspace)],
        };
        let saved = replace_rules(&config, None, RuleScope::Global, rules).expect("replace");
        assert!(saved.ask.iter().all(|item| item.scope == RuleScope::Global));
        let loaded = load_global_rules(&config).expect("load");
        assert_eq!(loaded.allow.len(), 1);
        assert_eq!(loaded.ask.len(), 1);
        let _ = fs::remove_dir_all(config);
    }
}
