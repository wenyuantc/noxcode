use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contract::{PatternSource, PermissionCapability, ToolContract};
use super::file_access::{
    external_rule_matches, ExternalPathRule, FileAccessPrompt, PermissionTarget,
};
use super::glob::glob_match;
use super::patch::{extract_patch_text, parse_patch, patch_counts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeToolRiskKind {
    Overwrite,
    Delete,
    Push,
    ForceGit,
    Mcp,
    Opaque,
    /// 由 ask 规则强制要求确认，与工具本身的风险无关。
    Rule,
    /// 创建 / 删除定时自动化。
    Automation,
    ExternalPath,
    /// 本机已打开应用的后台电脑控制。
    Computer,
    NetworkOrigin,
    NetworkProxy,
}

impl NativeToolRiskKind {
    pub fn zh_label(self) -> &'static str {
        match self {
            Self::Overwrite => "覆盖",
            Self::Delete => "删除",
            Self::Push => "推送",
            Self::ForceGit => "强制 Git",
            Self::Mcp => "MCP",
            Self::Opaque => "不透明命令",
            Self::Rule => "权限规则",
            Self::Automation => "自动化",
            Self::ExternalPath => "工作区外访问",
            Self::Computer => "电脑控制",
            Self::NetworkOrigin => "非公网来源",
            Self::NetworkProxy => "代理信任",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeToolRisk {
    Low,
    High {
        kind: NativeToolRiskKind,
        summary: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePermissionDecision {
    AllowSession,
    /// 当前运行会话内的 Bash 命令免确认，不改变权限模式或持久化规则。
    AllowSessionCommands,
    AllowOnce,
    AllowServer,
    /// 允许并保存一条工作区 allow 规则（由 `suggested_rule` 推导）。
    AllowAlways,
    Deny,
}

pub fn classify_native_tool_risk(
    name: &str,
    arguments: &str,
    file_exists: Option<bool>,
    is_mcp: bool,
) -> NativeToolRisk {
    if is_mcp || name.starts_with("mcp_") {
        return NativeToolRisk::High {
            kind: NativeToolRiskKind::Mcp,
            summary: format!("调用 MCP 工具 {name}"),
        };
    }
    match name {
        "Edit" => NativeToolRisk::High {
            kind: NativeToolRiskKind::Overwrite,
            summary: format!("覆盖已有文件 {}", arg_string(arguments, "file_path")),
        },
        "ApplyPatch" => classify_apply_patch(arguments),
        "Write" => {
            if file_exists.unwrap_or(false) {
                NativeToolRisk::High {
                    kind: NativeToolRiskKind::Overwrite,
                    summary: format!("覆盖已有文件 {}", arg_string(arguments, "file_path")),
                }
            } else {
                NativeToolRisk::Low
            }
        }
        // Monitor 在后台执行同样的 shell 命令，按 Bash 分类。
        "Bash" | "Monitor" => classify_bash(&arg_string(arguments, "command")),
        "Computer" => NativeToolRisk::High {
            kind: NativeToolRiskKind::Computer,
            summary: super::desktop::risk_summary(arguments),
        },
        _ => NativeToolRisk::Low,
    }
}

fn classify_apply_patch(arguments: &str) -> NativeToolRisk {
    match extract_patch_text(arguments).and_then(|text| parse_patch(&text)) {
        Ok(actions) => {
            let counts = patch_counts(&actions);
            NativeToolRisk::High {
                kind: if counts.delete > 0 {
                    NativeToolRiskKind::Delete
                } else {
                    NativeToolRiskKind::Overwrite
                },
                summary: counts.summary(),
            }
        }
        Err(_) => NativeToolRisk::High {
            kind: NativeToolRiskKind::Overwrite,
            summary: "应用补丁（格式无法解析）".to_string(),
        },
    }
}

fn arg_string(arguments: &str, key: &str) -> String {
    serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "(unknown)".to_string())
}

pub fn classify_plan_bash_risk(arguments: &str) -> NativeToolRisk {
    let command = arg_string(arguments, "command");
    let analysis = analyze_bash(&command);
    if let Some((kind, summary)) = analysis.worst {
        return NativeToolRisk::High { kind, summary };
    }
    // Wrappers can write (nohup), change executable lookup (env), or change the
    // environment of the real command. Keep them subject to explicit plan approval.
    if analysis.wrapped {
        NativeToolRisk::High {
            kind: NativeToolRiskKind::Opaque,
            summary: format!("计划模式下需确认的命令：{command}"),
        }
    } else {
        NativeToolRisk::Low
    }
}

fn classify_bash(command: &str) -> NativeToolRisk {
    match analyze_bash(command).worst {
        Some((kind, summary)) => NativeToolRisk::High { kind, summary },
        None => NativeToolRisk::Low,
    }
}

/// 一条 Bash 命令的分类结果：最严重的风险，以及是否经过了包装命令。
struct BashAnalysis {
    worst: Option<(NativeToolRiskKind, String)>,
    wrapped: bool,
}

/// 按段分类。所有段都在只读白名单内、没有写文件的重定向、也没有无法解析的
/// 结构时才没有风险；本地与 SSH 使用同一套判断。
fn analyze_bash(command: &str) -> BashAnalysis {
    let script = super::shell_parse::parse(command);
    let mut worst: Option<(NativeToolRiskKind, String)> = None;
    let mut raise = |kind: NativeToolRiskKind, summary: String| {
        worst = Some(pick_worse(worst.take(), kind, summary));
    };
    if !script.opaque.is_empty() {
        raise(
            NativeToolRiskKind::Opaque,
            format!("不透明命令（{}）：{command}", script.opaque.join("、")),
        );
    }
    let mut wrapped = false;
    let mut previous: Option<&str> = None;
    for segment in &script.segments {
        if !segment.assigns.is_empty() {
            wrapped = true;
            raise(
                NativeToolRiskKind::Opaque,
                format!("设置环境变量后执行：{command}"),
            );
        }
        if segment.redirects.iter().any(|item| !item.is_harmless()) {
            raise(
                NativeToolRiskKind::Overwrite,
                format!("Shell 输出重定向：{command}"),
            );
        }
        let (start, via_wrapper) = super::bash_policy::unwrap_command(&segment.argv);
        wrapped |= via_wrapper;
        let argv = &segment.argv[start..];
        let Some(first) = argv.first() else {
            previous = None;
            continue;
        };
        let name = super::bash_policy::command_name(first);
        if segment.piped
            && matches!(name, "sh" | "bash" | "zsh" | "dash" | "ksh")
            && previous.is_some_and(|prev| matches!(prev, "curl" | "wget"))
        {
            raise(
                NativeToolRiskKind::Opaque,
                format!("管道灌 shell：{command}"),
            );
        }
        if let Some((kind, label)) = super::bash_policy::dangerous(argv) {
            raise(kind, format!("{label}：{command}"));
        } else if let Err(reason) = super::bash_policy::read_only(argv, segment.globbed) {
            raise(
                NativeToolRiskKind::Opaque,
                format!("未验证的命令（{reason}）：{command}"),
            );
        }
        previous = Some(name);
    }
    BashAnalysis { worst, wrapped }
}

fn pick_worse(
    current: Option<(NativeToolRiskKind, String)>,
    kind: NativeToolRiskKind,
    summary: String,
) -> (NativeToolRiskKind, String) {
    match current {
        None => (kind, summary),
        Some((existing, existing_summary)) => {
            if risk_rank(kind) >= risk_rank(existing) {
                (kind, summary)
            } else {
                (existing, existing_summary)
            }
        }
    }
}

fn risk_rank(kind: NativeToolRiskKind) -> u8 {
    match kind {
        NativeToolRiskKind::Rule => 0,
        NativeToolRiskKind::Automation => 1,
        NativeToolRiskKind::ExternalPath => 1,
        NativeToolRiskKind::Overwrite => 1,
        NativeToolRiskKind::Mcp => 2,
        NativeToolRiskKind::Opaque => 3,
        NativeToolRiskKind::Delete => 4,
        NativeToolRiskKind::Push => 5,
        NativeToolRiskKind::Computer => 5,
        NativeToolRiskKind::ForceGit => 6,
        NativeToolRiskKind::NetworkOrigin | NativeToolRiskKind::NetworkProxy => 5,
    }
}

// ---------------------------------------------------------------------------
// 规则层：按能力 × 模式匹配的 allow / deny / ask，deny 优先于 allow，allow 优先于 ask。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    #[default]
    Workspace,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEffect {
    Allow,
    Deny,
    Ask,
}

fn default_rule_source() -> PatternSource {
    PatternSource::ToolName
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanBashRule {
    pub target: PermissionTarget,
    pub workspace_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    #[serde(default)]
    pub id: String,
    pub capability: PermissionCapability,
    /// 匹配模式：路径 / 工具名 / 输入用 glob，命令用前缀（`git push*`）或精确匹配。
    pub pattern: String,
    #[serde(default = "default_rule_source")]
    pub source: PatternSource,
    #[serde(default)]
    pub scope: RuleScope,
    #[serde(default)]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_path: Option<ExternalPathRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_bash: Option<PlanBashRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRules {
    #[serde(default)]
    pub allow: Vec<PermissionRule>,
    #[serde(default)]
    pub deny: Vec<PermissionRule>,
    #[serde(default)]
    pub ask: Vec<PermissionRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleDecision {
    Allow(PermissionRule),
    Deny(PermissionRule),
    Ask(PermissionRule),
    NoMatch,
}

/// 「总是允许」对话框给出的规则建议。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRuleSuggestion {
    pub capability: PermissionCapability,
    pub pattern: String,
    pub source: PatternSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_bash: Option<PlanBashRule>,
}

impl PermissionRules {
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty() && self.ask.is_empty()
    }

    pub fn len(&self) -> usize {
        self.allow.len() + self.deny.len() + self.ask.len()
    }

    /// 工作区规则排在全局规则前面；同一效果内按顺序匹配。
    pub fn merged(global: &PermissionRules, workspace: &PermissionRules) -> PermissionRules {
        let mut out = workspace.clone();
        out.allow.extend(global.allow.iter().cloned());
        out.deny.extend(global.deny.iter().cloned());
        out.ask.extend(global.ask.iter().cloned());
        out
    }

    pub fn push(&mut self, effect: RuleEffect, rule: PermissionRule) {
        let list = match effect {
            RuleEffect::Allow => &mut self.allow,
            RuleEffect::Deny => &mut self.deny,
            RuleEffect::Ask => &mut self.ask,
        };
        // 同一效果下相同能力 + 模式 + 来源只保留一条。
        list.retain(|existing| {
            !(existing.capability == rule.capability
                && existing.pattern == rule.pattern
                && existing.source == rule.source)
                || existing.scope != rule.scope
                || existing.external_path != rule.external_path
                || existing.plan_bash != rule.plan_bash
        });
        list.push(rule);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.len();
        self.allow.retain(|rule| rule.id != id);
        self.deny.retain(|rule| rule.id != id);
        self.ask.retain(|rule| rule.id != id);
        before != self.len()
    }

    pub fn retain_scope(&mut self, scope: RuleScope) {
        self.allow.retain(|rule| rule.scope == scope);
        self.deny.retain(|rule| rule.scope == scope);
        self.ask.retain(|rule| rule.scope == scope);
    }

    /// deny → allow → ask → 未命中。
    pub fn evaluate(
        &self,
        contract: &ToolContract,
        tool_name: &str,
        arguments: &str,
        workspace_root: Option<&Path>,
    ) -> RuleDecision {
        let candidates = RuleCandidates::from_call(tool_name, arguments, workspace_root);
        if let Some(rule) = self
            .deny
            .iter()
            .find(|rule| rule_matches(rule, contract, &candidates, MatchMode::Restrict))
        {
            return RuleDecision::Deny(rule.clone());
        }
        if let Some(rule) = self
            .allow
            .iter()
            .find(|rule| rule_matches(rule, contract, &candidates, MatchMode::Allow))
        {
            return RuleDecision::Allow(rule.clone());
        }
        if let Some(rule) = self
            .ask
            .iter()
            .find(|rule| rule_matches(rule, contract, &candidates, MatchMode::Restrict))
        {
            return RuleDecision::Ask(rule.clone());
        }
        RuleDecision::NoMatch
    }

    pub fn evaluate_plan_bash(
        &self,
        contract: &ToolContract,
        arguments: &str,
        context: &PlanBashRule,
    ) -> RuleDecision {
        let candidates =
            RuleCandidates::from_call("Bash", arguments, Some(Path::new(&context.workspace_root)));
        let exact_match = |rule: &&PermissionRule| {
            rule.capability == PermissionCapability::Bash
                && rule.source == PatternSource::Command
                && rule.external_path.is_none()
                && rule.plan_bash.as_ref() == Some(context)
                && candidates.command.as_deref() == Some(rule.pattern.as_str())
        };
        let restriction_matches = |rule: &&PermissionRule| {
            exact_match(rule) || rule_matches(rule, contract, &candidates, MatchMode::Restrict)
        };
        if let Some(rule) = self.deny.iter().find(restriction_matches) {
            RuleDecision::Deny(rule.clone())
        } else if let Some(rule) = self.ask.iter().find(restriction_matches) {
            RuleDecision::Ask(rule.clone())
        } else if let Some(rule) = self.allow.iter().find(exact_match) {
            RuleDecision::Allow(rule.clone())
        } else {
            RuleDecision::NoMatch
        }
    }

    pub fn evaluate_file_access(
        &self,
        contract: &ToolContract,
        name: &str,
        arguments: &str,
        root: &Path,
        access: &FileAccessPrompt,
    ) -> Vec<RuleDecision> {
        let physical_root = if access.target == super::file_access::PermissionTarget::Local {
            super::paths::resolve_local_path(root, ".").unwrap_or_else(|_| root.to_path_buf())
        } else {
            root.to_path_buf()
        };
        access
            .paths
            .iter()
            .map(|path| {
                let mut candidates = RuleCandidates::from_call(name, arguments, Some(root));
                candidates.paths = vec![
                    path.requested_path.clone(),
                    relative_display_path(&path.requested_path, Some(root)),
                    relative_display_path(&path.path, Some(&physical_root)),
                    path.path.clone(),
                ];
                let matches = |mode: MatchMode| {
                    let candidates = &candidates;
                    move |rule: &&PermissionRule| {
                        if rule.external_path.is_some() {
                            external_rule_matches(rule, &access.target, path)
                        } else {
                            rule_matches(rule, contract, candidates, mode)
                        }
                    }
                };
                if let Some(rule) = self.deny.iter().find(matches(MatchMode::Restrict)) {
                    RuleDecision::Deny(rule.clone())
                } else if let Some(rule) = self.allow.iter().find(matches(MatchMode::Allow)) {
                    RuleDecision::Allow(rule.clone())
                } else if let Some(rule) = self.ask.iter().find(matches(MatchMode::Restrict)) {
                    RuleDecision::Ask(rule.clone())
                } else {
                    RuleDecision::NoMatch
                }
            })
            .collect()
    }
}

/// 命令中的一段。`text` 含环境变量前缀和包装命令，`core` 去掉了它们。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandSegment {
    pub text: String,
    pub core: String,
    /// 有写文件的重定向。
    pub writes: bool,
}

impl CommandSegment {
    fn from_segment(segment: &super::shell_parse::Segment) -> Self {
        let (start, _) = super::bash_policy::unwrap_command(&segment.argv);
        let text = segment
            .assigns
            .iter()
            .chain(segment.argv.iter())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            text,
            core: segment.argv[start..].join(" "),
            writes: segment.redirects.iter().any(|item| !item.is_harmless()),
        }
    }
}

/// allow 规则要求命令的每一段都匹配；deny / ask 规则任意一段匹配即生效。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    Allow,
    Restrict,
}

/// 从一次调用里抽出的可匹配字段。
#[derive(Debug, Clone, Default)]
pub struct RuleCandidates {
    pub tool_name: String,
    pub command: Option<String>,
    /// 命令按段拆开后的形式，供命令规则逐段匹配。
    pub command_segments: Vec<CommandSegment>,
    /// 命令含无法解析的结构（展开、替换、子 shell 等）。
    pub command_opaque: bool,
    pub paths: Vec<String>,
    pub input: String,
    pub apps: Vec<String>,
    pub action: Option<String>,
}

impl RuleCandidates {
    pub fn from_call(tool_name: &str, arguments: &str, workspace_root: Option<&Path>) -> Self {
        let args = serde_json::from_str::<Value>(arguments).unwrap_or(Value::Null);
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty());
        let mut paths = Vec::new();
        for key in ["file_path", "path"] {
            if let Some(path) = args.get(key).and_then(Value::as_str) {
                let trimmed = path.trim();
                if !trimmed.is_empty() {
                    paths.push(relative_display_path(trimmed, workspace_root));
                }
            }
        }
        if tool_name == "ApplyPatch" {
            if let Ok(text) = extract_patch_text(arguments) {
                if let Ok(actions) = parse_patch(&text) {
                    for action in actions {
                        match action {
                            super::patch::PatchAction::Add { path, .. }
                            | super::patch::PatchAction::Delete { path } => {
                                paths.push(relative_display_path(&path, workspace_root));
                            }
                            super::patch::PatchAction::Update { path, move_to, .. } => {
                                paths.push(relative_display_path(&path, workspace_root));
                                if let Some(dest) = move_to {
                                    paths.push(relative_display_path(&dest, workspace_root));
                                }
                            }
                        }
                    }
                }
            }
        }
        let apps = args
            .get("app")
            .and_then(Value::as_str)
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .map(|item| vec![item])
            .unwrap_or_default();
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty());
        let script = command.as_deref().map(super::shell_parse::parse);
        let command_opaque = script
            .as_ref()
            .is_some_and(|script| !script.opaque.is_empty());
        let command_segments = script
            .map(|script| {
                script
                    .segments
                    .iter()
                    .map(CommandSegment::from_segment)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            tool_name: tool_name.to_string(),
            command,
            command_segments,
            command_opaque,
            paths,
            input: arguments.to_string(),
            apps,
            action,
        }
    }
}

/// 把路径统一成相对工作区、正斜杠的形式，便于 glob 匹配。
pub fn relative_display_path(path: &str, workspace_root: Option<&Path>) -> String {
    let normalized = path.replace('\\', "/");
    let Some(root) = workspace_root else {
        return normalized.trim_start_matches("./").to_string();
    };
    let root_text = root.to_string_lossy().replace('\\', "/");
    let root_text = root_text.trim_end_matches('/');
    if normalized == root_text {
        return ".".to_string();
    }
    if let Some(rest) = normalized.strip_prefix(&format!("{root_text}/")) {
        return rest.to_string();
    }
    normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .to_string()
}

fn rule_matches(
    rule: &PermissionRule,
    contract: &ToolContract,
    candidates: &RuleCandidates,
    mode: MatchMode,
) -> bool {
    if rule.external_path.is_some()
        || rule.plan_bash.is_some()
        || rule.capability != contract.permission
    {
        return false;
    }
    let pattern = rule.pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    match rule.source {
        PatternSource::ToolName => glob_or_exact(pattern, &candidates.tool_name),
        PatternSource::Command => candidates
            .command
            .as_deref()
            .is_some_and(|command| command_rule_matches(pattern, command, candidates, mode)),
        PatternSource::Path => candidates
            .paths
            .iter()
            .any(|path| glob_or_exact(pattern, path)),
        PatternSource::Input => {
            glob_or_exact(pattern, &candidates.input)
                || candidates
                    .apps
                    .iter()
                    .any(|app| computer_app_matches(pattern, app))
                || candidates
                    .action
                    .as_deref()
                    .is_some_and(|action| glob_or_exact(pattern, action))
        }
    }
}

pub fn computer_app_matches(pattern: &str, candidate: &str) -> bool {
    crate::native::tools::app_target::identity_matches(pattern, candidate)
        || glob_or_exact(pattern.trim(), candidate.trim())
}

fn glob_or_exact(pattern: &str, candidate: &str) -> bool {
    candidate == pattern || glob_match(pattern, candidate)
}

/// 命令规则与整条命令的匹配。整条命令与模式完全相同时总是命中。
/// allow：每一段都要匹配，且不能有写文件的重定向或无法解析的结构，
/// 避免 `git status*` 放行 `git status; rm -rf x`。
/// deny / ask：整条命令或任意一段（含去掉包装后的形式）匹配即命中。
fn command_rule_matches(
    pattern: &str,
    command: &str,
    candidates: &RuleCandidates,
    mode: MatchMode,
) -> bool {
    if pattern == "*" || pattern == command.trim() {
        return true;
    }
    let segments = &candidates.command_segments;
    match mode {
        MatchMode::Allow => {
            !candidates.command_opaque
                && !segments.is_empty()
                && segments.iter().all(|segment| {
                    !segment.writes && command_pattern_matches(pattern, &segment.text)
                })
        }
        MatchMode::Restrict => {
            command_pattern_matches(pattern, command)
                || segments.iter().any(|segment| {
                    command_pattern_matches(pattern, &segment.text)
                        || command_pattern_matches(pattern, &segment.core)
                })
        }
    }
}

/// 单段命令模式：`git push*` 前缀匹配（按空白切词后逐词比较），否则精确匹配。
pub fn command_pattern_matches(pattern: &str, command: &str) -> bool {
    let command = command.trim();
    if pattern == command || pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        let prefix = prefix.trim_end();
        if prefix.is_empty() {
            return true;
        }
        let prefix_tokens: Vec<&str> = prefix.split_whitespace().collect();
        let command_tokens: Vec<&str> = command.split_whitespace().collect();
        if command_tokens.len() < prefix_tokens.len() {
            return false;
        }
        return prefix_tokens
            .iter()
            .zip(command_tokens.iter())
            .all(|(a, b)| a == b);
    }
    glob_match(pattern, command)
}

/// 根据一次待确认的调用推导「总是允许」规则：单段 Bash 用前两个词做前缀
/// （复合命令用整条命令精确匹配），
/// 文件工具用相对路径，其余用工具名。
pub fn suggest_rule(
    contract: &ToolContract,
    tool_name: &str,
    arguments: &str,
    workspace_root: Option<&Path>,
) -> Option<PermissionRuleSuggestion> {
    let candidates = RuleCandidates::from_call(tool_name, arguments, workspace_root);
    match contract.permission {
        PermissionCapability::Bash => {
            let command = candidates.command.clone()?;
            // 复合命令、无法解析或带写文件重定向的命令只给精确规则，不生成通配。
            let single = match candidates.command_segments.as_slice() {
                [segment] if !candidates.command_opaque && !segment.writes => Some(segment),
                _ => None,
            };
            let pattern = match single {
                Some(segment) => {
                    let tokens: Vec<&str> = segment.text.split_whitespace().collect();
                    let first = *tokens.first()?;
                    match tokens.get(1) {
                        Some(second) if !second.starts_with('-') => format!("{first} {second}*"),
                        _ => format!("{first}*"),
                    }
                }
                None => command,
            };
            Some(PermissionRuleSuggestion {
                capability: PermissionCapability::Bash,
                pattern,
                source: PatternSource::Command,
                plan_bash: None,
            })
        }
        PermissionCapability::Edit if tool_name != "ApplyPatch" => {
            let path = candidates.paths.first()?.clone();
            Some(PermissionRuleSuggestion {
                capability: PermissionCapability::Edit,
                pattern: path,
                source: PatternSource::Path,
                plan_bash: None,
            })
        }
        PermissionCapability::Computer => {
            let pattern = candidates
                .apps
                .first()
                .cloned()
                .or_else(|| candidates.action.clone())
                .unwrap_or_else(|| tool_name.to_string());
            Some(PermissionRuleSuggestion {
                capability: PermissionCapability::Computer,
                pattern,
                source: PatternSource::Input,
                plan_bash: None,
            })
        }
        capability => Some(PermissionRuleSuggestion {
            capability,
            pattern: tool_name.to_string(),
            source: PatternSource::ToolName,
            plan_bash: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_command_grants_are_literal_and_bound_to_the_execution_context() {
        let context = PlanBashRule {
            target: PermissionTarget::Local,
            workspace_root: "/project".into(),
        };
        let command = "ls -la \"$HOME/Application Support/\" 2>/dev/null | head -30";
        let mut allowed = rule(PermissionCapability::Bash, command, PatternSource::Command);
        allowed.plan_bash = Some(context.clone());
        let mut rules = PermissionRules {
            allow: vec![allowed],
            ..Default::default()
        };
        let contract = super::super::contract::builtin_contract("Bash").unwrap();
        let args = |command: &str| serde_json::json!({"command":command}).to_string();
        assert!(matches!(
            rules.evaluate_plan_bash(contract, &args(command), &context),
            RuleDecision::Allow(_)
        ));
        for changed in [
            format!("{command}; touch bad"),
            command.replace("head -30", "head -50"),
        ] {
            assert_eq!(
                rules.evaluate_plan_bash(contract, &args(&changed), &context),
                RuleDecision::NoMatch
            );
        }
        for other in [
            PlanBashRule {
                workspace_root: "/other".into(),
                ..context.clone()
            },
            PlanBashRule {
                target: PermissionTarget::Ssh {
                    config_id: "ssh".into(),
                    host: "host".into(),
                    port: 22,
                    username: "user".into(),
                },
                ..context.clone()
            },
        ] {
            assert_eq!(
                rules.evaluate_plan_bash(contract, &args(command), &other),
                RuleDecision::NoMatch
            );
        }
        // Shell glob characters belong to the approved command, not the grant syntax.
        rules.allow[0].pattern = "ls *.txt".into();
        assert_eq!(
            rules.evaluate_plan_bash(contract, &args("ls private.txt"), &context),
            RuleDecision::NoMatch
        );
        assert!(matches!(
            rules.evaluate_plan_bash(contract, &args("ls *.txt"), &context),
            RuleDecision::Allow(_)
        ));
        rules.ask.push(rule(
            PermissionCapability::Bash,
            "ls*",
            PatternSource::Command,
        ));
        assert!(matches!(
            rules.evaluate_plan_bash(contract, &args("ls *.txt"), &context),
            RuleDecision::Ask(_)
        ));
        rules.deny.push(rule(
            PermissionCapability::Bash,
            "ls*",
            PatternSource::Command,
        ));
        assert!(matches!(
            rules.evaluate_plan_bash(contract, &args("ls *.txt"), &context),
            RuleDecision::Deny(_)
        ));
        rules.deny.clear();
        rules.ask.clear();
        rules.allow[0].plan_bash = None;
        assert_eq!(
            rules.evaluate_plan_bash(contract, &args("ls *.txt"), &context),
            RuleDecision::NoMatch
        );
    }

    #[test]
    fn plan_shell_only_auto_allows_verified_read_commands() {
        for command in [
            "pwd",
            "ls -la",
            "cat Cargo.toml | head -n 20",
            "cd src && wc -l main.rs",
            "git diff",
            "git status",
        ] {
            assert_eq!(
                classify_plan_bash_risk(&serde_json::json!({"command":command}).to_string()),
                NativeToolRisk::Low,
                "{command}"
            );
        }
        for command in [
            "touch file",
            "mkdir dir",
            "chmod 600 file",
            "rm file",
            "echo text > file",
            "echo text >> file",
            "sed -i '' s/a/b/ file",
            "find . -delete",
            "find . -exec touch file ;",
            "python3 read.py",
            "node -e 'write()'",
            "sqlite3 logs.db 'DELETE FROM logs'",
            "npm test",
            "curl -o file https://example.com",
            "nohup cat file",
            "env PATH=/tmp cat file",
            "git push",
            "git reset --hard",
            "echo $(touch file)",
            "echo #'\ntouch file\n#'",
        ] {
            assert!(
                matches!(
                    classify_plan_bash_risk(&serde_json::json!({"command":command}).to_string()),
                    NativeToolRisk::High { .. }
                ),
                "{command}"
            );
        }
    }

    fn rule(
        capability: PermissionCapability,
        pattern: &str,
        source: PatternSource,
    ) -> PermissionRule {
        PermissionRule {
            id: format!("{pattern}-{}", pattern.len()),
            external_path: None,
            plan_bash: None,
            capability,
            pattern: pattern.to_string(),
            source,
            scope: RuleScope::Workspace,
            note: String::new(),
        }
    }

    #[test]
    fn rules_prefer_deny_then_allow_then_ask() {
        let bash = super::super::contract::builtin_contract("Bash")
            .expect("bash")
            .clone();
        let mut rules = PermissionRules::default();
        rules.push(
            RuleEffect::Allow,
            rule(PermissionCapability::Bash, "git *", PatternSource::Command),
        );
        rules.push(
            RuleEffect::Deny,
            rule(
                PermissionCapability::Bash,
                "git push*",
                PatternSource::Command,
            ),
        );
        rules.push(
            RuleEffect::Ask,
            rule(PermissionCapability::Bash, "npm*", PatternSource::Command),
        );
        let status = rules.evaluate(&bash, "Bash", r#"{"command":"git status"}"#, None);
        assert!(matches!(status, RuleDecision::Allow(_)));
        let push = rules.evaluate(&bash, "Bash", r#"{"command":"git push origin main"}"#, None);
        assert!(matches!(push, RuleDecision::Deny(_)));
        let npm = rules.evaluate(&bash, "Bash", r#"{"command":"npm test"}"#, None);
        assert!(matches!(npm, RuleDecision::Ask(_)));
        let other = rules.evaluate(&bash, "Bash", r#"{"command":"ls"}"#, None);
        assert_eq!(other, RuleDecision::NoMatch);
        // 能力不同不匹配。
        let write = super::super::contract::builtin_contract("Write")
            .expect("write")
            .clone();
        assert_eq!(
            rules.evaluate(&write, "Write", r#"{"file_path":"git"}"#, None),
            RuleDecision::NoMatch
        );
    }

    #[test]
    fn path_rules_match_relative_globs_and_apply_patch_paths() {
        let write = super::super::contract::builtin_contract("Write")
            .expect("write")
            .clone();
        let patch = super::super::contract::builtin_contract("ApplyPatch")
            .expect("patch")
            .clone();
        let mut rules = PermissionRules::default();
        rules.push(
            RuleEffect::Allow,
            rule(PermissionCapability::Edit, "src/**", PatternSource::Path),
        );
        let root = Path::new("/repo");
        let ok = rules.evaluate(
            &write,
            "Write",
            r#"{"file_path":"/repo/src/lib/a.rs","content":"x"}"#,
            Some(root),
        );
        assert!(matches!(ok, RuleDecision::Allow(_)));
        let outside = rules.evaluate(
            &write,
            "Write",
            r#"{"file_path":"README.md","content":"x"}"#,
            Some(root),
        );
        assert_eq!(outside, RuleDecision::NoMatch);
        let patch_args =
            r#"{"patch":"*** Begin Patch\n*** Add File: src/new.rs\n+hi\n*** End Patch"}"#;
        let via_patch = rules.evaluate(&patch, "ApplyPatch", patch_args, Some(root));
        assert!(matches!(via_patch, RuleDecision::Allow(_)));
    }

    #[test]
    fn merged_rules_put_workspace_first_and_dedupe_on_push() {
        let global = PermissionRules {
            allow: vec![rule(
                PermissionCapability::Mcp,
                "mcp_*",
                PatternSource::ToolName,
            )],
            ..PermissionRules::default()
        };
        let mut workspace = PermissionRules::default();
        workspace.push(
            RuleEffect::Deny,
            rule(
                PermissionCapability::Mcp,
                "mcp_x_*",
                PatternSource::ToolName,
            ),
        );
        workspace.push(
            RuleEffect::Deny,
            rule(
                PermissionCapability::Mcp,
                "mcp_x_*",
                PatternSource::ToolName,
            ),
        );
        assert_eq!(workspace.deny.len(), 1);
        let merged = PermissionRules::merged(&global, &workspace);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.deny[0].pattern, "mcp_x_*");
        let mut removable = merged.clone();
        assert!(removable.remove(&workspace.deny[0].id));
        assert_eq!(removable.len(), 1);
    }

    #[test]
    fn suggested_rules_follow_tool_shape() {
        let bash = super::super::contract::builtin_contract("Bash")
            .expect("bash")
            .clone();
        let suggestion = suggest_rule(&bash, "Bash", r#"{"command":"git push origin main"}"#, None)
            .expect("suggestion");
        assert_eq!(suggestion.pattern, "git push*");
        assert_eq!(suggestion.source, PatternSource::Command);
        let single =
            suggest_rule(&bash, "Bash", r#"{"command":"rm -rf dist"}"#, None).expect("suggestion");
        assert_eq!(single.pattern, "rm*");
        let edit = super::super::contract::builtin_contract("Edit")
            .expect("edit")
            .clone();
        let path_rule = suggest_rule(
            &edit,
            "Edit",
            r#"{"file_path":"/repo/src/a.rs","old_string":"a","new_string":"b"}"#,
            Some(Path::new("/repo")),
        )
        .expect("suggestion");
        assert_eq!(path_rule.pattern, "src/a.rs");
        assert_eq!(path_rule.source, PatternSource::Path);
        let mcp = ToolContract::for_mcp("mcp_demo_run", false, true);
        let mcp_rule = suggest_rule(&mcp, "mcp_demo_run", "{}", None).expect("suggestion");
        assert_eq!(mcp_rule.pattern, "mcp_demo_run");
        assert_eq!(mcp_rule.capability, PermissionCapability::Mcp);
    }

    #[test]
    fn command_pattern_prefix_semantics() {
        assert!(command_pattern_matches("git push*", "git push origin"));
        assert!(!command_pattern_matches("git push*", "git pushy"));
        assert!(command_pattern_matches("git*", "git status"));
        assert!(command_pattern_matches("*", "anything"));
        assert!(command_pattern_matches("ls -la", "ls -la"));
        assert!(!command_pattern_matches("ls -la", "ls"));
    }

    #[test]
    fn write_new_file_is_low_existing_is_high() {
        assert_eq!(
            classify_native_tool_risk("Write", r#"{"file_path":"a.rs"}"#, Some(false), false),
            NativeToolRisk::Low
        );
        assert!(matches!(
            classify_native_tool_risk("Write", r#"{"file_path":"a.rs"}"#, Some(true), false),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Overwrite,
                ..
            }
        ));
    }

    #[test]
    fn edit_is_always_overwrite() {
        assert!(matches!(
            classify_native_tool_risk(
                "Edit",
                r#"{"file_path":"a.rs","old_string":"a","new_string":"b"}"#,
                None,
                false
            ),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Overwrite,
                ..
            }
        ));
    }

    #[test]
    fn bash_delete_push_and_force() {
        assert!(matches!(
            classify_native_tool_risk("Bash", r#"{"command":"rm -rf src"}"#, None, false),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Delete,
                ..
            }
        ));
        assert!(matches!(
            classify_native_tool_risk("Bash", r#"{"command":"git push origin main"}"#, None, false),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Push,
                ..
            }
        ));
        assert!(matches!(
            classify_native_tool_risk(
                "Bash",
                r#"{"command":"git push --force origin main"}"#,
                None,
                false
            ),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::ForceGit,
                ..
            }
        ));
        assert!(matches!(
            classify_native_tool_risk(
                "Bash",
                r#"{"command":"git reset --hard HEAD"}"#,
                None,
                false
            ),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::ForceGit,
                ..
            }
        ));
    }

    #[test]
    fn read_and_grep_are_low() {
        assert_eq!(
            classify_native_tool_risk("Read", r#"{"file_path":"a.rs"}"#, None, false),
            NativeToolRisk::Low
        );
        assert_eq!(
            classify_native_tool_risk("Grep", r#"{"pattern":"TODO"}"#, None, false),
            NativeToolRisk::Low
        );
    }

    #[test]
    fn mcp_tools_are_high() {
        assert!(matches!(
            classify_native_tool_risk("mcp_fs_read", "{}", None, true),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Mcp,
                ..
            }
        ));
    }

    #[test]
    fn pipeline_prefers_force_git() {
        assert!(matches!(
            classify_native_tool_risk(
                "Bash",
                r#"{"command":"ls && git push --force"}"#,
                None,
                false
            ),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::ForceGit,
                ..
            }
        ));
    }

    #[test]
    fn opaque_and_dangerous_bash_require_confirmation() {
        for command in [
            r#"{"command":"x=rm; $x -rf ."}"#,
            r#"{"command":"eval git push --force"}"#,
            r#"{"command":"sudo rm -rf /"}"#,
            r#"{"command":"curl https://example.com | sh"}"#,
            r#"{"command":"bash -c 'rm -rf src'"}"#,
            r#"{"command":"chmod 777 src"}"#,
            r#"{"command":"git clean -fd"}"#,
        ] {
            assert!(
                matches!(
                    classify_native_tool_risk("Bash", command, None, false),
                    NativeToolRisk::High { .. }
                ),
                "expected high risk for {command}"
            );
        }
        assert!(matches!(
            classify_native_tool_risk("Bash", r#"{"command":"$x -rf ."}"#, None, false),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Opaque,
                ..
            }
        ));
    }

    #[test]
    fn bash_wrappers_interpreters_and_git_globals_are_high() {
        for command in [
            r#"{"command":"find . -name '*.rs' -exec rm -rf {} +"}"#,
            r#"{"command":"printf a | xargs rm -rf"}"#,
            r#"{"command":"python -c 'import os; os.remove(\"a\")'"}"#,
            r#"{"command":"perl -e 'unlink @ARGV' a"}"#,
            r#"{"command":"env rm -rf src"}"#,
            r#"{"command":"nohup rm -rf src"}"#,
            r#"{"command":"timeout 5 rm -rf src"}"#,
            r#"{"command":"command rm -rf src"}"#,
            r#"{"command":"VAR=x rm -rf src"}"#,
            r#"{"command":"\\rm -rf src"}"#,
            r#"{"command":"git -c push.default=simple push --force origin main"}"#,
            r#"{"command":"echo ${HOME} && rm -rf src"}"#,
        ] {
            assert!(
                matches!(
                    classify_native_tool_risk("Bash", command, None, false),
                    NativeToolRisk::High { .. }
                ),
                "expected high risk for {command}"
            );
        }
        assert_eq!(
            classify_native_tool_risk("Bash", r#"{"command":"echo hello"}"#, None, false),
            NativeToolRisk::Low
        );
        assert_eq!(
            classify_native_tool_risk("Bash", r#"{"command":"git status"}"#, None, false),
            NativeToolRisk::Low
        );
    }

    #[test]
    fn bash_unknown_commands_and_overwrites_are_not_low_risk() {
        for command in [
            "git restore .",
            "git restore file.txt",
            "printf replacement > existing.txt",
            "echo replacement>>existing.txt",
            "cp a b",
            "mv a b",
            "./custom-script",
            "npm test",
            "python script.py",
            "sed -i '' s/a/b/ file",
            "git diff --output=result",
            "git -c core.fsmonitor=./script status",
            "/tmp/ls",
            "rg --pre ./script x",
        ] {
            assert!(
                matches!(classify_bash(command), NativeToolRisk::High { .. }),
                "{command}"
            );
        }
        for command in [
            "echo hello",
            "printf hello",
            "git status --short",
            "git diff --stat",
            "ls -la",
            "pwd && cat README.md",
        ] {
            assert_eq!(classify_bash(command), NativeToolRisk::Low, "{command}");
        }
    }

    #[test]
    fn apply_patch_delete_is_high_delete_otherwise_overwrite() {
        let delete = r#"{"patch":"*** Begin Patch\n*** Delete File: gone.txt\n*** End Patch"}"#;
        assert!(matches!(
            classify_native_tool_risk("ApplyPatch", delete, None, false),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Delete,
                ..
            }
        ));
        let add = r#"{"patch":"*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch"}"#;
        assert!(matches!(
            classify_native_tool_risk("ApplyPatch", add, None, false),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Overwrite,
                ..
            }
        ));
        assert!(matches!(
            classify_native_tool_risk("ApplyPatch", r#"{"patch":"nope"}"#, None, false),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Overwrite,
                summary,
            } if summary.contains("无法解析")
        ));
        assert_eq!(
            classify_native_tool_risk("Skill", r#"{"name":"demo"}"#, None, false),
            NativeToolRisk::Low
        );
        assert!(matches!(
            classify_native_tool_risk(
                "Computer",
                r#"{"action":"click","app":"Safari","element_index":10}"#,
                None,
                false
            ),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Computer,
                summary,
            } if summary.contains("点击") && summary.contains("Safari") && summary.contains("10")
        ));
    }

    #[test]
    fn computer_allow_rule_matches_app_identity() {
        let contract = super::super::contract::builtin_contract("Computer").expect("computer");
        let mut rules = PermissionRules::default();
        rules.push(
            RuleEffect::Allow,
            PermissionRule {
                id: "safari".into(),
                capability: PermissionCapability::Computer,
                pattern: "com.apple.Safari".into(),
                source: PatternSource::Input,
                scope: RuleScope::Workspace,
                note: String::new(),
                external_path: None,
                plan_bash: None,
            },
        );
        assert!(matches!(
            rules.evaluate(
                contract,
                "Computer",
                r#"{"action":"click","app":"Safari","element_index":0}"#,
                None
            ),
            RuleDecision::Allow(_)
        ));
        assert_eq!(
            rules.evaluate(
                contract,
                "Computer",
                r#"{"action":"click","app":"Notes","element_index":0}"#,
                None
            ),
            RuleDecision::NoMatch
        );
        let suggestion = suggest_rule(
            contract,
            "Computer",
            r#"{"action":"get_app_state","app":"com.apple.Safari"}"#,
            None,
        )
        .expect("suggestion");
        assert_eq!(suggestion.pattern, "com.apple.Safari");
        assert_eq!(suggestion.source, PatternSource::Input);
        assert_eq!(suggestion.capability, PermissionCapability::Computer);
    }

    fn plan_risk(command: &str) -> NativeToolRisk {
        classify_plan_bash_risk(&serde_json::json!({ "command": command }).to_string())
    }

    fn risk_kind(command: &str) -> Option<NativeToolRiskKind> {
        match classify_bash(command) {
            NativeToolRisk::Low => None,
            NativeToolRisk::High { kind, .. } => Some(kind),
        }
    }

    #[test]
    fn git_and_gh_read_only_rules_follow_subcommands_and_options() {
        for command in [
            "git -C sub status",
            "git --no-pager log --oneline -n 5",
            "git show HEAD:src/main.rs",
            "git branch",
            "git branch -a -v",
            "git tag -l",
            "git config --get user.name",
            "git remote -v",
            "git stash list",
            "git grep -n TODO",
            "git blame src/lib.rs",
            "gh pr view 12",
            "gh pr list --json title",
            "gh issue list",
            "gh api repos/o/r/pulls",
            "gh api -X GET repos/o/r",
            "gh auth status",
        ] {
            assert_eq!(plan_risk(command), NativeToolRisk::Low, "{command}");
        }
        for (command, kind) in [
            ("git branch new-feature", NativeToolRiskKind::Opaque),
            ("git tag v1", NativeToolRiskKind::Opaque),
            ("git config user.name x", NativeToolRiskKind::Opaque),
            ("git remote add origin url", NativeToolRiskKind::Opaque),
            ("git stash", NativeToolRiskKind::Opaque),
            ("git grep -O vim TODO", NativeToolRiskKind::Opaque),
            ("git -c core.pager=./x log", NativeToolRiskKind::Opaque),
            ("git log --output=out.txt", NativeToolRiskKind::Opaque),
            ("git diff --no-index a b", NativeToolRiskKind::Opaque),
            ("git commit -m msg", NativeToolRiskKind::Opaque),
            ("git push -f", NativeToolRiskKind::ForceGit),
            ("git -c a=b push --force", NativeToolRiskKind::ForceGit),
            ("git checkout -f main", NativeToolRiskKind::ForceGit),
            ("gh pr create --fill", NativeToolRiskKind::Push),
            ("gh pr merge 1", NativeToolRiskKind::Push),
            ("gh api -X POST repos/o/r/issues", NativeToolRiskKind::Push),
            (
                "gh api repos/o/r/issues -f title=x",
                NativeToolRiskKind::Push,
            ),
            ("gh pr view 1 --web", NativeToolRiskKind::Opaque),
            ("gh auth token", NativeToolRiskKind::Opaque),
        ] {
            assert_eq!(risk_kind(command), Some(kind), "{command}");
            assert!(
                matches!(plan_risk(command), NativeToolRisk::High { .. }),
                "{command}"
            );
        }
    }

    #[test]
    fn shell_syntax_is_parsed_instead_of_guessed() {
        for command in [
            "echo '(x)' '{y}'",
            "grep \"a b\" src/main.rs",
            "ls 2>/dev/null",
            "cat a.txt 2>&1 | head -n 5",
            "cat a | grep x | wc -l",
            "ls *.rs",
            "find . -name '*.rs'",
            "sed -n 1,20p file.txt",
            "sed -n '$p' file.txt",
            "rg -n TODO src",
            "sort -n data.txt",
            "tail -n 50 app.log",
            "cd src && ls; pwd",
        ] {
            assert_eq!(risk_kind(command), None, "{command}");
            assert_eq!(plan_risk(command), NativeToolRisk::Low, "{command}");
        }
        for (command, kind) in [
            ("ls > out.txt", NativeToolRiskKind::Overwrite),
            ("echo x >> notes.md", NativeToolRiskKind::Overwrite),
            ("cat a &> all.log", NativeToolRiskKind::Overwrite),
            ("echo $(touch f)", NativeToolRiskKind::Opaque),
            ("echo `touch f`", NativeToolRiskKind::Opaque),
            ("(cd src && rm -rf build)", NativeToolRiskKind::Delete),
            ("ls; rm -rf x", NativeToolRiskKind::Delete),
            ("ls && git push", NativeToolRiskKind::Push),
            // heredoc 无法静态确定，按现有排序不透明高于覆盖。
            ("cat <<EOF > f\nx\nEOF", NativeToolRiskKind::Opaque),
            ("tail -f app.log", NativeToolRiskKind::Opaque),
            ("sort -o out.txt data.txt", NativeToolRiskKind::Opaque),
            ("sort -no out.txt data.txt", NativeToolRiskKind::Opaque),
            ("sed -i s/a/b/ file", NativeToolRiskKind::Opaque),
            ("sed -n /x/w\\ out file", NativeToolRiskKind::Opaque),
            ("rg --pre=./x TODO", NativeToolRiskKind::Opaque),
            ("rg TODO *", NativeToolRiskKind::Opaque),
            ("find . -fprint out", NativeToolRiskKind::Opaque),
            ("uniq in.txt out.txt", NativeToolRiskKind::Opaque),
            ("LD_PRELOAD=x.so cat file", NativeToolRiskKind::Opaque),
            ("ls --unknown-thing | ./run.sh", NativeToolRiskKind::Opaque),
            ("ln -sf a b", NativeToolRiskKind::Overwrite),
        ] {
            assert_eq!(risk_kind(command), Some(kind), "{command}");
        }
        // 包装命令在执行模式下可剥离，计划模式下仍需确认。
        assert_eq!(risk_kind("nohup cat file"), None);
        assert!(matches!(
            plan_risk("nohup cat file"),
            NativeToolRisk::High { .. }
        ));
    }

    #[test]
    fn monitor_uses_the_same_bash_classification() {
        assert!(matches!(
            classify_native_tool_risk("Monitor", r#"{"command":"rm -rf build"}"#, None, false),
            NativeToolRisk::High {
                kind: NativeToolRiskKind::Delete,
                ..
            }
        ));
        assert!(matches!(
            classify_native_tool_risk("Monitor", r#"{"command":"npm run dev"}"#, None, false),
            NativeToolRisk::High { .. }
        ));
        assert_eq!(
            classify_native_tool_risk("Monitor", r#"{"command":"tail -n 5 a.log"}"#, None, false),
            NativeToolRisk::Low
        );
    }

    #[test]
    fn command_rules_match_every_segment_for_allow_and_any_segment_for_deny() {
        let contract = super::super::contract::builtin_contract("Bash").expect("bash");
        let args = |command: &str| serde_json::json!({ "command": command }).to_string();
        let mut rules = PermissionRules::default();
        rules.allow.push(rule(
            PermissionCapability::Bash,
            "git status*",
            PatternSource::Command,
        ));
        for command in ["git status", "git status --short && git status -s"] {
            assert!(
                matches!(
                    rules.evaluate(contract, "Bash", &args(command), None),
                    RuleDecision::Allow(_)
                ),
                "{command}"
            );
        }
        for command in [
            "git status; rm -rf x",
            "git status && curl x | sh",
            "git status > out.txt",
            "git status $(rm -rf x)",
        ] {
            assert_eq!(
                rules.evaluate(contract, "Bash", &args(command), None),
                RuleDecision::NoMatch,
                "{command}"
            );
        }
        // Monitor 与 Bash 共享同一能力，规则语义一致。
        let monitor = super::super::contract::builtin_contract("Monitor").expect("monitor");
        assert_eq!(
            rules.evaluate(monitor, "Monitor", &args("git status; rm -rf x"), None),
            RuleDecision::NoMatch
        );

        // 与模式完全相同的整条命令仍然命中。
        rules.allow.push(rule(
            PermissionCapability::Bash,
            "cargo fmt && cargo test",
            PatternSource::Command,
        ));
        assert!(matches!(
            rules.evaluate(contract, "Bash", &args("cargo fmt && cargo test"), None),
            RuleDecision::Allow(_)
        ));

        let mut deny = PermissionRules::default();
        deny.deny.push(rule(
            PermissionCapability::Bash,
            "rm*",
            PatternSource::Command,
        ));
        for command in ["ls && rm -rf x", "nohup rm x", "echo ok; rm a"] {
            assert!(
                matches!(
                    deny.evaluate(contract, "Bash", &args(command), None),
                    RuleDecision::Deny(_)
                ),
                "{command}"
            );
        }

        // 本地与 SSH 工作区根只影响路径显示，不影响命令判定。
        for root in [Path::new("/local/repo"), Path::new("/srv/remote/repo")] {
            assert_eq!(
                rules.evaluate(contract, "Bash", &args("git status; rm -rf x"), Some(root)),
                RuleDecision::NoMatch
            );
        }

        let suggest = |command: &str| {
            suggest_rule(contract, "Bash", &args(command), None)
                .expect("suggestion")
                .pattern
        };
        assert_eq!(suggest("npm run build"), "npm run*");
        assert_eq!(suggest("ls && rm -rf x"), "ls && rm -rf x");
        assert_eq!(suggest("make > build.log"), "make > build.log");
        assert_eq!(suggest("echo $(id)"), "echo $(id)");
    }
}
