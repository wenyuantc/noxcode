import { Check, Copy, Folder, FolderOpen, GitFork, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { listManagedWorktrees, removeManagedWorktree, updateNativeSettings } from "@/lib/backend";
import type { ManagedWorktreeList, NativeSettings } from "@/lib/types";
import { formatRelativeTime } from "@/lib/utils";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard, SettingRow } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

async function chooseDirectory(): Promise<string | null> {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({ directory: true, multiple: false });
    return typeof selected === "string" ? selected : null;
  } catch {
    return null;
  }
}

async function openDirectory(path: string): Promise<void> {
  try {
    const { openPath, revealItemInDir } = await import("@tauri-apps/plugin-opener");
    try {
      await revealItemInDir(path);
    } catch {
      await openPath(path);
    }
  } catch {
    // 桌面外或无权限静默降级
  }
}

export interface WorktreesSectionProps {
  initialList?: ManagedWorktreeList | null;
}

export function WorktreesSection({ initialList = null }: WorktreesSectionProps = {}) {
  const { t, i18n } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [draft, setDraft] = useState(native);
  const [list, setList] = useState<ManagedWorktreeList | null>(initialList);
  const [loading, setLoading] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  const [confirmingPath, setConfirmingPath] = useState<string | null>(null);
  const [copiedPath, setCopiedPath] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);

  const initializedRef = useRef(false);
  const draftRef = useRef(draft);
  draftRef.current = draft;

  useEffect(() => {
    if (native && !initializedRef.current) {
      initializedRef.current = true;
      setDraft(native);
    }
  }, [native]);

  const persist = useCallback(
    async (value: NativeSettings) => {
      try {
        const updated = await updateNativeSettings({
          worktree_root: value.worktree_root,
          worktree_fetch_before_create: value.worktree_fetch_before_create,
          worktree_auto_prune: value.worktree_auto_prune,
          worktree_auto_prune_limit: value.worktree_auto_prune_limit,
        });
        setNative(updated);
      } catch {
        // 静默失败，保留草稿，下次变更自动重试
      }
    },
    [setNative],
  );

  useEffect(() => {
    if (!initializedRef.current) return;
    const timer = window.setTimeout(() => {
      if (draftRef.current) void persist(draftRef.current);
    }, 600);
    return () => window.clearTimeout(timer);
  }, [draft, persist]);

  const reload = useCallback(() => {
    setLoading(true);
    listManagedWorktrees()
      .then((next) => {
        setList(next);
        setFeedback(null);
      })
      .catch((error: unknown) => {
        setFeedback({
          variant: "error",
          message: error instanceof Error ? error.message : String(error),
        });
      })
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    if (!confirmingPath) return;

    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && !target.closest("[data-worktree-delete-confirm]")) {
        setConfirmingPath(null);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setConfirmingPath(null);
      }
    };

    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [confirmingPath]);

  const handleCopy = (path: string) => {
    void navigator.clipboard.writeText(path);
    setCopiedPath(path);
    window.setTimeout(() => {
      setCopiedPath((curr) => (curr === path ? null : curr));
    }, 1500);
  };

  const handleRemove = (path: string) => {
    setRemoving(path);
    void removeManagedWorktree(path)
      .then((next) => {
        setList(next);
        setFeedback(null);
        setConfirmingPath(null);
      })
      .catch((error: unknown) => {
        setFeedback({
          variant: "error",
          message: error instanceof Error ? error.message : String(error),
        });
      })
      .finally(() => setRemoving(null));
  };

  if (!native || !draft) return null;

  const items = list?.items ?? [];

  return (
    <div className="space-y-6">
      <SettingCard
        icon={GitFork}
        title={t("settings:worktrees.configTitle")}
        description={t("settings:worktrees.configDescription")}
        divided
      >
        <SettingRow
          title={t("settings:worktrees.root")}
          description={t("settings:worktrees.rootHint")}
          vertical
        >
          <div className="flex w-full gap-2">
            <Input
              id="worktree-root"
              className="h-8 flex-1 font-mono text-xs"
              value={draft.worktree_root}
              placeholder={list?.default_root ?? ""}
              onChange={(event) => setDraft({ ...draft, worktree_root: event.target.value })}
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 text-xs"
              onClick={() => {
                void chooseDirectory().then((selected) => {
                  if (selected) setDraft({ ...draft, worktree_root: selected });
                });
              }}
            >
              <FolderOpen className="size-3.5" />
              {t("settings:worktrees.browse")}
            </Button>
          </div>
        </SettingRow>

        <SettingRow
          title={t("settings:worktrees.fetchBeforeCreate")}
          description={t("settings:worktrees.fetchBeforeCreateHint")}
        >
          <Switch
            checked={draft.worktree_fetch_before_create}
            onCheckedChange={(checked) =>
              setDraft({ ...draft, worktree_fetch_before_create: checked })
            }
          />
        </SettingRow>

        <SettingRow
          title={t("settings:worktrees.autoPrune")}
          description={t("settings:worktrees.autoPruneHint")}
        >
          <Switch
            checked={draft.worktree_auto_prune}
            onCheckedChange={(checked) => setDraft({ ...draft, worktree_auto_prune: checked })}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:worktrees.autoPruneLimit")}
          description={t("settings:worktrees.autoPruneLimitHint")}
        >
          <Input
            id="worktree-prune-limit"
            className="h-8 w-24 text-xs font-mono text-right"
            type="number"
            min={1}
            max={200}
            step={1}
            disabled={!draft.worktree_auto_prune}
            value={draft.worktree_auto_prune_limit}
            onChange={(event) =>
              setDraft({ ...draft, worktree_auto_prune_limit: Number(event.target.value) })
            }
          />
        </SettingRow>
      </SettingCard>

      <SettingCard
        icon={GitFork}
        title={
          items.length === 0
            ? t("settings:worktrees.listEmptyTitle")
            : t("settings:worktrees.managedTitle")
        }
        description={
          items.length === 0
            ? t("settings:worktrees.listEmpty")
            : t("settings:worktrees.managedDescription")
        }
        badge={items.length > 0 ? String(items.length) : undefined}
        headerAction={
          <Button
            variant="outline"
            size="sm"
            className="h-7 text-xs gap-1"
            onClick={reload}
            disabled={loading}
          >
            {loading ? (
              <Loader2 className="size-3 animate-spin" />
            ) : (
              <RefreshCw className="size-3" />
            )}
            {t("settings:worktrees.refresh")}
          </Button>
        }
      >
        {feedback ? (
          <div className="px-5 pt-4">
            <SettingFeedbackCallout variant={feedback.variant} message={feedback.message} />
          </div>
        ) : null}
        {items.length === 0 ? (
          <div className="flex min-h-40 items-center justify-center px-5 py-10 text-center">
            <p className="text-xs text-muted-foreground">{t("settings:worktrees.listEmpty")}</p>
          </div>
        ) : (
          <div className="divide-y divide-border/50">
            {items.map((item) => {
              const isConfirming = confirmingPath === item.path;
              const isRemoving = removing === item.path;

              return (
                <div
                  key={`${item.session_id}:${item.path}`}
                  className="flex flex-col gap-2.5 px-5 py-3.5 transition-colors hover:bg-muted/20"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-xs font-medium text-foreground truncate max-w-sm">
                          {item.title || t("settings:worktrees.untitled")}
                        </span>
                        {item.workspace_name ? (
                          <span className="inline-flex items-center gap-1 rounded-md border border-border/70 bg-muted/60 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                            <Folder className="size-2.5" />
                            {item.workspace_name}
                          </span>
                        ) : null}
                        {item.in_use ? (
                          <span className="inline-flex items-center gap-1 rounded-md border border-emerald-500/30 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                            <span className="size-1.5 rounded-full bg-emerald-500 animate-pulse" />
                            {t("settings:worktrees.inUse")}
                          </span>
                        ) : null}
                        {item.remote ? (
                          <span className="inline-flex items-center rounded-md border border-sky-500/30 bg-sky-500/10 px-1.5 py-0.5 text-[10px] font-medium text-sky-600 dark:text-sky-400">
                            {t("settings:worktrees.remote")}
                          </span>
                        ) : null}
                        {!item.exists && !item.remote ? (
                          <span className="inline-flex items-center rounded-md border border-destructive/30 bg-destructive/10 px-1.5 py-0.5 text-[10px] font-medium text-destructive">
                            {t("settings:worktrees.missing")}
                          </span>
                        ) : null}
                        {item.created_at ? (
                          <span className="text-[11px] text-muted-foreground/60">
                            · {formatRelativeTime(item.created_at, i18n?.language ?? "zh-CN")}
                          </span>
                        ) : null}
                      </div>
                    </div>

                    <div className="shrink-0">
                      {isConfirming ? (
                        <div
                          data-worktree-delete-confirm
                          className="flex items-center gap-1.5 animate-in fade-in zoom-in-95 duration-150"
                        >
                          <Button
                            type="button"
                            variant="destructive"
                            size="sm"
                            className="h-7 px-2.5 text-xs gap-1 font-medium shadow-2xs"
                            disabled={isRemoving}
                            onClick={() => handleRemove(item.path)}
                          >
                            {isRemoving ? (
                              <Loader2 className="size-3 animate-spin" />
                            ) : (
                              <Trash2 className="size-3" />
                            )}
                            {t("settings:worktrees.confirmDelete")}
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
                            disabled={isRemoving}
                            onClick={() => setConfirmingPath(null)}
                          >
                            {t("common:cancel")}
                          </Button>
                        </div>
                      ) : (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-xs"
                          className="text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition-colors"
                          disabled={item.in_use || isRemoving}
                          title={
                            item.in_use
                              ? t("settings:worktrees.inUseTooltip")
                              : t("settings:worktrees.delete")
                          }
                          onClick={() => setConfirmingPath(item.path)}
                        >
                          {isRemoving ? (
                            <Loader2 className="size-3.5 animate-spin" />
                          ) : (
                            <Trash2 className="size-3.5" />
                          )}
                        </Button>
                      )}
                    </div>
                  </div>

                  <div className="flex items-center justify-between gap-2 rounded-lg border border-border/50 bg-muted/30 px-2.5 py-1.5">
                    <span className="font-mono text-[11px] text-muted-foreground break-all select-all flex-1 min-w-0">
                      {item.path}
                    </span>
                    <div className="flex items-center gap-1 shrink-0">
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-xs"
                        className="h-6 w-6 text-muted-foreground hover:text-foreground"
                        title={
                          copiedPath === item.path
                            ? t("settings:worktrees.copied")
                            : t("settings:worktrees.copyPath")
                        }
                        onClick={() => handleCopy(item.path)}
                      >
                        {copiedPath === item.path ? (
                          <Check className="size-3 text-emerald-500" />
                        ) : (
                          <Copy className="size-3" />
                        )}
                      </Button>
                      {item.exists && !item.remote ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-xs"
                          className="h-6 w-6 text-muted-foreground hover:text-foreground"
                          title={t("settings:worktrees.openInFolder")}
                          onClick={() => void openDirectory(item.path)}
                        >
                          <FolderOpen className="size-3" />
                        </Button>
                      ) : null}
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </SettingCard>
    </div>
  );
}
