import { useCallback, useEffect, useState } from "react";
import { Plus, ShieldCheck, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  addNativePermissionRule,
  deleteNativePermissionRule,
  getNativePermissionRules,
} from "@/lib/backend";
import type {
  NativePermissionRulesView,
  PermissionCapability,
  PermissionPatternSource,
  PermissionRule,
  PermissionRuleEffect,
  PermissionRuleScope,
} from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

const CAPABILITIES: PermissionCapability[] = [
  "bash",
  "edit",
  "read",
  "mcp",
  "web_fetch",
  "web_search",
  "subagent",
  "skill",
];
const SOURCES: PermissionPatternSource[] = ["command", "path", "tool_name", "input"];
const EFFECTS: PermissionRuleEffect[] = ["allow", "deny", "ask"];

function defaultSourceFor(capability: PermissionCapability): PermissionPatternSource {
  if (capability === "bash") return "command";
  if (capability === "edit" || capability === "read") return "path";
  return "tool_name";
}

export function PermissionRulesSection() {
  const { t } = useTranslation(["settings", "common"]);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const [view, setView] = useState<NativePermissionRulesView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [saving, setSaving] = useState(false);

  // Form State
  const [effect, setEffect] = useState<PermissionRuleEffect>("allow");
  const [capability, setCapability] = useState<PermissionCapability>("bash");
  const [source, setSource] = useState<PermissionPatternSource>("command");
  const [scope, setScope] = useState<PermissionRuleScope>("global");
  const [pattern, setPattern] = useState("");
  const [note, setNote] = useState("");

  const reload = useCallback(() => {
    getNativePermissionRules(workspaceId)
      .then((next) => {
        setView(next);
        setError(null);
      })
      .catch((reason: unknown) => setError(String(reason)));
  }, [workspaceId]);

  useEffect(() => {
    reload();
  }, [reload]);

  const openCreate = () => {
    setEffect("allow");
    setCapability("bash");
    setSource("command");
    setScope("global");
    setPattern("");
    setNote("");
    setError(null);
    setMessage(null);
    setDialogOpen(true);
  };

  const submit = async () => {
    const trimmed = pattern.trim();
    if (!trimmed) return;
    setSaving(true);
    setError(null);
    try {
      const rule: PermissionRule = {
        id: "",
        capability,
        pattern: trimmed,
        source,
        scope,
        note: note.trim(),
      };
      await addNativePermissionRule(effect, rule, workspaceId);
      setPattern("");
      setNote("");
      setDialogOpen(false);
      setMessage(t("common:saved", { defaultValue: "规则已添加" }));
      reload();
    } catch (reason: unknown) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  const remove = (id: string) => {
    deleteNativePermissionRule(id, workspaceId)
      .then(() => {
        setMessage(t("common:deleted", { defaultValue: "已删除" }));
        reload();
      })
      .catch((reason: unknown) => setError(String(reason)));
  };

  const renderRuleList = (rules: PermissionRule[], ruleEffect: PermissionRuleEffect) => {
    if (rules.length === 0) return null;

    const effectConfig = {
      allow: {
        badgeClass:
          "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
        label: t("settings:permissions.effect.allow"),
      },
      deny: {
        badgeClass: "border-destructive/30 bg-destructive/10 text-destructive",
        label: t("settings:permissions.effect.deny"),
      },
      ask: {
        badgeClass: "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400",
        label: t("settings:permissions.effect.ask"),
      },
    }[ruleEffect];

    return (
      <div className="space-y-1.5">
        {rules.map((rule) => (
          <div
            key={rule.id}
            className="group flex items-center justify-between gap-3 rounded-xl border border-border/70 bg-card p-3 shadow-2xs transition-all hover:border-border"
          >
            <div className="flex flex-wrap items-center gap-2 min-w-0 flex-1">
              <span
                className={`inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider ${effectConfig.badgeClass}`}
              >
                {effectConfig.label}
              </span>
              <span className="rounded-md border border-border/60 bg-muted/40 px-2 py-0.5 text-[10px] font-medium text-muted-foreground">
                {t(`settings:permissions.capability.${rule.capability}`, {
                  defaultValue: rule.capability,
                })}
              </span>
              <code className="min-w-0 truncate rounded-md bg-muted/60 px-2 py-0.5 font-mono text-xs text-foreground font-semibold">
                {rule.pattern}
              </code>
              <span className="text-[10px] text-muted-foreground font-mono">
                ({t(`settings:permissions.source.${rule.source}`)})
              </span>
              {rule.note ? (
                <span className="hidden truncate text-[11px] text-muted-foreground sm:inline max-w-xs">
                  · {rule.note}
                </span>
              ) : null}
            </div>
            <Button
              variant="ghost"
              size="icon-xs"
              className="text-muted-foreground opacity-60 hover:text-destructive hover:opacity-100"
              onClick={() => remove(rule.id)}
              title={t("common:delete")}
            >
              <Trash2 className="size-3.5" />
            </Button>
          </div>
        ))}
      </div>
    );
  };

  const renderScopeSection = (
    title: string,
    rules: NativePermissionRulesView["global"] | null,
    scopePath?: string,
  ) => {
    if (!rules) return null;
    const totalCount = rules.deny.length + rules.allow.length + rules.ask.length;

    return (
      <div className="space-y-3">
        <div className="flex items-center justify-between border-b border-border/50 pb-1.5">
          <span className="text-xs font-semibold text-foreground tracking-tight">{title}</span>
          <span className="text-[10px] font-mono text-muted-foreground">{totalCount} 条规则</span>
        </div>
        {totalCount === 0 ? (
          <p className="py-2 text-xs text-muted-foreground">{t("settings:permissions.empty")}</p>
        ) : (
          <div className="space-y-2">
            {renderRuleList(rules.deny, "deny")}
            {renderRuleList(rules.allow, "allow")}
            {renderRuleList(rules.ask, "ask")}
          </div>
        )}
        {scopePath ? (
          <p className="text-[10px] font-mono text-muted-foreground truncate">{scopePath}</p>
        ) : null}
      </div>
    );
  };

  const totalRuleCount = view
    ? view.global.deny.length +
      view.global.allow.length +
      view.global.ask.length +
      (view.workspace
        ? view.workspace.deny.length + view.workspace.allow.length + view.workspace.ask.length
        : 0)
    : 0;

  return (
    <div className="space-y-6">
      {message ? (
        <SettingFeedbackCallout
          variant="success"
          message={message}
          onClose={() => setMessage(null)}
        />
      ) : null}
      {error ? (
        <SettingFeedbackCallout variant="error" message={error} onClose={() => setError(null)} />
      ) : null}

      <SettingCard
        icon={ShieldCheck}
        title={t("settings:permissions.title", { defaultValue: "权限规则" })}
        description={t("settings:permissions.hint")}
        badge={`${totalRuleCount} 条规则`}
        headerAction={
          <Button size="sm" onClick={openCreate} className="h-7 gap-1 text-xs">
            <Plus className="size-3.5" />
            {t("settings:permissions.addTitle")}
          </Button>
        }
      >
        <div className="space-y-6">
          {view
            ? renderScopeSection(
                "全局规则 (Global)",
                view.global,
                t("settings:permissions.globalPath"),
              )
            : null}

          {view && view.workspace
            ? renderScopeSection(
                "工作区规则 (Workspace)",
                view.workspace,
                t("settings:permissions.workspacePath", { path: view.workspace_root ?? "" }),
              )
            : null}
        </div>
      </SettingCard>

      {/* 添加规则 Dialog */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-md rounded-2xl p-0 overflow-hidden">
          <DialogHeader className="border-b border-border/50 px-6 py-4">
            <DialogTitle className="text-base font-semibold tracking-tight">
              {t("settings:permissions.addTitle")}
            </DialogTitle>
            <DialogDescription className="text-xs text-muted-foreground">
              {t("settings:permissions.addHint")}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-3.5 px-6 py-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("settings:permissions.effectLabel")}
                </label>
                <Select
                  value={effect}
                  onValueChange={(val) => setEffect(val as PermissionRuleEffect)}
                >
                  <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {EFFECTS.map((item) => (
                      <SelectItem key={item} value={item} className="text-xs">
                        {t(`settings:permissions.effect.${item}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("settings:permissions.capabilityLabel")}
                </label>
                <Select
                  value={capability}
                  onValueChange={(val) => {
                    const next = val as PermissionCapability;
                    setCapability(next);
                    setSource(defaultSourceFor(next));
                  }}
                >
                  <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CAPABILITIES.map((item) => (
                      <SelectItem key={item} value={item} className="text-xs">
                        {t(`settings:permissions.capability.${item}`, { defaultValue: item })}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("settings:permissions.sourceLabel")}
                </label>
                <Select
                  value={source}
                  onValueChange={(val) => setSource(val as PermissionPatternSource)}
                >
                  <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {SOURCES.map((item) => (
                      <SelectItem key={item} value={item} className="text-xs">
                        {t(`settings:permissions.source.${item}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("settings:permissions.scopeLabel")}
                </label>
                <Select value={scope} onValueChange={(val) => setScope(val as PermissionRuleScope)}>
                  <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="global" className="text-xs">
                      {t("settings:permissions.scopeGlobal")}
                    </SelectItem>
                    <SelectItem value="workspace" className="text-xs" disabled={!workspaceId}>
                      {t("settings:permissions.scopeWorkspace")}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div>
              <label className="text-xs font-medium text-muted-foreground">
                {t("settings:permissions.patternLabel")}
              </label>
              <Input
                className="mt-1 h-8 font-mono text-xs"
                placeholder={capability === "bash" ? "git *" : "*.env*"}
                value={pattern}
                onChange={(e) => setPattern(e.target.value)}
              />
            </div>

            <div>
              <label className="text-xs font-medium text-muted-foreground">
                {t("settings:permissions.noteLabel")}
              </label>
              <Input
                className="mt-1 h-8 text-xs"
                placeholder="说明备注（可选）"
                value={note}
                onChange={(e) => setNote(e.target.value)}
              />
            </div>
          </div>

          <DialogFooter className="border-t border-border/50 px-6 py-3 bg-muted/10">
            <Button
              variant="outline"
              size="sm"
              className="h-8 text-xs"
              onClick={() => setDialogOpen(false)}
            >
              {t("common:cancel")}
            </Button>
            <Button
              size="sm"
              className="h-8 text-xs"
              disabled={saving || !pattern.trim()}
              onClick={() => void submit()}
            >
              {t("settings:permissions.add")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
