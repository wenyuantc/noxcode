import { ArrowUp, Loader2, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  compactNativeSession,
  forkNativeSession,
  listGitFiles,
  listNativeGlobalSkills,
  stopNativeSession,
} from "@/lib/backend";
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

/** `/init [补充要求]` 展开成的提示词：Agent 摸底仓库后生成或补充 AGENTS.md。 */
export function buildInitPrompt(extra?: string): string {
  const lines = [
    "请为当前仓库生成或补充 AGENTS.md（若已有 AGENTS.md / CLAUDE.md 则在其基础上补充，不要重复已有内容）。",
    "先用 Glob / Read / Grep 摸底：项目结构与模块职责、构建 / 测试 / lint 命令、编码约定、关键架构约束、常见陷阱。",
    "输出要求：简洁、面向编程 Agent、只写能从仓库验证的事实；每条命令都注明来源文件；不超过 150 行。",
    "完成后用 Write 写入仓库根目录的 AGENTS.md，并在回复里列出你新增或修改的段落。",
  ];
  if (extra?.trim()) lines.push(`补充要求：${extra.trim()}`);
  return lines.join("\n");
}

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
    selectedSessionId ? state.usage[selectedSessionId] : undefined,
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
    // `/init`：让 Agent 分析仓库并生成 / 补充 AGENTS.md（作为普通提示词提交）。
    const initMatch = /^\/init(?:\s+([\s\S]*))?$/i.exec(prompt);
    if (initMatch) {
      setDraft(buildInitPrompt(initMatch[1]));
      return;
    }
    // `/fork [checkpoint_id]`：复制当前会话上下文到新会话（可选先回滚到检查点）。
    const forkMatch = /^\/fork(?:\s+(\S+))?$/i.exec(prompt);
    if (forkMatch) {
      if (!selectedSessionId) {
        setError(t("sessions:forkNeedsSession"));
        return;
      }
      setError(null);
      setSending(true);
      try {
        const forked = await forkNativeSession(selectedSessionId, forkMatch[1]);
        await useWorkspaceStore.getState().refreshSessions();
        useSessionStore.getState().selectSession(forked);
        setDraft("");
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setSending(false);
      }
      return;
    }
    // `/compact [指令]`：对运行中的会话请求上下文压缩，不算一条用户输入。
    const compactMatch = /^\/compact(?:\s+([\s\S]*))?$/i.exec(prompt);
    if (compactMatch) {
      if (!live) {
        setError(t("sessions:compactNeedsLiveSession"));
        return;
      }
      setError(null);
      setSending(true);
      try {
        const accepted = await compactNativeSession(live.session_record_id, compactMatch[1]);
        if (!accepted) setError(t("sessions:compactNeedsLiveSession"));
        else setDraft("");
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setSending(false);
      }
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
