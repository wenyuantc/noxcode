import {
  Brain,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  FileText,
  FolderOpen,
  Layers,
  Loader2,
  RefreshCw,
  Search,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  deleteNativeMemory,
  dreamNativeMemory,
  listNativeMemories,
  openNativeMemoryDir,
  updateNativeSettings,
} from "@/lib/backend";
import { resolveMemoryWorkspaceId } from "@/lib/memoryWorkspace";
import { errorMessage, showToast } from "@/lib/toast";
import type { NativeMemoryEntry, NativeMemoryView } from "@/lib/types";
import { cn } from "@/lib/utils";
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
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useChannelStore } from "@/stores/channelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard, SettingRow } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

const KIND_BADGE_STYLES: Record<string, string> = {
  project:
    "border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-400 hover:bg-blue-500/15",
  user: "border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-400 hover:bg-purple-500/15",
  feedback:
    "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400 hover:bg-amber-500/15",
  reference:
    "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 hover:bg-emerald-500/15",
  default: "border-border/60 bg-muted/50 text-muted-foreground",
};

const KIND_CHIP_ACTIVE_STYLES: Record<string, string> = {
  all: "border-primary bg-primary/10 text-primary",
  project: "border-blue-500/50 bg-blue-500/15 text-blue-600 dark:text-blue-400",
  user: "border-purple-500/50 bg-purple-500/15 text-purple-600 dark:text-purple-400",
  feedback: "border-amber-500/50 bg-amber-500/15 text-amber-600 dark:text-amber-400",
  reference: "border-emerald-500/50 bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
};

export function shortenPath(fullPath: string): string {
  const normalized = fullPath.replace(/\\/g, "/");
  const parts = normalized.split("/").filter(Boolean);
  if (parts.length <= 2) return fullPath;
  return `…/${parts.slice(-2).join("/")}`;
}

export function calculateKindCounts(entries?: NativeMemoryEntry[] | null): Record<string, number> {
  const counts: Record<string, number> = {
    all: entries?.length ?? 0,
    project: 0,
    user: 0,
    feedback: 0,
    reference: 0,
  };
  if (!entries) return counts;
  for (const entry of entries) {
    const k = entry.kind in counts ? entry.kind : "reference";
    counts[k] = (counts[k] ?? 0) + 1;
  }
  return counts;
}

export function filterMemoryEntries(
  entries?: NativeMemoryEntry[] | null,
  searchQuery = "",
  selectedKind = "all",
): NativeMemoryEntry[] {
  if (!entries) return [];
  const q = searchQuery.trim().toLowerCase();
  return entries.filter((entry) => {
    if (selectedKind !== "all" && entry.kind !== selectedKind) {
      return false;
    }
    if (!q) return true;
    return (
      entry.name.toLowerCase().includes(q) ||
      entry.description.toLowerCase().includes(q) ||
      entry.file_name.toLowerCase().includes(q) ||
      entry.body.toLowerCase().includes(q)
    );
  });
}

export function MemorySection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const activeWorkspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const [workspaceId, setWorkspaceId] = useState<string | null>(activeWorkspaceId);
  const workspaceIdRef = useRef(workspaceId);
  workspaceIdRef.current = workspaceId;
  const channelId = useChannelStore((state) => state.activeChannelId);

  const [view, setView] = useState<NativeMemoryView | null>(null);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [interval, setInterval] = useState<number>(native?.memory_dream_interval ?? 10);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedKind, setSelectedKind] = useState<string>("all");
  const [copiedFile, setCopiedFile] = useState<string | null>(null);
  const [copiedDir, setCopiedDir] = useState(false);
  const [indexDialogOpen, setIndexDialogOpen] = useState(false);

  // 仅承载需要常驻页面内的提示：列表加载失败、Dream 长摘要与缺少渠道指引。
  const [inlineFeedback, setInlineFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);

  useEffect(() => {
    void useWorkspaceStore.getState().load();
  }, []);

  useEffect(() => {
    const workspaceIds = workspaces.map((item) => item.id);
    setWorkspaceId((current) => resolveMemoryWorkspaceId(current, activeWorkspaceId, workspaceIds));
  }, [activeWorkspaceId, workspaces]);

  useEffect(() => {
    if (native) setInterval(native.memory_dream_interval);
  }, [native]);

  const reload = useCallback(() => {
    if (!workspaceId) {
      setView(null);
      return;
    }
    const requested = workspaceId;
    listNativeMemories(requested)
      .then((next) => {
        if (workspaceIdRef.current === requested) setView(next);
      })
      .catch((reason: unknown) => {
        if (workspaceIdRef.current === requested) {
          setInlineFeedback({ variant: "error", message: errorMessage(reason) });
        }
      });
  }, [workspaceId]);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    setOpen(null);
    setInlineFeedback(null);
    setView(null);
    setSearchQuery("");
    setSelectedKind("all");
  }, [workspaceId]);

  const remove = (entry: NativeMemoryEntry) => {
    if (!workspaceId) return;
    deleteNativeMemory(workspaceId, entry.file_name)
      .then(() => {
        showToast({ variant: "success", description: t("common:deleted") });
        reload();
      })
      .catch((reason: unknown) => {
        showToast({ variant: "error", description: errorMessage(reason) });
      });
  };

  const dream = () => {
    if (!workspaceId || !channelId) {
      setInlineFeedback({ variant: "error", message: t("settings:memory.needChannel") });
      return;
    }
    setBusy(true);
    setInlineFeedback(null);
    dreamNativeMemory(workspaceId, channelId)
      .then((summary) => {
        setInlineFeedback({ variant: "success", message: summary });
        reload();
      })
      .catch((reason: unknown) => {
        showToast({ variant: "error", description: errorMessage(reason) });
      })
      .finally(() => setBusy(false));
  };

  const saveInterval = () => {
    void updateNativeSettings({ memory_dream_interval: interval })
      .then((res) => {
        setNative(res);
        showToast({ variant: "success", description: t("common:saved") });
      })
      .catch((err: unknown) => {
        showToast({ variant: "error", description: errorMessage(err) });
      });
  };

  const copyText = (text: string, id: string) => {
    void navigator.clipboard.writeText(text).then(() => {
      setCopiedFile(id);
      showToast({ variant: "success", description: t("settings:memory.copySuccess") });
      setTimeout(() => setCopiedFile(null), 2000);
    });
  };

  const copyDirectory = (dir: string) => {
    void navigator.clipboard.writeText(dir).then(() => {
      setCopiedDir(true);
      showToast({ variant: "success", description: t("settings:memory.copySuccess") });
      setTimeout(() => setCopiedDir(false), 2000);
    });
  };

  // 各分类统计
  const kindCounts = useMemo(() => calculateKindCounts(view?.entries), [view?.entries]);

  // 检索与分类过滤
  const filteredEntries = useMemo(
    () => filterMemoryEntries(view?.entries, searchQuery, selectedKind),
    [view?.entries, searchQuery, selectedKind],
  );

  const categories: Array<{ key: string; label: string }> = [
    { key: "all", label: t("settings:memory.filterAll") },
    { key: "project", label: t("settings:memory.kind.project") },
    { key: "user", label: t("settings:memory.kind.user") },
    { key: "feedback", label: t("settings:memory.kind.feedback") },
    { key: "reference", label: t("settings:memory.kind.reference") },
  ];

  if (!native) return null;

  return (
    <TooltipProvider>
      <div className="space-y-6">
        {inlineFeedback ? (
          <SettingFeedbackCallout
            variant={inlineFeedback.variant}
            message={inlineFeedback.message}
            onClose={() => setInlineFeedback(null)}
          />
        ) : null}

        {/* 记忆配置与指标卡片 */}
        <SettingCard
          icon={Brain}
          title={t("settings:memory.settingsTitle")}
          description={t("settings:memory.settingsHint")}
          divided
        >
          {/* 指标小卡行 */}
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 p-4 sm:p-5 bg-muted/15">
            {/* 指标 1：已存记忆 */}
            <div className="flex flex-col gap-1 rounded-xl border border-border/60 bg-card/70 p-3 shadow-2xs">
              <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Brain className="size-3.5 text-primary" />
                <span>{t("settings:memory.totalEntries")}</span>
              </div>
              <div className="mt-1 flex items-baseline gap-1.5">
                <span className="text-xl font-semibold tracking-tight text-foreground font-mono">
                  {view?.entries.length ?? 0}
                </span>
                <span className="text-[11px] text-muted-foreground">条</span>
              </div>
            </div>

            {/* 指标 2：累计抽取 */}
            <div className="flex flex-col gap-1 rounded-xl border border-border/60 bg-card/70 p-3 shadow-2xs">
              <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Sparkles className="size-3.5 text-amber-500" />
                <span>{t("settings:memory.extractionsCount")}</span>
              </div>
              <div className="mt-1 flex items-baseline gap-1.5">
                <span className="text-xl font-semibold tracking-tight text-foreground font-mono">
                  {view?.extractions ?? 0}
                </span>
                <span className="text-[11px] text-muted-foreground">次</span>
              </div>
            </div>

            {/* 指标 3：深度整理 */}
            <div className="flex flex-col gap-1 rounded-xl border border-border/60 bg-card/70 p-3 shadow-2xs">
              <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Layers className="size-3.5 text-blue-500" />
                <span>{t("settings:memory.dreamsCount")}</span>
              </div>
              <div className="mt-1 flex items-baseline gap-1.5">
                <span className="text-xl font-semibold tracking-tight text-foreground font-mono">
                  {view?.dreams ?? 0}
                </span>
                <span className="text-[11px] text-muted-foreground">次</span>
              </div>
            </div>

            {/* 指标 4：存储目录 */}
            <div className="flex flex-col justify-between gap-1 rounded-xl border border-border/60 bg-card/70 p-3 shadow-2xs">
              <div className="flex items-center justify-between gap-1 text-xs text-muted-foreground">
                <div className="flex items-center gap-1.5">
                  <FolderOpen className="size-3.5 text-violet-500" />
                  <span>{t("settings:memory.storageDir")}</span>
                </div>
                {view?.dir ? (
                  <div className="flex items-center gap-0.5">
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      className="size-6 text-muted-foreground hover:text-foreground"
                      onClick={() => copyDirectory(view.dir)}
                      title={t("settings:memory.copyDir")}
                    >
                      {copiedDir ? (
                        <Check className="size-3 text-emerald-500" />
                      ) : (
                        <Copy className="size-3" />
                      )}
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      className="size-6 text-muted-foreground hover:text-foreground"
                      onClick={() => workspaceId && void openNativeMemoryDir(workspaceId)}
                      title={t("settings:memory.openDir")}
                    >
                      <FolderOpen className="size-3" />
                    </Button>
                  </div>
                ) : null}
              </div>
              <div className="mt-1">
                {view?.dir ? (
                  <Tooltip>
                    <TooltipTrigger className="block max-w-full truncate text-left text-xs font-mono text-muted-foreground/90 cursor-help">
                      {shortenPath(view.dir)}
                    </TooltipTrigger>
                    <TooltipContent
                      side="bottom"
                      className="max-w-md break-all font-mono text-[11px]"
                    >
                      {view.dir}
                    </TooltipContent>
                  </Tooltip>
                ) : (
                  <span className="text-xs text-muted-foreground/60">—</span>
                )}
              </div>
            </div>
          </div>

          {/* 管线开关与周期配置 */}
          <SettingRow
            title={t("settings:memory.enabled")}
            description="启用后，Agent 将自动提取与学习对话中的长期记忆。"
          >
            <Switch
              id="memory-enabled"
              checked={native.memory_enabled}
              onCheckedChange={(checked) => {
                void updateNativeSettings({ memory_enabled: checked })
                  .then((res) => {
                    setNative(res);
                    showToast({ variant: "success", description: t("common:saved") });
                  })
                  .catch((err: unknown) => {
                    showToast({ variant: "error", description: errorMessage(err) });
                  });
              }}
            />
          </SettingRow>

          <SettingRow
            title={t("settings:memory.dreamInterval")}
            description="每隔指定会话轮次自动触发一次记忆提炼与深度巩固（Dream）。"
          >
            <div className="flex items-center gap-2">
              <div className="relative w-28">
                <Input
                  id="memory-dream-interval"
                  className="h-8 pr-7 text-xs font-mono text-right"
                  type="number"
                  min={0}
                  max={1000}
                  step={1}
                  value={interval}
                  onChange={(e) => setInterval(Number(e.target.value))}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") saveInterval();
                  }}
                />
                <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
                  轮
                </span>
              </div>
              <Button size="sm" variant="outline" className="h-8 text-xs" onClick={saveInterval}>
                {t("common:save")}
              </Button>
            </div>
          </SettingRow>
        </SettingCard>

        {/* 记忆实体条目管理 */}
        <SettingCard
          icon={Sparkles}
          title={t("settings:memory.entriesTitle")}
          description={t("settings:memory.hint")}
          badge={
            view
              ? `${filteredEntries.length === view.entries.length ? view.entries.length : `${filteredEntries.length}/${view.entries.length}`} 条记忆`
              : undefined
          }
          headerAction={
            view ? (
              <div className="flex items-center gap-1.5">
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 text-xs gap-1"
                  disabled={!view.index}
                  onClick={() => setIndexDialogOpen(true)}
                >
                  <FileText className="size-3" />
                  {t("settings:memory.viewIndex")}
                </Button>
                <Button variant="outline" size="sm" className="h-7 text-xs gap-1" onClick={reload}>
                  <RefreshCw className="size-3" />
                  {t("settings:memory.refresh")}
                </Button>
                <Button
                  size="sm"
                  className="h-7 text-xs gap-1"
                  disabled={busy || view.entries.length === 0}
                  onClick={dream}
                >
                  {busy ? (
                    <Loader2 className="size-3 animate-spin" />
                  ) : (
                    <Sparkles className="size-3" />
                  )}
                  {busy ? t("settings:memory.dreaming") : t("settings:memory.dream")}
                </Button>
              </div>
            ) : null
          }
        >
          <div className="space-y-4">
            {/* 工具栏：工作区选择器 + 关键词搜索框 */}
            <div className="flex flex-col sm:flex-row items-stretch sm:items-center gap-2.5">
              <div className="w-full sm:w-56 shrink-0">
                <Select
                  value={workspaceId ?? undefined}
                  disabled={workspaces.length === 0}
                  onValueChange={(value) => {
                    if (typeof value === "string") setWorkspaceId(value);
                  }}
                >
                  <SelectTrigger id="memory-workspace" className="h-8 w-full bg-background text-xs">
                    <SelectValue>
                      {() =>
                        workspaces.find((item) => item.id === workspaceId)?.name ??
                        t("settings:memory.noWorkspaces")
                      }
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {workspaces.map((workspace) => (
                      <SelectItem key={workspace.id} value={workspace.id} className="text-xs">
                        {workspace.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="relative flex-1">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  type="text"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  placeholder={t("settings:memory.searchPlaceholder")}
                  className="h-8 pl-8 pr-8 text-xs bg-background"
                />
                {searchQuery ? (
                  <button
                    type="button"
                    onClick={() => setSearchQuery("")}
                    className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                  >
                    <X className="size-3.5" />
                  </button>
                ) : null}
              </div>
            </div>

            {/* 分类 Filter Chips */}
            {view && view.entries.length > 0 ? (
              <div className="flex flex-wrap items-center gap-1.5 pt-0.5">
                {categories.map((cat) => {
                  const active = selectedKind === cat.key;
                  const count =
                    cat.key === "all" ? (view?.entries.length ?? 0) : (kindCounts[cat.key] ?? 0);
                  return (
                    <button
                      key={cat.key}
                      type="button"
                      onClick={() => setSelectedKind(cat.key)}
                      className={cn(
                        "inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-medium transition-all",
                        active
                          ? (KIND_CHIP_ACTIVE_STYLES[cat.key] ??
                              "border-primary bg-primary/10 text-primary")
                          : "border-border/60 bg-muted/30 text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                      )}
                    >
                      <span>{cat.label}</span>
                      <span
                        className={cn(
                          "rounded-full px-1.5 py-0.2 text-[10px] font-mono leading-tight",
                          active ? "bg-current/15 text-current" : "bg-muted text-muted-foreground",
                        )}
                      >
                        {count}
                      </span>
                    </button>
                  );
                })}
              </div>
            ) : null}

            {/* 列表渲染 */}
            {!workspaceId ? (
              <div className="py-8 text-center text-xs text-muted-foreground">
                {t("settings:memory.needWorkspace")}
              </div>
            ) : view ? (
              view.entries.length === 0 ? (
                <div className="flex flex-col items-center justify-center py-10 text-center">
                  <div className="flex size-10 items-center justify-center rounded-xl border border-border/70 bg-muted/30 text-muted-foreground">
                    <Brain className="size-5" />
                  </div>
                  <p className="mt-3 text-xs font-medium text-foreground">
                    {t("settings:memory.empty")}
                  </p>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    Agent 在会话过程中会自动沉淀记忆文件，并展示在此处。
                  </p>
                </div>
              ) : filteredEntries.length === 0 ? (
                <div className="flex flex-col items-center justify-center py-10 text-center rounded-xl border border-dashed border-border/70 p-6">
                  <Search className="size-8 text-muted-foreground/40 mb-2" />
                  <p className="text-xs font-medium text-foreground">
                    {t("settings:memory.noMatches")}
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    className="mt-3 text-xs h-7"
                    onClick={() => {
                      setSearchQuery("");
                      setSelectedKind("all");
                    }}
                  >
                    {t("settings:memory.clearFilter")}
                  </Button>
                </div>
              ) : (
                <div className="space-y-2">
                  {filteredEntries.map((entry) => {
                    const isOpen = open === entry.file_name;
                    return (
                      <div
                        key={entry.file_name}
                        className={cn(
                          "group rounded-xl border transition-all duration-150 overflow-hidden",
                          isOpen
                            ? "border-primary/40 bg-card shadow-2xs ring-1 ring-primary/20"
                            : "border-border/70 bg-card hover:border-border hover:shadow-2xs",
                        )}
                      >
                        {/* 条目首行与描述（上下两行结构） */}
                        <div className="p-3">
                          <div className="flex items-center justify-between gap-3">
                            <button
                              type="button"
                              className="flex min-w-0 flex-1 items-center gap-2 text-left group/title"
                              onClick={() => setOpen(isOpen ? null : entry.file_name)}
                            >
                              {isOpen ? (
                                <ChevronDown className="size-3.5 shrink-0 text-muted-foreground transition-transform" />
                              ) : (
                                <ChevronRight className="size-3.5 shrink-0 text-muted-foreground transition-transform" />
                              )}
                              <span className="font-medium text-xs text-foreground group-hover/title:text-primary transition-colors truncate">
                                {entry.name}
                              </span>
                              <span
                                className={cn(
                                  "inline-flex items-center rounded-md border px-1.5 py-0.5 text-[10px] font-mono shrink-0 transition-colors",
                                  KIND_BADGE_STYLES[entry.kind] ?? KIND_BADGE_STYLES.default,
                                )}
                              >
                                {t(`settings:memory.kind.${entry.kind}`, {
                                  defaultValue: entry.kind,
                                })}
                              </span>
                            </button>
                            <div className="flex items-center gap-2 shrink-0">
                              <span className="text-[11px] text-muted-foreground font-mono">
                                {entry.updated_at}
                              </span>
                              <Button
                                variant="ghost"
                                size="icon-xs"
                                className="size-7 text-muted-foreground opacity-60 hover:text-destructive hover:opacity-100 hover:bg-destructive/10 transition-all"
                                onClick={() => remove(entry)}
                                title={t("common:delete")}
                              >
                                <Trash2 className="size-3.5" />
                              </Button>
                            </div>
                          </div>

                          {/* 独立第二行：描述信息 */}
                          {entry.description ? (
                            <p
                              className="mt-1.5 pl-5.5 text-xs text-muted-foreground line-clamp-2 leading-relaxed cursor-pointer hover:text-foreground/90 transition-colors"
                              onClick={() => setOpen(isOpen ? null : entry.file_name)}
                            >
                              {entry.description}
                            </p>
                          ) : null}
                        </div>

                        {/* 展开详情 */}
                        {isOpen ? (
                          <div className="border-t border-border/50 bg-muted/20 p-3.5 space-y-2">
                            <div className="flex items-center justify-between text-[11px] text-muted-foreground">
                              <div className="flex items-center gap-2 font-mono truncate max-w-[70%]">
                                <FileText className="size-3 shrink-0" />
                                <span className="truncate">{entry.file_name}</span>
                                {entry.created_at ? (
                                  <span className="text-muted-foreground/60 shrink-0 hidden sm:inline">
                                    · {t("settings:memory.createdAt")} {entry.created_at}
                                  </span>
                                ) : null}
                              </div>
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-6 px-2 text-[11px] gap-1 text-muted-foreground hover:text-foreground shrink-0"
                                onClick={() => copyText(entry.body, entry.file_name)}
                              >
                                {copiedFile === entry.file_name ? (
                                  <Check className="size-3 text-emerald-500" />
                                ) : (
                                  <Copy className="size-3" />
                                )}
                                {copiedFile === entry.file_name
                                  ? t("settings:memory.copySuccess")
                                  : t("settings:memory.copyContent")}
                              </Button>
                            </div>
                            <pre className="max-h-72 overflow-auto rounded-lg border border-border/50 bg-background/80 p-3 font-mono text-[11px] leading-relaxed text-foreground whitespace-pre-wrap selection:bg-primary/20">
                              {entry.body}
                            </pre>
                          </div>
                        ) : null}
                      </div>
                    );
                  })}
                </div>
              )
            ) : null}
          </div>
        </SettingCard>

        {/* MEMORY.md 总索引查看弹窗 */}
        <Dialog open={indexDialogOpen} onOpenChange={setIndexDialogOpen}>
          <DialogContent className="max-w-2xl max-h-[85vh] flex flex-col">
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <FileText className="size-4 text-primary" />
                {t("settings:memory.indexTitle")}
              </DialogTitle>
              <DialogDescription>{t("settings:memory.indexHint")}</DialogDescription>
            </DialogHeader>
            <div className="flex-1 min-h-0 py-2">
              {view?.index ? (
                <pre className="h-full max-h-[50vh] overflow-auto rounded-lg border border-border/60 bg-muted/20 p-4 font-mono text-xs leading-relaxed text-foreground whitespace-pre-wrap">
                  {view.index}
                </pre>
              ) : (
                <div className="py-8 text-center text-xs text-muted-foreground">
                  {t("settings:memory.indexEmpty")}
                </div>
              )}
            </div>
            <DialogFooter className="flex items-center justify-between sm:justify-between">
              <div className="text-[11px] text-muted-foreground font-mono truncate max-w-xs">
                {view?.dir}
              </div>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!view?.index}
                  onClick={() => {
                    if (view?.index) {
                      void navigator.clipboard.writeText(view.index);
                      showToast({
                        variant: "success",
                        description: t("settings:memory.copySuccess"),
                      });
                    }
                  }}
                >
                  <Copy className="size-3.5 mr-1" />
                  {t("settings:memory.copyIndex")}
                </Button>
                <Button size="sm" onClick={() => setIndexDialogOpen(false)}>
                  {t("common:close")}
                </Button>
              </div>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>
    </TooltipProvider>
  );
}
