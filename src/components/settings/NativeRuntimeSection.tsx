import {
  Bot,
  Check,
  Download,
  Gauge,
  Loader2,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Terminal,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { installLspServer, listLspServers, updateNativeSettings } from "@/lib/backend";
import { isNativePermissionMode, NATIVE_PERMISSION_MODES } from "@/lib/types";
import type { LspServerStatus, NativeSettings } from "@/lib/types";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";
import { SettingCard, SettingRow } from "./SettingCard";

const SUBAGENT_POLICIES = ["conservative", "balanced", "aggressive"] as const;
type SubagentPolicy = (typeof SUBAGENT_POLICIES)[number];

function isSubagentPolicy(value: unknown): value is SubagentPolicy {
  return value === "conservative" || value === "balanced" || value === "aggressive";
}

function TokenKInput({
  id,
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  id: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
}) {
  return (
    <div className="relative w-36">
      <Input
        id={id}
        className="h-8 pr-8 text-xs font-mono text-right"
        type="number"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-xs text-muted-foreground font-mono">
        K
      </span>
    </div>
  );
}

export function NativeRuntimeSection() {
  const { t } = useTranslation(["settings", "common", "sessions"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [draft, setDraft] = useState(native);
  const [lspServers, setLspServers] = useState<LspServerStatus[]>([]);
  const [lspLoading, setLspLoading] = useState(true);
  const [lspInstalling, setLspInstalling] = useState<Set<string>>(() => new Set());
  const [lspErrors, setLspErrors] = useState<Record<string, string>>({});
  const [lspFeedback, setLspFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);

  const initializedRef = useRef(false);
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const lspInstallingRef = useRef(new Set<string>());

  const refreshLspServers = useCallback(async (silent = false) => {
    if (!silent) setLspLoading(true);
    try {
      setLspServers(await listLspServers());
    } catch (error) {
      setLspFeedback({
        variant: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      if (!silent) setLspLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshLspServers();
  }, [refreshLspServers]);

  const handleInstallLsp = useCallback(
    async (language: string) => {
      if (lspInstallingRef.current.has(language)) return;
      lspInstallingRef.current.add(language);
      setLspInstalling(new Set(lspInstallingRef.current));
      setLspErrors((prev) => {
        if (!(language in prev)) return prev;
        const next = { ...prev };
        delete next[language];
        return next;
      });
      try {
        await installLspServer(language);
        await refreshLspServers(true);
      } catch (error) {
        setLspErrors((prev) => ({
          ...prev,
          [language]: error instanceof Error ? error.message : String(error),
        }));
      } finally {
        lspInstallingRef.current.delete(language);
        setLspInstalling(new Set(lspInstallingRef.current));
      }
    },
    [refreshLspServers],
  );

  const pendingLspCount = lspServers.filter(
    (server) => !server.installed_command && server.installable && !lspInstalling.has(server.id),
  ).length;

  const handleInstallMissing = useCallback(() => {
    for (const server of lspServers) {
      if (!server.installed_command && server.installable) {
        void handleInstallLsp(server.id);
      }
    }
  }, [handleInstallLsp, lspServers]);

  // 仅首次加载时用 native 初始化 draft；自动保存回写不再重置草稿，避免丢失正在编辑的输入
  useEffect(() => {
    if (native && !initializedRef.current) {
      initializedRef.current = true;
      setDraft(native);
    }
  }, [native]);

  // 静默自动保存：失败不打扰用户
  const persist = useCallback(
    async (value: NativeSettings) => {
      try {
        const updated = await updateNativeSettings(value);
        setNative(updated);
      } catch {
        // 静默失败，保留草稿，下次变更自动重试
      }
    },
    [setNative],
  );

  // 防抖自动保存：停止变更 600ms 后写入
  useEffect(() => {
    if (!initializedRef.current) return;
    const timer = window.setTimeout(() => {
      if (draftRef.current) void persist(draftRef.current);
    }, 600);
    return () => window.clearTimeout(timer);
  }, [draft, persist]);

  // 卸载时 flush 未保存的变更
  useEffect(() => {
    return () => {
      if (initializedRef.current && draftRef.current) {
        void persist(draftRef.current);
      }
    };
  }, [persist]);

  if (!native || !draft) return null;

  const policyLabel = (value: unknown) => {
    if (value === "conservative") return t("settings:runtime.policyConservative");
    if (value === "aggressive") return t("settings:runtime.policyAggressive");
    return t("settings:runtime.policyBalanced");
  };

  return (
    <div className="space-y-6">
      {/* 顶部提示区 */}
      <div className="flex items-center justify-between">
        <p className="text-xs text-muted-foreground">{t("settings:runtime.hint")}</p>
      </div>

      {/* 1. 会话与权限限制 */}
      <SettingCard
        icon={ShieldCheck}
        title={t("settings:sections.permissions")}
        description="控制会话最大轮次、子代理轮次上限及交互权限策略。"
        divided
      >
        <SettingRow
          title={t("settings:runtime.maxTurns")}
          description={t("settings:runtime.maxTurnsHint")}
        >
          <Input
            id="native-max-turns"
            className="h-8 w-28 text-xs font-mono text-right"
            type="number"
            min={0}
            max={500}
            step={1}
            value={draft.max_turns}
            onChange={(e) => setDraft({ ...draft, max_turns: Number(e.target.value) })}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.maxSubagentTurns")}
          description={t("settings:runtime.maxSubagentTurnsHint")}
        >
          <Input
            id="native-max-subagent-turns"
            className="h-8 w-28 text-xs font-mono text-right"
            type="number"
            min={0}
            max={500}
            step={1}
            value={draft.max_subagent_turns}
            onChange={(e) => setDraft({ ...draft, max_subagent_turns: Number(e.target.value) })}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.permissionMode")}
          description={t("settings:runtime.permissionModeHint")}
        >
          <Select
            value={draft.permission_mode}
            onValueChange={(val) => {
              if (isNativePermissionMode(val)) {
                setDraft({ ...draft, permission_mode: val });
              }
            }}
          >
            <SelectTrigger className="h-8 w-44 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {NATIVE_PERMISSION_MODES.map((mode) => (
                <SelectItem key={mode} value={mode} className="text-xs">
                  {t(`sessions:permission.${mode}.title`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.permissionTimeout")}
          description={t("settings:runtime.permissionTimeoutHint")}
        >
          <div className="relative w-28">
            <Input
              id="native-permission-timeout"
              className="h-8 pr-7 text-xs font-mono text-right"
              type="number"
              min={0}
              max={86400}
              step={1}
              value={draft.permission_timeout_secs}
              onChange={(e) =>
                setDraft({ ...draft, permission_timeout_secs: Number(e.target.value) })
              }
            />
            <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
              s
            </span>
          </div>
        </SettingRow>
      </SettingCard>

      {/* 2. 工具运行时与工件快照 */}
      <SettingCard
        icon={Terminal}
        title={t("settings:runtime.toolRuntime")}
        description={t("settings:runtime.toolRuntimeHint")}
        divided
      >
        <SettingRow
          title={t("settings:runtime.bashDefaultTimeout")}
          description="Bash 命令执行的默认等待超时秒数。"
        >
          <div className="relative w-28">
            <Input
              id="native-bash-timeout"
              className="h-8 pr-7 text-xs font-mono text-right"
              type="number"
              min={1}
              max={600}
              step={1}
              value={draft.bash_default_timeout_secs}
              onChange={(e) =>
                setDraft({ ...draft, bash_default_timeout_secs: Number(e.target.value) })
              }
            />
            <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
              s
            </span>
          </div>
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.autoCheckpoint")}
          description={t("settings:runtime.checkpointsHint")}
        >
          <Switch
            id="native-auto-checkpoint"
            checked={draft.auto_checkpoint_after_tool_call}
            onCheckedChange={(checked) =>
              setDraft({ ...draft, auto_checkpoint_after_tool_call: checked })
            }
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.checkpointRetention")}
          description="历史检查点自动清理与保留天数。"
        >
          <div className="relative w-28">
            <Input
              id="native-checkpoint-retention"
              className="h-8 pr-8 text-xs font-mono text-right"
              type="number"
              min={0}
              max={365}
              step={1}
              value={draft.checkpoint_retention_days}
              onChange={(e) =>
                setDraft({ ...draft, checkpoint_retention_days: Number(e.target.value) })
              }
            />
            <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
              天
            </span>
          </div>
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.artifactRetention")}
          description="生成工件的历史记录保留天数。"
        >
          <div className="relative w-28">
            <Input
              id="native-artifact-retention"
              className="h-8 pr-8 text-xs font-mono text-right"
              type="number"
              min={0}
              max={365}
              step={1}
              value={draft.artifact_retention_days}
              onChange={(e) =>
                setDraft({ ...draft, artifact_retention_days: Number(e.target.value) })
              }
            />
            <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
              天
            </span>
          </div>
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.shellSnapshot")}
          description="记录终端执行状态快照，用于任务回溯。"
        >
          <Switch
            id="native-shell-snapshot"
            checked={draft.shell_snapshot_enabled}
            onCheckedChange={(checked) => setDraft({ ...draft, shell_snapshot_enabled: checked })}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.rgSidecar")}
          description="启用 Ripgrep 侧车加速文件树检索。"
        >
          <Switch
            id="native-rg-sidecar"
            checked={draft.rg_sidecar_enabled}
            onCheckedChange={(checked) => setDraft({ ...draft, rg_sidecar_enabled: checked })}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.lspEnabled")}
          description={t("settings:runtime.lspEnabledHint")}
        >
          <Switch
            id="native-lsp-enabled"
            checked={draft.lsp_enabled}
            onCheckedChange={(checked) => setDraft({ ...draft, lsp_enabled: checked })}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.bashSandbox")}
          description={t("settings:runtime.bashSandboxHint")}
        >
          <Switch
            id="native-bash-sandbox"
            checked={draft.bash_sandbox_enabled}
            onCheckedChange={(checked) => setDraft({ ...draft, bash_sandbox_enabled: checked })}
          />
        </SettingRow>
      </SettingCard>

      {/* 3. Language server 安装 */}
      <SettingCard
        icon={Download}
        title={t("settings:runtime.lspInstallTitle")}
        description={t("settings:runtime.lspInstallHint")}
        headerAction={
          !lspLoading && pendingLspCount > 0 ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 gap-1.5 text-xs"
              onClick={handleInstallMissing}
            >
              {lspInstalling.size > 0 ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <Download className="size-3" />
              )}
              {t("settings:runtime.lspInstallMissing", { count: pendingLspCount })}
            </Button>
          ) : null
        }
        divided
      >
        {lspFeedback ? (
          <div className="px-5 pt-3">
            <SettingFeedbackCallout
              variant={lspFeedback.variant}
              message={lspFeedback.message}
              onClose={() => setLspFeedback(null)}
            />
          </div>
        ) : null}
        {lspLoading ? (
          <div className="flex items-center gap-2 px-5 py-4 text-xs text-muted-foreground">
            <Loader2 className="size-3 animate-spin" />
            {t("settings:runtime.lspLoading")}
          </div>
        ) : (
          lspServers.map((server) => {
            const installing = lspInstalling.has(server.id);
            const error = lspErrors[server.id];
            return (
              <SettingRow
                key={server.id}
                title={
                  <span className="flex items-center gap-2">
                    {server.label}
                    {server.installed_command ? (
                      <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] text-emerald-600 dark:text-emerald-400">
                        <Check className="size-3" />
                        {t("settings:runtime.lspInstalled")}
                      </span>
                    ) : null}
                  </span>
                }
                description={
                  error ? (
                    <span className="text-destructive">{error}</span>
                  ) : server.installed_command ? (
                    `${t("settings:runtime.lspCommand")}: ${server.installed_command}`
                  ) : (
                    `${t("settings:runtime.lspInstallCommand")}: ${server.install_command ?? server.commands.join(" / ")}`
                  )
                }
              >
                {server.installed_command ? (
                  <span className="text-[11px] text-muted-foreground">
                    {t("settings:runtime.lspReady")}
                  </span>
                ) : server.installable ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-7 text-xs"
                    disabled={installing}
                    onClick={() => void handleInstallLsp(server.id)}
                  >
                    {installing ? (
                      <Loader2 className="mr-1.5 size-3 animate-spin" />
                    ) : (
                      <Download className="mr-1.5 size-3" />
                    )}
                    {t("settings:runtime.lspInstall")}
                  </Button>
                ) : (
                  <span className="text-[11px] text-muted-foreground">
                    {t("settings:runtime.lspManual")}
                  </span>
                )}
              </SettingRow>
            );
          })
        )}
      </SettingCard>

      {/* 4. 上下文窗口与 Token 预算 */}
      <SettingCard
        icon={Gauge}
        title={t("settings:runtime.contextWindow")}
        description={t("settings:runtime.contextWindowHint")}
        divided
      >
        <SettingRow
          title={t("settings:runtime.useCustomContextWindow")}
          description="是否覆盖模型默认的上下文窗口大小。"
        >
          <div className="flex items-center gap-3">
            {draft.use_custom_context_window ? (
              <TokenKInput
                id="native-context-window-k"
                min={8}
                max={1000}
                value={draft.context_window_tokens / 1000}
                onChange={(value) => setDraft({ ...draft, context_window_tokens: value * 1000 })}
              />
            ) : null}
            <Switch
              id="native-use-custom-context-window"
              checked={draft.use_custom_context_window}
              onCheckedChange={(checked) =>
                setDraft({ ...draft, use_custom_context_window: checked })
              }
            />
          </div>
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.compactThreshold")}
          description="上下文占用百分比达到该阈值时自动执行会话压缩。"
        >
          <div className="relative w-28">
            <Input
              id="native-compact-threshold"
              className="h-8 pr-7 text-xs font-mono text-right"
              type="number"
              min={30}
              max={99}
              step={1}
              value={draft.auto_compact_threshold_percent}
              onChange={(e) =>
                setDraft({ ...draft, auto_compact_threshold_percent: Number(e.target.value) })
              }
            />
            <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
              %
            </span>
          </div>
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.microcompact")}
          description="启用轻量微压缩，精简旧工具调用历史以节省 Token。"
        >
          <Switch
            id="native-microcompact"
            checked={draft.microcompact_enabled}
            onCheckedChange={(checked) => setDraft({ ...draft, microcompact_enabled: checked })}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.rolloutBudget")}
          description={t("settings:runtime.rolloutBudgetHint")}
        >
          <TokenKInput
            id="native-rollout-token-budget-k"
            min={0}
            max={100000}
            value={draft.rollout_token_budget / 1000}
            onChange={(value) => setDraft({ ...draft, rollout_token_budget: value * 1000 })}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.maxToolOutput")}
          description={t("settings:runtime.maxToolOutputHint")}
        >
          <TokenKInput
            id="native-max-tool-output-k"
            min={0.256}
            max={65.536}
            step={0.001}
            value={draft.max_tool_output_tokens / 1000}
            onChange={(value) => setDraft({ ...draft, max_tool_output_tokens: value * 1000 })}
          />
        </SettingRow>
      </SettingCard>

      {/* 5. 子智能体策略 */}
      <SettingCard
        icon={Bot}
        title={t("settings:sections.subagents")}
        description="配置主代理调度子智能体时的并发限制与资源分配策略。"
        divided
      >
        <SettingRow
          title={t("settings:runtime.maxConcurrent")}
          description={t("settings:runtime.maxConcurrentHint")}
        >
          <Input
            id="native-max-concurrent-subagents"
            className="h-8 w-28 text-xs font-mono text-right"
            type="number"
            min={1}
            max={16}
            step={1}
            value={draft.max_concurrent_subagents}
            onChange={(e) =>
              setDraft({ ...draft, max_concurrent_subagents: Number(e.target.value) })
            }
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.policy")}
          description={t("settings:runtime.policyHint")}
        >
          <Select
            value={draft.subagent_policy}
            onValueChange={(val) => {
              if (isSubagentPolicy(val)) {
                setDraft({ ...draft, subagent_policy: val });
              }
            }}
          >
            <SelectTrigger id="native-subagent-policy" className="h-8 w-36 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {SUBAGENT_POLICIES.map((policy) => (
                <SelectItem key={policy} value={policy} className="text-xs">
                  {policyLabel(policy)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.budgetShare")}
          description={t("settings:runtime.budgetShareHint")}
        >
          <div className="relative w-28">
            <Input
              id="native-subagent-budget-share"
              className="h-8 pr-7 text-xs font-mono text-right"
              type="number"
              min={5}
              max={100}
              step={1}
              value={draft.subagent_budget_share_percent}
              onChange={(e) =>
                setDraft({ ...draft, subagent_budget_share_percent: Number(e.target.value) })
              }
            />
            <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
              %
            </span>
          </div>
        </SettingRow>
      </SettingCard>

      {/* 6. 模型请求重试 */}
      <SettingCard
        icon={RefreshCw}
        title={t("settings:runtime.modelRetry")}
        description={t("settings:runtime.modelRetryHint")}
        divided
      >
        <SettingRow
          title={t("settings:runtime.modelRetryMax")}
          description="网络抖动或超限时的重试次数上限。"
        >
          <Input
            id="native-retry-max"
            className="h-8 w-28 text-xs font-mono text-right"
            type="number"
            min={0}
            max={20}
            step={1}
            value={draft.model_retry_max_retries}
            onChange={(e) =>
              setDraft({ ...draft, model_retry_max_retries: Number(e.target.value) })
            }
          />
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.modelRetryBaseDelay")}
          description="首次重试的基础等待间隔。"
        >
          <div className="relative w-28">
            <Input
              id="native-retry-base"
              className="h-8 pr-8 text-xs font-mono text-right"
              type="number"
              min={100}
              max={60000}
              step={100}
              value={draft.model_retry_base_delay_ms}
              onChange={(e) =>
                setDraft({ ...draft, model_retry_base_delay_ms: Number(e.target.value) })
              }
            />
            <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
              ms
            </span>
          </div>
        </SettingRow>

        <SettingRow
          title={t("settings:runtime.modelRetryBackoff")}
          description="重试延迟指数递增倍数因子。"
        >
          <Input
            id="native-retry-factor"
            className="h-8 w-28 text-xs font-mono text-right"
            type="number"
            min={1}
            max={4}
            step={0.1}
            value={draft.model_retry_backoff_factor}
            onChange={(e) =>
              setDraft({ ...draft, model_retry_backoff_factor: Number(e.target.value) })
            }
          />
        </SettingRow>
      </SettingCard>

      {/* 7. 全局系统提示词 */}
      <SettingCard
        icon={Sparkles}
        title={t("settings:runtime.globalPrompt")}
        description={t("settings:runtime.globalPromptHint")}
      >
        <Textarea
          id="native-global-prompt"
          rows={5}
          className="resize-y font-mono text-xs leading-relaxed"
          placeholder="可在此输入全局注入的系统角色与环境提示词约束…"
          value={draft.global_prompt_template}
          onChange={(event) => setDraft({ ...draft, global_prompt_template: event.target.value })}
        />
      </SettingCard>
    </div>
  );
}
