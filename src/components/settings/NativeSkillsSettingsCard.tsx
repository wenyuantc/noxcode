import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { Copy, FolderOpen, Plus, RefreshCw, Trash2, Wrench } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  copyNativeSkillToGlobal,
  deleteNativeSkill,
  listNativeSkills,
  openNativeSkillPath,
  openNativeSkillsDir,
  setNativeSkillEnabled,
} from "@/lib/backend";
import {
  SKILL_DIR_GLOBAL,
  SKILL_DIR_PLUGIN,
  normalizeSkillPath,
  parseWorkspaceSkillDirFilter,
  shortWorkspaceDirLabel,
  skillBelongsToDir,
  workspaceSkillDirFilter,
  workspaceSkillPath,
} from "@/lib/skillWorkspaceDir";
import type { NativeSkill, NativeSkillSource, NativeSkillsView, Workspace } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";
import { SkillCreateDialog } from "./SkillCreateDialog";
import { SkillImportDialog } from "./SkillImportDialog";

type SourceFilter = "all" | NativeSkillSource;
type StatusFilter = "all" | "enabled" | "disabled";

function isManagedSource(source: NativeSkillSource): boolean {
  return source === "global" || source === "workspace_noxcode";
}

function normalizePath(path: string): string {
  return normalizeSkillPath(path);
}

export function NativeSkillsSettingsCard() {
  const { t } = useTranslation("settings");
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const [view, setView] = useState<NativeSkillsView | null>(null);
  const [query, setQuery] = useState("");
  const [dirFilter, setDirFilter] = useState(SKILL_DIR_GLOBAL);
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [busy, setBusy] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [feedback, setFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);
  const loadGeneration = useRef(0);

  const disabled = useMemo(() => new Set((view?.disabled_paths ?? []).map(normalizePath)), [view]);

  const selectedWorkspaceId = parseWorkspaceSkillDirFilter(dirFilter);
  const mutationWorkspaceId = selectedWorkspaceId ?? workspaceId;
  const sortedWorkspaces = useMemo(
    () =>
      [...workspaces].sort((left, right) =>
        left.name.localeCompare(right.name, undefined, { numeric: true }),
      ),
    [workspaces],
  );

  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setBusy(true);
    try {
      const next = await listNativeSkills(selectedWorkspaceId);
      if (generation !== loadGeneration.current) return;
      setView(next);
    } catch (err) {
      if (generation !== loadGeneration.current) return;
      setFeedback({
        variant: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      if (generation === loadGeneration.current) setBusy(false);
    }
  }, [selectedWorkspaceId]);

  useEffect(() => {
    void useWorkspaceStore
      .getState()
      .load()
      .catch((err) => {
        setFeedback({
          variant: "error",
          message: err instanceof Error ? err.message : String(err),
        });
      });
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const workspaceLabel = (workspace: Workspace) => {
    const path = workspaceSkillPath(workspace);
    if (path) return `${workspace.name} · ${shortWorkspaceDirLabel(path)}`;
    if (workspace.workspace_type === "ssh") {
      return `${workspace.name} · ${t("skills.filter.ssh")}`;
    }
    return workspace.name;
  };

  const dirLabel = (dir: string) => {
    if (dir === SKILL_DIR_GLOBAL) return t("skills.filter.globalDir");
    if (dir === SKILL_DIR_PLUGIN) return t("skills.filter.pluginDir");
    const workspaceIdFromFilter = parseWorkspaceSkillDirFilter(dir);
    const workspace = sortedWorkspaces.find((item) => item.id === workspaceIdFromFilter);
    if (workspace) return workspaceLabel(workspace);
    return dir;
  };

  const enabledCount = (view?.skills ?? []).filter(
    (skill) => !disabled.has(normalizePath(skill.skill_md_path)),
  ).length;

  const filtered = (view?.skills ?? []).filter((skill) => {
    const hay = `${skill.name} ${skill.description}`.toLowerCase();
    if (query && !hay.includes(query.toLowerCase())) return false;
    if (!skillBelongsToDir(skill, dirFilter)) return false;
    if (sourceFilter !== "all" && skill.source !== sourceFilter) return false;
    const on = !disabled.has(normalizePath(skill.skill_md_path));
    if (statusFilter === "enabled" && !on) return false;
    if (statusFilter === "disabled" && on) return false;
    return true;
  });

  const localSkills = filtered.filter((skill) => skill.source !== "plugin");
  const pluginSkills = filtered.filter((skill) => skill.source === "plugin");
  const warnings = (view?.diagnostics ?? []).filter((item) => item.level === "warning").length;
  const errors = (view?.diagnostics ?? []).filter((item) => item.level === "error").length;

  const toggle = async (skill: NativeSkill, enabled: boolean) => {
    try {
      const paths = await setNativeSkillEnabled(skill.skill_md_path, enabled);
      setView((current) => (current ? { ...current, disabled_paths: paths } : current));
    } catch (err) {
      setFeedback({
        variant: "error",
        message: err instanceof Error ? err.message : t("skills.toggleFailed"),
      });
    }
  };

  const copy = async (skill: NativeSkill) => {
    try {
      await copyNativeSkillToGlobal(skill.skill_md_path);
      setFeedback({ variant: "success", message: t("skills.copiedToCommon") });
      await load();
    } catch (err) {
      setFeedback({
        variant: "error",
        message: err instanceof Error ? err.message : t("skills.copyFailed"),
      });
    }
  };

  const remove = async (skill: NativeSkill) => {
    const ok = await confirm(t("skills.delete.description", { name: skill.name }), {
      title: t("skills.delete.title"),
      kind: "warning",
    });
    if (!ok) return;
    try {
      await deleteNativeSkill(skill.skill_md_path, mutationWorkspaceId);
      setFeedback({ variant: "success", message: t("skills.deleted") });
      await load();
    } catch (err) {
      setFeedback({
        variant: "error",
        message: err instanceof Error ? err.message : t("skills.deleteFailed"),
      });
    }
  };

  const sourceLabel = (skill: NativeSkill) =>
    skill.plugin
      ? `${t("skills.source.plugin")} ${skill.plugin}`
      : t(`skills.source.${skill.source}`);

  const renderGroup = (title: string, skills: NativeSkill[]) => {
    if (skills.length === 0) return null;
    return (
      <div className="space-y-2">
        <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
          {title}
        </p>
        {skills.map((skill) => {
          const on = !disabled.has(normalizePath(skill.skill_md_path));
          return (
            <div
              key={skill.skill_md_path}
              className="flex flex-col gap-2 rounded-xl border border-border/70 bg-card p-3.5"
            >
              <div className="flex items-start gap-3">
                <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                  <Wrench className="size-4" />
                </div>
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs font-semibold tracking-tight">{skill.name}</span>
                    <span className="rounded-md border border-border/50 bg-background px-1.5 py-0.5 text-[10px] text-muted-foreground">
                      {sourceLabel(skill)}
                    </span>
                    <span className="rounded-md border border-border/50 bg-background px-1.5 py-0.5 text-[10px] text-muted-foreground">
                      {on ? t("skills.enabled") : t("skills.disabled")}
                    </span>
                  </div>
                  <p className="text-[11px] leading-relaxed text-muted-foreground">
                    {skill.description || t("skills.noDescription")}
                  </p>
                  <p className="truncate font-mono text-[10px] text-muted-foreground">
                    {skill.skill_md_path}
                  </p>
                </div>
                <Switch checked={on} onCheckedChange={(checked) => void toggle(skill, checked)} />
              </div>
              <div className="flex flex-wrap gap-1.5">
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1 text-xs"
                  onClick={() => void openNativeSkillPath(skill.dir)}
                >
                  <FolderOpen className="size-3.5" />
                  {t("skills.openPath")}
                </Button>
                {skill.source !== "global" ? (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 gap-1 text-xs"
                    onClick={() => void copy(skill)}
                  >
                    <Copy className="size-3.5" />
                    {t("skills.copyToCommon")}
                  </Button>
                ) : null}
                {isManagedSource(skill.source) ? (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 gap-1 text-xs text-destructive"
                    onClick={() => void remove(skill)}
                  >
                    <Trash2 className="size-3.5" />
                    {t("skills.delete.action")}
                  </Button>
                ) : null}
              </div>
            </div>
          );
        })}
      </div>
    );
  };

  return (
    <div className="space-y-6">
      <SettingCard
        icon={Wrench}
        title={t("skills.title")}
        description={t("skills.hint")}
        badge={t("skills.footerSummary", {
          total: view?.skills.length ?? 0,
          enabled: enabledCount,
        })}
        headerAction={
          <div className="flex flex-wrap gap-1.5">
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1 text-xs"
              onClick={() => setCreateOpen(true)}
            >
              <Plus className="size-3.5" />
              {t("skills.create.open")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 text-xs"
              onClick={() => setImportOpen(true)}
            >
              {t("skills.import.open")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1 text-xs"
              disabled={busy}
              onClick={() => void load()}
            >
              <RefreshCw className="size-3.5" />
              {busy ? t("skills.refreshing") : t("skills.refresh")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1 text-xs"
              onClick={() => void openNativeSkillsDir()}
            >
              <FolderOpen className="size-3.5" />
              {t("skills.openDir")}
            </Button>
          </div>
        }
      >
        {feedback ? (
          <SettingFeedbackCallout
            variant={feedback.variant}
            message={feedback.message}
            onClose={() => setFeedback(null)}
          />
        ) : null}
        <div className="mb-3 flex flex-wrap gap-2">
          <Input
            className="h-8 max-w-xs text-xs"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("skills.searchPlaceholder")}
          />
          <Select
            value={dirFilter}
            onValueChange={(value) => {
              if (typeof value === "string") setDirFilter(value);
            }}
          >
            <SelectTrigger className="h-8 w-64 bg-background text-xs" title={dirLabel(dirFilter)}>
              <SelectValue>{() => dirLabel(dirFilter)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={SKILL_DIR_GLOBAL}>{t("skills.filter.globalDir")}</SelectItem>
              <SelectItem value={SKILL_DIR_PLUGIN}>{t("skills.filter.pluginDir")}</SelectItem>
              {sortedWorkspaces.map((workspace) => (
                <SelectItem
                  key={workspace.id}
                  value={workspaceSkillDirFilter(workspace.id)}
                  title={workspaceSkillPath(workspace) || workspace.name}
                >
                  {workspaceLabel(workspace)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={sourceFilter}
            onValueChange={(value) => {
              if (typeof value === "string") setSourceFilter(value as SourceFilter);
            }}
          >
            <SelectTrigger className="h-8 w-44 bg-background text-xs">
              <SelectValue>
                {() =>
                  sourceFilter === "all"
                    ? t("skills.filter.all")
                    : t(`skills.source.${sourceFilter}`)
                }
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("skills.filter.all")}</SelectItem>
              {(
                [
                  "global",
                  "workspace_noxcode",
                  "workspace_zcode",
                  "workspace_agents",
                  "workspace_claude",
                  "plugin",
                ] as NativeSkillSource[]
              ).map((source) => (
                <SelectItem key={source} value={source}>
                  {t(`skills.source.${source}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={statusFilter}
            onValueChange={(value) => {
              if (value === "all" || value === "enabled" || value === "disabled") {
                setStatusFilter(value);
              }
            }}
          >
            <SelectTrigger className="h-8 w-32 bg-background text-xs">
              <SelectValue>
                {() => t(`skills.filter.${statusFilter === "all" ? "all" : statusFilter}`)}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("skills.filter.all")}</SelectItem>
              <SelectItem value="enabled">{t("skills.filter.enabled")}</SelectItem>
              <SelectItem value="disabled">{t("skills.filter.disabled")}</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 text-center">
            <p className="text-xs font-semibold">{t("skills.empty")}</p>
          </div>
        ) : (
          <div className="space-y-5">
            {renderGroup(t("skills.group.local"), localSkills)}
            {renderGroup(t("skills.group.plugin"), pluginSkills)}
          </div>
        )}

        {view?.diagnostics.length ? (
          <div className="mt-4 border-t border-border/40 pt-3">
            <button
              type="button"
              className="text-[11px] text-muted-foreground underline-offset-2 hover:underline"
              onClick={() => setDiagnosticsOpen((open) => !open)}
            >
              {t("skills.diagnostics.summary", { errorCount: errors, warningCount: warnings })}{" "}
              {diagnosticsOpen ? t("skills.diagnostics.collapse") : t("skills.diagnostics.expand")}
            </button>
            {diagnosticsOpen ? (
              <ul className="mt-2 space-y-1 text-[11px] text-muted-foreground">
                {view.diagnostics.map((item) => (
                  <li key={`${item.code}:${item.path}`}>
                    {item.message} · {item.path}
                  </li>
                ))}
              </ul>
            ) : null}
          </div>
        ) : null}

        {view?.global_dir ? (
          <div className="mt-4 flex items-center justify-between border-t border-border/40 pt-3 text-[10px] font-mono text-muted-foreground">
            <span>{t("skills.storageDir")}</span>
            <span className="max-w-md truncate">{view.global_dir}</span>
          </div>
        ) : null}
      </SettingCard>

      <SkillCreateDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        onCreated={() => void load()}
      />
      <SkillImportDialog
        open={importOpen}
        workspaceId={mutationWorkspaceId}
        onOpenChange={setImportOpen}
        onImported={() => void load()}
      />
    </div>
  );
}
