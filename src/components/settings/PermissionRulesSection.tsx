import { useCallback, useEffect, useState } from "react";
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
import { Input } from "@/components/ui/input";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard } from "./SettingCard";

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

  const submit = () => {
    const trimmed = pattern.trim();
    if (!trimmed) return;
    const rule: PermissionRule = {
      id: "",
      capability,
      pattern: trimmed,
      source,
      scope,
      note: note.trim(),
    };
    addNativePermissionRule(effect, rule, workspaceId)
      .then(() => {
        setPattern("");
        setNote("");
        reload();
      })
      .catch((reason: unknown) => setError(String(reason)));
  };

  const remove = (id: string) => {
    deleteNativePermissionRule(id, workspaceId)
      .then(() => reload())
      .catch((reason: unknown) => setError(String(reason)));
  };

  const renderList = (title: string, rules: PermissionRule[], ruleEffect: PermissionRuleEffect) => (
    <div className="space-y-1">
      <p className="text-xs font-medium text-muted-foreground">
        {title} · {t(`settings:permissions.effect.${ruleEffect}`)} ({rules.length})
      </p>
      {rules.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t("settings:permissions.empty")}</p>
      ) : (
        <ul className="divide-y rounded-md border">
          {rules.map((rule) => (
            <li key={rule.id} className="flex items-center gap-3 px-3 py-2 text-sm">
              <span className="w-24 shrink-0 text-xs text-muted-foreground">
                {t(`settings:permissions.capability.${rule.capability}`, {
                  defaultValue: rule.capability,
                })}
              </span>
              <code className="min-w-0 flex-1 truncate rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
                {rule.pattern}
              </code>
              <span className="shrink-0 text-xs text-muted-foreground">
                {t(`settings:permissions.source.${rule.source}`)}
              </span>
              {rule.note ? (
                <span className="hidden max-w-40 truncate text-xs text-muted-foreground sm:inline">
                  {rule.note}
                </span>
              ) : null}
              <Button variant="ghost" size="sm" onClick={() => remove(rule.id)}>
                {t("common:delete")}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );

  const renderScope = (
    label: string,
    rules: NativePermissionRulesView["global"] | null,
    scopeId: PermissionRuleScope,
  ) => {
    if (!rules) return null;
    return (
      <div className="space-y-3">
        {renderList(label, rules.deny, "deny")}
        {renderList(label, rules.allow, "allow")}
        {renderList(label, rules.ask, "ask")}
        <p className="text-xs text-muted-foreground">
          {scopeId === "global"
            ? t("settings:permissions.globalPath")
            : t("settings:permissions.workspacePath", { path: view?.workspace_root ?? "" })}
        </p>
      </div>
    );
  };

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t("settings:permissions.hint")}</p>
      {error ? <p className="text-sm text-destructive">{error}</p> : null}
      <SettingCard
        title={t("settings:permissions.addTitle")}
        description={t("settings:permissions.addHint")}
      >
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="block text-sm">
            <span>{t("settings:permissions.effectLabel")}</span>
            <select
              className="mt-1 h-8 w-full rounded-md border bg-background px-2 text-sm"
              value={effect}
              onChange={(event) => setEffect(event.target.value as PermissionRuleEffect)}
            >
              {EFFECTS.map((item) => (
                <option key={item} value={item}>
                  {t(`settings:permissions.effect.${item}`)}
                </option>
              ))}
            </select>
          </label>
          <label className="block text-sm">
            <span>{t("settings:permissions.capabilityLabel")}</span>
            <select
              className="mt-1 h-8 w-full rounded-md border bg-background px-2 text-sm"
              value={capability}
              onChange={(event) => {
                const next = event.target.value as PermissionCapability;
                setCapability(next);
                setSource(defaultSourceFor(next));
              }}
            >
              {CAPABILITIES.map((item) => (
                <option key={item} value={item}>
                  {t(`settings:permissions.capability.${item}`)}
                </option>
              ))}
            </select>
          </label>
          <label className="block text-sm">
            <span>{t("settings:permissions.sourceLabel")}</span>
            <select
              className="mt-1 h-8 w-full rounded-md border bg-background px-2 text-sm"
              value={source}
              onChange={(event) => setSource(event.target.value as PermissionPatternSource)}
            >
              {SOURCES.map((item) => (
                <option key={item} value={item}>
                  {t(`settings:permissions.source.${item}`)}
                </option>
              ))}
            </select>
          </label>
          <label className="block text-sm">
            <span>{t("settings:permissions.scopeLabel")}</span>
            <select
              className="mt-1 h-8 w-full rounded-md border bg-background px-2 text-sm"
              value={scope}
              onChange={(event) => setScope(event.target.value as PermissionRuleScope)}
            >
              <option value="global">{t("settings:permissions.scope.global")}</option>
              <option value="workspace" disabled={!view?.workspace_root}>
                {t("settings:permissions.scope.workspace")}
              </option>
            </select>
          </label>
          <label className="block text-sm sm:col-span-2">
            <span>{t("settings:permissions.patternLabel")}</span>
            <Input
              className="mt-1 font-mono"
              value={pattern}
              placeholder={t(`settings:permissions.patternPlaceholder.${source}`)}
              onChange={(event) => setPattern(event.target.value)}
            />
          </label>
          <label className="block text-sm sm:col-span-2">
            <span>{t("settings:permissions.noteLabel")}</span>
            <Input
              className="mt-1"
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
          </label>
        </div>
        <Button className="mt-3" disabled={!pattern.trim()} onClick={submit}>
          {t("settings:permissions.add")}
        </Button>
      </SettingCard>
      <SettingCard
        title={t("settings:permissions.globalTitle")}
        description={t("settings:permissions.orderHint")}
      >
        {renderScope(t("settings:permissions.scope.global"), view?.global ?? null, "global")}
      </SettingCard>
      {view?.workspace ? (
        <SettingCard
          title={t("settings:permissions.workspaceTitle")}
          description={view.workspace_root ?? ""}
        >
          {renderScope(t("settings:permissions.scope.workspace"), view.workspace, "workspace")}
        </SettingCard>
      ) : null}
    </div>
  );
}
