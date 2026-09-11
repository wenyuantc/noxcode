import { useEffect, useMemo, useState, type ReactElement } from "react";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  createNativeSubagent,
  generateNativeSubagent,
  listAiChannels,
  listWorkspaces,
} from "@/lib/backend";
import {
  composerThinkingEnabled,
  composerThinkingLevels,
  resolveComposerThinkingLevel,
} from "@/lib/modelCatalog";
import {
  canSubmitSubagentForm,
  EMPTY_SUBAGENT_FORM,
  formFromGeneratedSubagent,
  subagentPayloadFrom,
} from "@/lib/subagentForm";
import type { AiChannel, NativeSubagent, Workspace } from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { ChannelModelPicker } from "@/components/session/ChannelModelPicker";
import { ThinkingLevelPicker } from "@/components/session/ThinkingLevelPicker";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { SubagentFormFields } from "./SubagentFormFields";

export function SubagentAiCreateDialog({
  open,
  onOpenChange,
  onCreated,
  trigger,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated?: (created: NativeSubagent) => void;
  trigger?: ReactElement;
}) {
  const { t } = useTranslation("settings");
  const channels = useChannelStore((state) => state.channels);
  const storeChannelId = useChannelStore((state) => state.activeChannelId);
  const storeModelId = useChannelStore((state) => state.activeModelId);
  const [step, setStep] = useState<"prompt" | "review">("prompt");
  const [description, setDescription] = useState("");
  const [channelId, setChannelId] = useState<string | null>(null);
  const [modelId, setModelId] = useState<string | null>(null);
  const [effort, setEffort] = useState("");
  const [formChannels, setFormChannels] = useState<AiChannel[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [form, setForm] = useState(EMPTY_SUBAGENT_FORM);
  const [busy, setBusy] = useState<"generate" | "create" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const selectedChannel = channels.find((item) => item.id === channelId);
  const selectedModel = selectedChannel?.models.find((item) => item.id === modelId);
  const thinkingOn = composerThinkingEnabled(selectedModel);
  const efforts = composerThinkingLevels(selectedModel);
  const resolvedEffort = resolveComposerThinkingLevel(
    efforts,
    effort,
    selectedModel?.thinking_level,
  );
  const enabledChannels = useMemo(
    () => formChannels.filter((channel) => channel.enabled),
    [formChannels],
  );
  const canGenerate =
    description.trim().length > 0 && Boolean(channelId?.trim()) && Boolean(modelId?.trim());

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setStep("prompt");
    setDescription("");
    setForm(EMPTY_SUBAGENT_FORM);
    setError(null);
    setBusy(null);
    setChannelId(useChannelStore.getState().activeChannelId);
    setModelId(useChannelStore.getState().activeModelId);
    setEffort(useUiStore.getState().composerThinkingLevel ?? "");
    if (useChannelStore.getState().channels.length === 0) {
      void useChannelStore.getState().load();
    }
    void Promise.all([listAiChannels(), listWorkspaces()])
      .then(([channelItems, workspaceItems]) => {
        if (cancelled) return;
        setFormChannels(channelItems);
        setWorkspaces(workspaceItems);
      })
      .catch((reason) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  useEffect(() => {
    if (!open || channelId || !storeChannelId) return;
    setChannelId(storeChannelId);
    setModelId(storeModelId);
  }, [open, channelId, storeChannelId, storeModelId]);

  const close = () => {
    if (busy !== null) return;
    onOpenChange(false);
    setError(null);
  };

  const handleGenerate = async () => {
    if (!canGenerate || !channelId || !modelId) {
      setError(t("subagents.messages.needChannel"));
      return;
    }
    setBusy("generate");
    setError(null);
    try {
      const draft = await generateNativeSubagent({
        description,
        channel_id: channelId,
        model: modelId,
        reasoning_effort: thinkingOn ? resolvedEffort || null : null,
        workspace_id: useWorkspaceStore.getState().activeWorkspaceId,
      });
      setForm(formFromGeneratedSubagent(draft));
      setStep("review");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  };

  const handleCreate = async () => {
    setBusy("create");
    setError(null);
    try {
      const created = await createNativeSubagent(subagentPayloadFrom(form));
      onCreated?.(created);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  };

  const formLocked = busy !== null;
  const isReview = step === "review";

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (next) {
          onOpenChange(true);
          return;
        }
        close();
      }}
    >
      {trigger ? <DialogTrigger render={trigger} /> : null}
      <DialogContent
        className="flex max-h-[90vh] w-full flex-col gap-0 overflow-hidden sm:max-w-2xl"
        showCloseButton={!formLocked}
      >
        <DialogHeader className="shrink-0 pb-3">
          <DialogTitle>
            {isReview ? t("subagents.dialogs.aiReviewTitle") : t("subagents.dialogs.aiCreateTitle")}
          </DialogTitle>
          <DialogDescription>
            {isReview
              ? t("subagents.dialogs.aiReviewDescription")
              : t("subagents.dialogs.aiCreateDescription")}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto pr-1">
          {isReview ? (
            <SubagentFormFields
              form={form}
              enabledChannels={enabledChannels}
              workspaces={workspaces}
              busy={formLocked}
              onPatch={(updates) => setForm((current) => ({ ...current, ...updates }))}
            />
          ) : (
            <div className="space-y-3">
              <Textarea
                value={description}
                disabled={formLocked}
                onChange={(event) => setDescription(event.target.value)}
                placeholder={t("subagents.dialogs.aiPromptPlaceholder")}
                rows={6}
              />
              <div className="flex flex-wrap items-center gap-1.5">
                <ChannelModelPicker
                  disabled={formLocked}
                  selection={{ channelId, modelId }}
                  onSelectionChange={(nextChannel, nextModel) => {
                    setChannelId(nextChannel);
                    setModelId(nextModel);
                  }}
                  onError={setError}
                />
                {thinkingOn && efforts.length > 0 ? (
                  <ThinkingLevelPicker
                    value={resolvedEffort}
                    levels={efforts}
                    disabled={formLocked}
                    onChange={setEffort}
                  />
                ) : null}
              </div>
            </div>
          )}
          {error ? <p className="mt-3 text-sm text-destructive">{error}</p> : null}
        </div>
        <DialogFooter className="mt-4 shrink-0">
          {isReview ? (
            <Button
              variant="outline"
              className="sm:mr-auto"
              onClick={() => {
                setError(null);
                setStep("prompt");
              }}
              disabled={formLocked}
            >
              {t("subagents.actions.back")}
            </Button>
          ) : null}
          <Button variant="outline" onClick={close} disabled={formLocked}>
            {t("subagents.actions.cancel")}
          </Button>
          {isReview ? (
            <Button
              onClick={() => void handleCreate()}
              disabled={formLocked || !canSubmitSubagentForm(form)}
            >
              {busy === "create" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("subagents.actions.create")}
            </Button>
          ) : (
            <Button onClick={() => void handleGenerate()} disabled={formLocked || !canGenerate}>
              {busy === "generate" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {busy === "generate"
                ? t("subagents.messages.generating")
                : t("subagents.actions.generate")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
