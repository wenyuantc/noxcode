import { ArrowUp, Loader2, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { listGitFiles, listNativeGlobalSkills, stopNativeSession } from "@/lib/backend";
import { submitSessionPrompt } from "@/lib/sessionSubmission";
import { FALLBACK_THINKING_LEVELS } from "@/lib/modelCatalog";
import type { NativeSkill } from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { BranchPicker } from "./BranchPicker";
import { ChannelModelPicker } from "./ChannelModelPicker";
import { ContextCapacity } from "./ContextCapacity";
import { PermissionModePicker } from "./PermissionModePicker";
import { ThinkingLevelPicker } from "./ThinkingLevelPicker";
import { WorkspacePicker } from "./WorkspacePicker";

export function Composer({ compact = false }: { compact?: boolean }) {
  const { t } = useTranslation(["sessions", "layout"]);
  const draft = useUiStore((state) => state.composerDraft);
  const setDraft = useUiStore((state) => state.setComposerDraft);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channels = useChannelStore((state) => state.channels);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const activeModelId = useChannelStore((state) => state.activeModelId);
  const composerPlanMode = useUiStore((state) => state.composerPlanMode);
  const selectedSessionId = useSessionStore((state) => state.selectedSessionId);
  const live = useSessionStore((state) =>
    selectedSessionId ? state.liveBySession[selectedSessionId] : undefined,
  );
  const usage = useSessionStore((state) =>
    live ? state.usage[live.session_record_id] : undefined,
  );
  const turnState = useSessionStore((state) =>
    live ? state.turnState[live.session_record_id] : undefined,
  );
  const channel = channels.find((item) => item.id === channelId);
  const [model, setModel] = useState(activeModelId ?? "");
  const [effort, setEffort] = useState("medium");
  const [error, setError] = useState<string | null>(null);
  const [files, setFiles] = useState<string[]>([]);
  const [skills, setSkills] = useState<NativeSkill[]>([]);
  const [mentionOpen, setMentionOpen] = useState<"@" | "/" | null>(null);
  const [sending, setSending] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const selectedModel = channel?.models.find((item) => item.id === model);
  const efforts = selectedModel?.thinking_levels?.length
    ? selectedModel.thinking_levels
    : FALLBACK_THINKING_LEVELS;
  const resolvedEffort = efforts.includes(effort)
    ? effort
    : (efforts.find((level) => level === "medium") ?? efforts[0] ?? "medium");

  useEffect(() => {
    setModel(activeModelId ?? "");
  }, [activeModelId]);

  useEffect(() => {
    const last = draft.split(/\s/).pop() ?? "";
    if (last.startsWith("@") && workspaceId) {
      setMentionOpen("@");
      void listGitFiles(workspaceId, last.slice(1), 30)
        .then(setFiles)
        .catch(() => setFiles([]));
    } else if (last.startsWith("/")) {
      setMentionOpen("/");
      void listNativeGlobalSkills()
        .then((doc) =>
          setSkills(
            doc.skills.filter((skill) =>
              skill.name.toLowerCase().includes(last.slice(1).toLowerCase()),
            ),
          ),
        )
        .catch(() => setSkills([]));
    } else {
      setMentionOpen(null);
    }
  }, [draft, workspaceId]);

  const working = Boolean(live) && turnState !== "waiting_input" && turnState !== "ended";
  const sendBusy = sending || working;

  const send = async () => {
    const prompt = draft.trim();
    if (!prompt) {
      setError(t("sessions:emptyPrompt"));
      return;
    }
    if (!workspaceId) {
      setError(t("sessions:needWorkspace"));
      return;
    }
    if (!channelId || !model) {
      setError(t("sessions:needChannel"));
      return;
    }
    if (sendBusy) return;
    setError(null);
    setSending(true);
    try {
      await submitSessionPrompt({
        sessionId: selectedSessionId,
        workspaceId,
        channelId,
        prompt,
        model: model || null,
        reasoningEffort: resolvedEffort || null,
        planMode: composerPlanMode,
      });
      setDraft("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSending(false);
    }
  };

  const insertToken = (token: string) => {
    const parts = draft.split(/\s/);
    parts[parts.length - 1] = token;
    setDraft(`${parts.join(" ")} `);
    setMentionOpen(null);
    textareaRef.current?.focus();
  };

  return (
    <div className="mx-auto w-full max-w-3xl">
      {!compact ? (
        <div className="mb-2 flex items-center gap-2">
          <WorkspacePicker />
          <BranchPicker />
        </div>
      ) : null}
      <div className="rounded-xl border bg-card shadow-sm">
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              if (!sendBusy) void send();
            }
          }}
          placeholder={t("layout:composerPlaceholder")}
          className="min-h-24 w-full resize-none bg-transparent px-4 py-3 text-sm outline-none"
        />
        {mentionOpen === "@" && files.length > 0 ? (
          <div className="mx-3 mb-2 max-h-40 overflow-y-auto rounded-md border bg-popover text-sm">
            {files.map((file) => (
              <button
                key={file}
                type="button"
                className="block w-full truncate px-3 py-1.5 text-left hover:bg-accent"
                onClick={() => insertToken(`@${file}`)}
              >
                {file}
              </button>
            ))}
          </div>
        ) : null}
        {mentionOpen === "/" ? (
          <div className="mx-3 mb-2 max-h-40 overflow-y-auto rounded-md border bg-popover text-sm">
            {skills.length === 0 ? (
              <p className="px-3 py-2 text-muted-foreground">{t("sessions:noSkills")}</p>
            ) : (
              skills.map((skill) => (
                <button
                  key={skill.name}
                  type="button"
                  className="block w-full truncate px-3 py-1.5 text-left hover:bg-accent"
                  onClick={() => insertToken(`使用技能：${skill.name}`)}
                >
                  {skill.name}
                </button>
              ))
            )}
          </div>
        ) : null}
        <div className="flex flex-wrap items-center gap-2 border-t px-3 py-2 text-xs">
          <PermissionModePicker />
          <ChannelModelPicker />
          <ThinkingLevelPicker value={resolvedEffort} levels={efforts} onChange={setEffort} />
          <ContextCapacity usage={usage} />
          <span className="flex-1" />
          {working && live ? (
            <Button
              size="sm"
              variant="outline"
              onClick={() => void stopNativeSession(live.session_record_id)}
            >
              <Square className="size-3.5" />
              {t("sessions:stop")}
            </Button>
          ) : null}
          <Button size="sm" onClick={() => void send()} disabled={sendBusy}>
            {sendBusy ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <ArrowUp className="size-3.5" />
            )}
            {t("sessions:send")}
          </Button>
        </div>
      </div>
      {error ? <p className="mt-2 text-sm text-destructive">{error}</p> : null}
    </div>
  );
}
