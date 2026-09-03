//! 逐工具契约：把「白名单 + 启发式」升级为每个工具自带的元数据。
//!
//! 调度器据此决定并行、超时、结果预算与审批；权限层据此决定能力归类。
//! 内置工具的契约在 [`super::catalog`] 里声明，MCP 工具按 `annotations`
//! 动态生成。

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// 工具副作用的影响范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectScope {
    None,
    Session,
    Workspace,
    Network,
    System,
}

/// 工具风险等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

/// 权限能力：规则层按能力匹配 allow / deny / ask。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionCapability {
    Read,
    Edit,
    Bash,
    Mcp,
    WebSearch,
    WebFetch,
    Subagent,
    Skill,
    TodoRead,
    TodoWrite,
    AskUser,
    AutomationRead,
    AutomationWrite,
    AgentMessageSend,
    AgentMessageRespond,
    SessionContextRead,
    GoalRead,
    Workflow,
}

impl PermissionCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Bash => "bash",
            Self::Mcp => "mcp",
            Self::WebSearch => "websearch",
            Self::WebFetch => "webfetch",
            Self::Subagent => "subagent",
            Self::Skill => "skill",
            Self::TodoRead => "todo.read",
            Self::TodoWrite => "todo.write",
            Self::AskUser => "ask_user",
            Self::AutomationRead => "automation.read",
            Self::AutomationWrite => "automation.write",
            Self::AgentMessageSend => "agent.message.send",
            Self::AgentMessageRespond => "agent.message.respond",
            Self::SessionContextRead => "session.context.read",
            Self::GoalRead => "goal.read",
            Self::Workflow => "workflow",
        }
    }
}

/// 权限规则可以从哪些字段抽取匹配模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternSource {
    Command,
    Path,
    Input,
    ToolName,
}

/// 工具输出超过预算时的处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStrategy {
    /// 直接截断，保留头尾。
    Truncate,
    /// 完整输出落盘为 artifact，模型只看预览。
    Artifact,
}

/// 预览保留方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewDirection {
    Head,
    Tail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultBudget {
    /// 事件流 / 会话记录里内联保留的上限（字节）。
    pub max_inline_bytes: usize,
    /// 送给模型的上限（字节），超过则按 `strategy` 处理。
    pub max_model_bytes: usize,
    pub strategy: ResultStrategy,
    pub preview: PreviewDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolTimeout {
    /// `0` 表示不限制（例如需要等待用户的工具）。
    pub default_ms: u64,
    pub max_ms: u64,
    /// 是否允许模型在参数里覆盖超时（Bash 的 `timeout`）。
    pub allow_call_override: bool,
}

impl ToolTimeout {
    pub const fn none() -> Self {
        Self {
            default_ms: 0,
            max_ms: 0,
            allow_call_override: false,
        }
    }

    pub const fn fixed(ms: u64) -> Self {
        Self {
            default_ms: ms,
            max_ms: ms,
            allow_call_override: false,
        }
    }

    pub fn is_unbounded(&self) -> bool {
        self.default_ms == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolContract {
    pub name: String,
    /// 一句话能力描述，供设置页与诊断使用。
    pub capability: &'static str,
    pub read_only: bool,
    pub destructive: bool,
    /// 与其他 `concurrent_safe` 工具可在同一轮并行执行。
    pub concurrent_safe: bool,
    pub side_effect_scope: SideEffectScope,
    pub risk_level: RiskLevel,
    /// 执行前需要走审批流程（规则层 / 弹窗）。
    pub needs_approval: bool,
    pub allowed_in_plan_mode: bool,
    /// 需要用户实时交互（提问、审批），不能被工具超时打断。
    pub requires_user_interaction: bool,
    pub permission: PermissionCapability,
    pub pattern_sources: Vec<PatternSource>,
    pub result_budget: ResultBudget,
    pub timeout: ToolTimeout,
}

impl ToolContract {
    /// 是否可以和相邻工具并行：只读、非破坏、无需审批、且未声明 unsafe。
    pub fn can_run_concurrently(&self) -> bool {
        self.concurrent_safe && !self.destructive && !self.needs_approval
    }

    /// MCP 工具契约：默认视为高风险、需要审批；`readOnlyHint` 为真时降为只读。
    pub fn for_mcp(name: &str, read_only_hint: bool, destructive_hint: bool) -> Self {
        Self {
            name: name.to_string(),
            capability: "调用 MCP 服务器提供的工具",
            read_only: read_only_hint && !destructive_hint,
            destructive: destructive_hint,
            concurrent_safe: read_only_hint && !destructive_hint,
            side_effect_scope: if read_only_hint {
                SideEffectScope::Network
            } else {
                SideEffectScope::System
            },
            risk_level: if read_only_hint && !destructive_hint {
                RiskLevel::Medium
            } else {
                RiskLevel::High
            },
            needs_approval: true,
            allowed_in_plan_mode: false,
            requires_user_interaction: false,
            permission: PermissionCapability::Mcp,
            pattern_sources: vec![PatternSource::ToolName, PatternSource::Input],
            result_budget: ResultBudget {
                max_inline_bytes: 1_000_000,
                max_model_bytes: 60_000,
                strategy: ResultStrategy::Artifact,
                preview: PreviewDirection::Head,
            },
            timeout: ToolTimeout {
                default_ms: 120_000,
                max_ms: 300_000,
                allow_call_override: false,
            },
        }
    }

    /// 无契约的未知工具（例如动态注册但尚未声明）按最保守的方式处理。
    pub fn fallback(name: &str) -> Self {
        let mut contract = Self::for_mcp(name, false, false);
        contract.capability = "unknown";
        contract
    }
}

/// 契约注册表：内置工具由 catalog 提供，运行时可追加 MCP 契约。
#[derive(Debug, Clone, Default)]
pub struct ContractRegistry {
    entries: HashMap<String, ToolContract>,
}

impl ContractRegistry {
    pub fn new(contracts: impl IntoIterator<Item = ToolContract>) -> Self {
        let mut registry = Self::default();
        registry.extend(contracts);
        registry
    }

    pub fn extend(&mut self, contracts: impl IntoIterator<Item = ToolContract>) {
        for contract in contracts {
            self.entries.insert(contract.name.clone(), contract);
        }
    }

    pub fn get(&self, name: &str) -> Option<&ToolContract> {
        self.entries.get(name)
    }

    /// 查不到时回退到保守契约，保证调度逻辑总有依据。
    pub fn resolve(&self, name: &str) -> ToolContract {
        self.entries
            .get(name)
            .cloned()
            .unwrap_or_else(|| ToolContract::fallback(name))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

static BUILTIN: LazyLock<ContractRegistry> =
    LazyLock::new(|| ContractRegistry::new(super::catalog::tool_contracts()));

/// 内置工具契约（含仅计划模式可见的提问工具）。
pub fn builtin_contract(name: &str) -> Option<&'static ToolContract> {
    BUILTIN.get(name)
}

/// 内置工具契约，查不到时返回保守回退。
pub fn resolve_builtin_contract(name: &str) -> ToolContract {
    BUILTIN.resolve(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_contract_defaults_to_approval_and_serial_execution() {
        let contract = ToolContract::for_mcp("mcp_demo_tool", false, false);
        assert!(contract.needs_approval);
        assert!(!contract.read_only);
        assert!(!contract.can_run_concurrently());
        assert_eq!(contract.permission, PermissionCapability::Mcp);
        let read_only = ToolContract::for_mcp("mcp_demo_list", true, false);
        assert!(read_only.read_only);
        // 仍需审批，所以不能并行。
        assert!(!read_only.can_run_concurrently());
    }

    #[test]
    fn registry_resolves_unknown_tools_conservatively() {
        let registry = ContractRegistry::default();
        let contract = registry.resolve("Mystery");
        assert_eq!(contract.capability, "unknown");
        assert!(contract.needs_approval);
        assert_eq!(contract.risk_level, RiskLevel::High);
    }

    #[test]
    fn builtin_registry_covers_core_tools() {
        for name in ["Read", "Write", "Edit", "Bash", "Glob", "Grep", "Agent"] {
            assert!(builtin_contract(name).is_some(), "missing contract {name}");
        }
        assert!(builtin_contract("Read").unwrap().can_run_concurrently());
        assert!(!builtin_contract("Write").unwrap().can_run_concurrently());
        assert!(!builtin_contract("Bash").unwrap().can_run_concurrently());
        assert!(
            builtin_contract("Bash")
                .unwrap()
                .timeout
                .allow_call_override
        );
        assert!(
            builtin_contract("AskQuestion")
                .unwrap()
                .requires_user_interaction
        );
        assert!(builtin_contract("AskQuestion")
            .unwrap()
            .timeout
            .is_unbounded());
    }

    #[test]
    fn capability_names_are_stable() {
        assert_eq!(PermissionCapability::TodoWrite.as_str(), "todo.write");
        assert_eq!(
            PermissionCapability::SessionContextRead.as_str(),
            "session.context.read"
        );
    }
}
