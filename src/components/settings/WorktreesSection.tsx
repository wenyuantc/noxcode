import { FolderOpen, GitFork, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { listManagedWorktrees, removeManagedWorktree, updateNativeSettings } from "@/lib/backend";
import type { ManagedWorktreeList, NativeSettings } from "@/lib/types";
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

export function WorktreesSection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [draft, setDraft] = useState(native);
  const [list, setList] = useState<ManagedWorktreeList | null>(null);
  const [loading, setLoading] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
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

  if (!native || !draft) return null;

  const items = list?.items ?? [];

  return (
    <div className="space-y-6">
      <p className="text-xs text-muted-foreground">{t("settings:worktrees.hint")}</p>

      <SettingCard icon={GitFork} title={t("settings:sections.worktrees")} divided>
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
            : t("settings:worktrees.listTitle")
        }
        description={items.length === 0 ? t("settings:worktrees.listEmpty") : undefined}
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
            {items.map((item) => (
              <div
                key={`${item.session_id}:${item.path}`}
                className="flex items-start gap-3 px-5 py-3.5"
              >
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs font-medium text-foreground truncate">
                      {item.title || t("settings:worktrees.untitled")}
                    </span>
                    {item.in_use ? (
                      <span className="inline-flex items-center rounded-md border border-border/60 bg-muted/50 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {t("settings:worktrees.inUse")}
                      </span>
                    ) : null}
                    {item.remote ? (
                      <span className="inline-flex items-center rounded-md border border-border/60 bg-muted/50 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {t("settings:worktrees.remote")}
                      </span>
                    ) : null}
                    {!item.exists && !item.remote ? (
                      <span className="inline-flex items-center rounded-md border border-destructive/30 bg-destructive/10 px-1.5 py-0.5 text-[10px] text-destructive">
                        {t("settings:worktrees.missing")}
                      </span>
                    ) : null}
                  </div>
                  <p className="font-mono text-[11px] text-muted-foreground break-all">
                    {item.path}
                  </p>
                  {item.workspace_name ? (
                    <p className="text-[11px] text-muted-foreground">{item.workspace_name}</p>
                  ) : null}
                </div>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  className="text-muted-foreground hover:text-destructive"
                  disabled={item.in_use || removing === item.path}
                  title={t("settings:worktrees.delete")}
                  onClick={() => {
                    setRemoving(item.path);
                    void removeManagedWorktree(item.path)
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
                      .finally(() => setRemoving(null));
                  }}
                >
                  {removing === item.path ? (
                    <Loader2 className="size-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="size-3.5" />
                  )}
                </Button>
              </div>
            ))}
          </div>
        )}
      </SettingCard>
    </div>
  );
}
