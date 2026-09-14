import { useEffect, useState } from "react";
import { ClipboardPaste, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { createNativeSubagent, listAiChannels, listWorkspaces } from "@/lib/backend";
import {
  parseSubagentImportJson,
  serializeSubagentDraftsJson,
  toImportedSubagentPayload,
  type SubagentImportDraft,
  type SubagentImportWarning,
} from "@/lib/subagentJson";
import type { AiChannel, NativeSubagent, Workspace } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";

function warningMessage(
  t: (key: string, options?: Record<string, string | number>) => string,
  name: string,
  warning: SubagentImportWarning,
): string {
  return t(`subagents.warnings.${warning.code}`, {
    name,
    channelId: warning.channelId ?? "",
    model: warning.model ?? "",
    dropped: warning.dropped ?? 0,
  });
}

export function SubagentJsonImportDialog({
  open,
  onOpenChange,
  onImported,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImported: (created: NativeSubagent[], warnings: string[]) => void;
}) {
  const { t } = useTranslation("settings");
  const [text, setText] = useState("");
  const [drafts, setDrafts] = useState<SubagentImportDraft[] | null>(null);
  const [channels, setChannels] = useState<AiChannel[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [contextReady, setContextReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const applyText = (next: string, reportError = false) => {
    setText(next);
    setError(null);
    try {
      setDrafts(parseSubagentImportJson(next));
    } catch (err) {
      setDrafts(null);
      if (reportError && next.trim().length > 0) {
        setError(err instanceof Error ? err.message : String(err));
      }
    }
  };

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setText("");
    setDrafts(null);
    setError(null);
    setBusy(false);
    setContextReady(false);
    setChannels([]);
    setWorkspaces([]);
    void Promise.all([listAiChannels(), listWorkspaces()])
      .then(([channelItems, workspaceItems]) => {
        if (cancelled) return;
        setChannels(channelItems);
        setWorkspaces(workspaceItems);
        setContextReady(true);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setContextReady(false);
        setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const close = () => {
    if (busy) return;
    onOpenChange(false);
    setError(null);
  };

  const handlePasteClipboard = async () => {
    if (busy) return;
    setError(null);
    try {
      const next = await navigator.clipboard.readText();
      applyText(next, true);
    } catch {
      setError(t("subagents.messages.clipboardReadFailed"));
    }
  };

  const handleCreate = async () => {
    if (busy || !contextReady) return;
    setBusy(true);
    setError(null);
    const created: NativeSubagent[] = [];
    const warningTexts: string[] = [];
    let parsed: SubagentImportDraft[] = [];
    try {
      parsed = parseSubagentImportJson(text);
      setDrafts(parsed);
      const [channelItems, workspaceItems] = await Promise.all([
        listAiChannels(),
        listWorkspaces(),
      ]);
      setChannels(channelItems);
      setWorkspaces(workspaceItems);
      setContextReady(true);
      const ctx = { channels: channelItems, workspaces: workspaceItems };
      for (const [index, draft] of parsed.entries()) {
        const { payload, warnings } = toImportedSubagentPayload(draft, ctx);
        created.push(await createNativeSubagent(payload));
        warningTexts.push(...warnings.map((warning) => warningMessage(t, draft.name, warning)));
        const remaining = parsed.slice(index + 1);
        setDrafts(remaining.length > 0 ? remaining : null);
        setText(remaining.length > 0 ? serializeSubagentDraftsJson(remaining) : "");
      }
      onImported(created, warningTexts);
      onOpenChange(false);
    } catch (err) {
      if (created.length > 0) onImported(created, warningTexts);
      const remaining = parsed.slice(created.length);
      if (remaining.length > 0) {
        setDrafts(remaining);
        setText(serializeSubagentDraftsJson(remaining));
      }
      const detail = err instanceof Error ? err.message : String(err);
      setError(
        created.length > 0
          ? t("subagents.messages.importPartial", { count: created.length, detail })
          : detail,
      );
    } finally {
      setBusy(false);
    }
  };

  const canImport = !busy && contextReady && drafts !== null && drafts.length > 0;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent
        className="flex max-h-[90vh] w-full flex-col gap-0 overflow-hidden sm:max-w-2xl"
        showCloseButton={!busy}
      >
        <DialogHeader className="shrink-0 pb-3">
          <DialogTitle>{t("subagents.dialogs.importTitle")}</DialogTitle>
          <DialogDescription>{t("subagents.dialogs.importDescription")}</DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
          <Textarea
            value={text}
            disabled={busy}
            onChange={(event) => applyText(event.target.value)}
            placeholder={t("subagents.dialogs.importPlaceholder")}
            rows={10}
            className="min-h-40 font-mono text-xs"
          />
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-7 gap-1 text-xs"
            onClick={() => void handlePasteClipboard()}
            disabled={busy}
          >
            <ClipboardPaste className="size-3.5" />
            {t("subagents.actions.pasteClipboard")}
          </Button>
          {drafts && drafts.length > 0 ? (
            <div className="space-y-2 rounded-md border border-border p-3">
              {drafts.map((draft, index) => {
                const warnings = contextReady
                  ? toImportedSubagentPayload(draft, { channels, workspaces }).warnings
                  : [];
                return (
                  <div key={`${draft.name}-${index}`} className="space-y-1">
                    <p className="truncate text-sm font-medium text-foreground">{draft.name}</p>
                    <p className="line-clamp-2 text-xs text-muted-foreground">
                      {draft.description}
                    </p>
                    {warnings.map((warning) => (
                      <p
                        key={`${draft.name}-${warning.code}`}
                        className="text-xs text-amber-700 dark:text-amber-400"
                      >
                        {warningMessage(t, draft.name, warning)}
                      </p>
                    ))}
                  </div>
                );
              })}
            </div>
          ) : null}
          {error ? <p className="text-sm text-destructive">{error}</p> : null}
        </div>
        <DialogFooter className="mt-4 shrink-0">
          <Button variant="outline" onClick={close} disabled={busy}>
            {t("subagents.actions.cancel")}
          </Button>
          <Button onClick={() => void handleCreate()} disabled={!canImport}>
            {busy ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
            {busy ? t("subagents.messages.importing") : t("subagents.actions.importJson")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
