import { useEffect, useMemo, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { Loader2, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  createNativeSubagent,
  deleteNativeSubagent,
  listAiChannels,
  listWorkspaces,
  updateNativeSubagent,
} from "@/lib/backend";
import {
  canSubmitSubagentForm,
  EMPTY_SUBAGENT_FORM,
  subagentPayloadFrom,
  toSubagentForm,
} from "@/lib/subagentForm";
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
import { SubagentFormFields } from "./SubagentFormFields";

export function SubagentEditorDialog({
  open,
  onOpenChange,
  item = null,
  onCreated,
  onUpdated,
  onDeleted,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  item?: NativeSubagent | null;
  onCreated?: (created: NativeSubagent) => void;
  onUpdated?: (updated: NativeSubagent) => void;
  onDeleted?: (id: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [channels, setChannels] = useState<AiChannel[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [form, setForm] = useState(EMPTY_SUBAGENT_FORM);
  const [saving, setSaving] = useState<"save" | "delete" | "create" | null>(null);
  const [deleteConfirming, setDeleteConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isCreate = item == null;
  const enabledChannels = useMemo(() => channels.filter((channel) => channel.enabled), [channels]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void Promise.all([listAiChannels(), listWorkspaces()])
      .then(([channelItems, workspaceItems]) => {
        if (cancelled) return;
        setChannels(channelItems);
        setWorkspaces(workspaceItems);
        setForm(item ? toSubagentForm(item, workspaceItems) : EMPTY_SUBAGENT_FORM);
        setError(null);
      })
      .catch((reason) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [open, item]);

  const close = () => {
    if (saving !== null || deleteConfirming) return;
    onOpenChange(false);
    setError(null);
  };

  const handleCreate = async () => {
    setSaving("create");
    setError(null);
    try {
      const created = await createNativeSubagent(subagentPayloadFrom(form));
      onCreated?.(created);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleSave = async () => {
    if (!item) return;
    setSaving("save");
    setError(null);
    try {
      const updated = await updateNativeSubagent(item.id, subagentPayloadFrom(form));
      onUpdated?.(updated);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleDelete = async () => {
    if (!item || saving !== null || deleteConfirming) return;
    setDeleteConfirming(true);
    setError(null);
    try {
      const confirmed = await confirm(t("subagents.dialogs.deleteConfirm", { name: item.name }), {
        title: t("subagents.dialogs.deleteTitle"),
        kind: "warning",
      });
      if (!confirmed) return;
      setSaving("delete");
      await deleteNativeSubagent(item.id);
      onDeleted?.(item.id);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeleteConfirming(false);
      setSaving(null);
    }
  };

  const busy = saving !== null;
  const formLocked = busy || deleteConfirming;
  const canSubmit = canSubmitSubagentForm(form);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent
        className="flex max-h-[90vh] w-full flex-col gap-0 overflow-hidden sm:max-w-2xl"
        showCloseButton={!formLocked}
      >
        <DialogHeader className="shrink-0 pb-3">
          <DialogTitle>
            {isCreate ? t("subagents.dialogs.createTitle") : t("subagents.dialogs.editTitle")}
          </DialogTitle>
          <DialogDescription>{t("subagents.description")}</DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto pr-1">
          <SubagentFormFields
            form={form}
            enabledChannels={enabledChannels}
            workspaces={workspaces}
            busy={formLocked}
            onPatch={(updates) => setForm((current) => ({ ...current, ...updates }))}
          />
          {error ? <p className="mt-3 text-sm text-destructive">{error}</p> : null}
        </div>
        <DialogFooter className="mt-4 shrink-0">
          {!isCreate ? (
            <Button
              variant="destructive"
              className="sm:mr-auto"
              onClick={() => void handleDelete()}
              disabled={formLocked}
            >
              {saving === "delete" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              <Trash2 className="mr-1 h-4 w-4" />
              {t("subagents.actions.delete")}
            </Button>
          ) : null}
          <Button variant="outline" onClick={close} disabled={formLocked}>
            {t("subagents.actions.cancel")}
          </Button>
          {isCreate ? (
            <Button onClick={() => void handleCreate()} disabled={formLocked || !canSubmit}>
              {saving === "create" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("subagents.actions.create")}
            </Button>
          ) : (
            <Button onClick={() => void handleSave()} disabled={formLocked || !canSubmit}>
              {saving === "save" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("subagents.actions.save")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
