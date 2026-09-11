import { useEffect, useState } from "react";
import { Bot, Loader2, Pencil, Plus, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import { listNativeSubagents } from "@/lib/backend";
import type { NativeSubagent } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";
import { SubagentAiCreateDialog } from "./SubagentAiCreateDialog";
import { SubagentEditorDialog } from "./SubagentEditorDialog";

export function SubagentsSettingsTab() {
  const { t } = useTranslation("settings");
  const [items, setItems] = useState<NativeSubagent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [editing, setEditing] = useState<NativeSubagent | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [aiDialogOpen, setAiDialogOpen] = useState(false);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      setItems(await listNativeSubagents(useWorkspaceStore.getState().activeWorkspaceId));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const openCreate = () => {
    setEditing(null);
    setError(null);
    setMessage(null);
    setDialogOpen(true);
  };

  const openEdit = (item: NativeSubagent) => {
    setEditing(item);
    setError(null);
    setMessage(null);
    setDialogOpen(true);
  };

  const builtinItems = [
    { id: "general", name: "general", hintKey: "subagents.builtin.general" },
    { id: "explore", name: "explore", hintKey: "subagents.builtin.explore" },
  ] as const;

  return (
    <div className="space-y-6">
      {!dialogOpen && !aiDialogOpen && message ? (
        <SettingFeedbackCallout
          variant="success"
          message={message}
          onClose={() => setMessage(null)}
        />
      ) : null}
      {!dialogOpen && !aiDialogOpen && error ? (
        <SettingFeedbackCallout variant="error" message={error} onClose={() => setError(null)} />
      ) : null}

      <SettingCard
        icon={Bot}
        title={t("subagents.title")}
        description={t("subagents.description")}
        badge={`${items.length + builtinItems.length} 个子代理`}
        headerAction={
          <div className="flex items-center gap-1.5">
            <SubagentAiCreateDialog
              open={aiDialogOpen}
              onOpenChange={(next) => {
                setAiDialogOpen(next);
                if (next) {
                  setError(null);
                  setMessage(null);
                }
              }}
              trigger={
                <Button type="button" size="sm" variant="outline" className="h-7 gap-1 text-xs">
                  <Sparkles className="size-3.5" />
                  {t("subagents.actions.aiCreate")}
                </Button>
              }
              onCreated={(created) => {
                setItems((current) => [created, ...current]);
                setMessage(t("subagents.messages.created"));
              }}
            />
            <Button size="sm" onClick={openCreate} className="h-7 gap-1 text-xs">
              <Plus className="size-3.5" />
              {t("subagents.actions.new")}
            </Button>
          </div>
        }
      >
        <div className="space-y-4">
          <div className="space-y-2">
            <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/70">
              系统内置
            </p>
            <div className="grid gap-2 sm:grid-cols-2">
              {builtinItems.map((item) => (
                <div
                  key={item.id}
                  className="flex items-start gap-3 rounded-xl border border-border/70 bg-card/60 p-3 text-xs shadow-2xs"
                >
                  <div className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                    <Bot className="size-3.5" />
                  </div>
                  <div className="min-w-0 flex-1 space-y-0.5">
                    <div className="flex items-center gap-1.5">
                      <span className="font-semibold text-foreground">{item.name}</span>
                      <span className="rounded-md border border-border/50 bg-muted/50 px-1 py-0.1 font-mono text-[9px] text-muted-foreground">
                        Builtin
                      </span>
                    </div>
                    <p className="text-[11px] text-muted-foreground leading-relaxed">
                      {t(item.hintKey)}
                    </p>
                  </div>
                </div>
              ))}
            </div>
          </div>

          <div className="space-y-2 border-t border-border/50 pt-2">
            <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/70">
              自定义子智能体
            </p>
            {loading ? (
              <div className="flex h-28 items-center justify-center text-xs text-muted-foreground">
                <Loader2 className="mr-2 size-4 animate-spin text-primary" />
                {t("subagents.list.loading")}
              </div>
            ) : items.length === 0 ? (
              <div className="rounded-xl border border-dashed border-border/80 py-8 text-center">
                <p className="text-xs text-muted-foreground">{t("subagents.list.empty")}</p>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  可创建特定角色的子代理，并限制其可用工具或指派特定模型。
                </p>
              </div>
            ) : (
              <div className="grid gap-2.5">
                {items.map((item) => (
                  <div
                    key={item.id}
                    className="group flex flex-col justify-between gap-3 rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all hover:border-border hover:shadow-xs sm:flex-row sm:items-center"
                  >
                    <div className="flex min-w-0 items-start gap-3">
                      <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                        <Bot className="size-4" />
                      </div>
                      <div className="min-w-0 flex-1 space-y-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="truncate text-xs font-semibold tracking-tight text-foreground">
                            {item.name}
                          </span>
                          <span className="rounded-md border border-border/60 bg-muted/40 px-1.5 py-0.2 font-mono text-[10px] text-muted-foreground">
                            {item.scope === "workspaces"
                              ? t("subagents.fields.scopeWorkspacesCount", {
                                  count: (item.workspace_ids ?? []).length,
                                })
                              : t("subagents.fields.scopeAll")}
                          </span>
                          {item.source === "file" ? (
                            <span
                              className="rounded-md border border-border/50 bg-background px-1.5 py-0.2 font-mono text-[10px] text-muted-foreground"
                              title={item.path ?? ""}
                            >
                              {t("subagents.fields.fileProfile")}
                            </span>
                          ) : null}
                        </div>
                        <p className="line-clamp-2 text-[11px] leading-relaxed text-muted-foreground">
                          {item.description}
                        </p>
                      </div>
                    </div>

                    {item.source !== "file" ? (
                      <div className="flex shrink-0 items-center gap-1.5 self-end sm:self-center">
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-7 gap-1 text-xs"
                          onClick={() => openEdit(item)}
                        >
                          <Pencil className="size-3" />
                          {t("subagents.actions.edit")}
                        </Button>
                      </div>
                    ) : null}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </SettingCard>

      <SubagentEditorDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        item={editing}
        onCreated={(created) => {
          setItems((current) => [created, ...current]);
          setMessage(t("subagents.messages.created"));
        }}
        onUpdated={(updated) => {
          setItems((current) => current.map((item) => (item.id === updated.id ? updated : item)));
          setMessage(t("subagents.messages.updated"));
        }}
        onDeleted={(id) => {
          setItems((current) => current.filter((item) => item.id !== id));
          setEditing(null);
          setMessage(t("subagents.messages.deleted"));
        }}
      />
    </div>
  );
}
