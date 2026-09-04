import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUp, Loader2, Square } from "lucide-react";
import { useEffect, useRef, useState, type ClipboardEvent, type DragEvent } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  compactNativeSession,
  deleteComposerImages,
  expandNativeSlashCommand,
  forkNativeSession,
  listGitFiles,
  listNativeSkills,
  listNativeSlashCommands,
  listNativeSubagents,
  stageComposerImage,
  stageComposerImageFromPath,
  stopNativeSession,
} from "@/lib/backend";
import {
  appendComposerTrigger,
  collectFilesFromDataTransfer,
  fileNameFromPath,
  filterComposerImageFiles,
  filterComposerImagePaths,
  mergeComposerImageItems,
  removeComposerImagesByIds,
  selectedComposerImageIds,
  toggleComposerImageSelected,
  type ComposerImageFileLike,
  type ComposerImageItem,
  type ComposerImageSkip,
  type ComposerTriggerChar,
} from "@/lib/composerImages";
import { clampMentionIndex, resolveComposerMentionKey } from "@/lib/composerMention";
import {
  builtinSlashCommands,
  filterComposerSlashItems,
  isBuiltinSlashName,
  parseComposerTrigger,
  parseLeadingSlash,
  parseSkillInvocation,
  skillInvocationPrompt,
  subagentDelegationPrompt,
  type ComposerSlashItem,
} from "@/lib/composerSlash";
import { applyComposerPlanMode, resolveComposerPlanMode } from "@/lib/planMode";
import { submitSessionPrompt } from "@/lib/sessionSubmission";
import { composerThinkingLevels, resolveComposerThinkingLevel } from "@/lib/modelCatalog";
import { cn } from "@/lib/utils";
import { ComposerSlashMenu } from "./ComposerSlashMenu";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { BranchPicker } from "./BranchPicker";
import { ChannelModelPicker } from "./ChannelModelPicker";
import { ComposerImageStrip } from "./ComposerImageStrip";
import { ComposerPlusMenu } from "./ComposerPlusMenu";
import { ContextCapacity } from "./ContextCapacity";
import { PermissionModePicker } from "./PermissionModePicker";
import { ThinkingLevelPicker } from "./ThinkingLevelPicker";
import { WorkspacePicker } from "./WorkspacePicker";

const IMAGE_DIALOG_FILTERS = [
  { name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp"] },
];

async function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result ?? "");
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.onerror = () => reject(reader.error ?? new Error("read failed"));
    reader.readAsDataURL(file);
  });
}

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
  const defaultPlanMode = useUiStore((state) => state.composerPlanMode);
  const effort = useUiStore((state) => state.composerThinkingLevel);
  const setEffort = useUiStore((state) => state.setComposerThinkingLevel);
  const selectedSessionId = useSessionStore((state) => state.selectedSessionId);
  const planModeBySession = useSessionStore((state) => state.planModeBySession);
  const composerPlanMode = resolveComposerPlanMode(
    selectedSessionId,
    planModeBySession,
    defaultPlanMode,
  );
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
  const [error, setError] = useState<string | null>(null);
  const [files, setFiles] = useState<string[]>([]);
  const [slashItems, setSlashItems] = useState<ComposerSlashItem[]>([]);
  const [mentionOpen, setMentionOpen] = useState<"@" | "/" | "$" | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [sending, setSending] = useState(false);
  const [attachments, setAttachments] = useState<ComposerImageItem[]>([]);
  const [dragging, setDragging] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const mentionListRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const attachmentsRef = useRef<ComposerImageItem[]>([]);
  const dragDepthRef = useRef(0);
  const sendingRef = useRef(false);
  const applyDroppedPathsRef = useRef<(paths: string[]) => void>(() => undefined);
  attachmentsRef.current = attachments;

  const selectedModel = channel?.models.find((item) => item.id === model);
  const efforts = composerThinkingLevels(selectedModel);
  const resolvedEffort = resolveComposerThinkingLevel(
    efforts,
    effort,
    selectedModel?.thinking_level,
  );

  useEffect(() => {
    setModel(activeModelId ?? "");
  }, [activeModelId]);

  useEffect(() => {
    if (sendingRef.current) return;
    const stale = attachmentsRef.current;
    if (stale.length > 0) {
      void deleteComposerImages(stale.map((item) => item.path)).catch(() => undefined);
    }
    setAttachments([]);
  }, [selectedSessionId]);

  useEffect(() => {
    return () => {
      if (sendingRef.current) return;
      const leftover = attachmentsRef.current;
      if (leftover.length > 0) {
        void deleteComposerImages(leftover.map((item) => item.path)).catch(() => undefined);
      }
    };
  }, []);

  const trigger = parseComposerTrigger(draft);

  useEffect(() => {
    if (trigger?.kind === "@" && workspaceId) {
      setMentionOpen("@");
      void listGitFiles(workspaceId, trigger.query, 30)
        .then(setFiles)
        .catch(() => setFiles([]));
      return;
    }
    if (trigger?.kind === "/" || trigger?.kind === "$") {
      setMentionOpen(trigger.kind);
      return;
    }
    setMentionOpen(null);
    setSlashItems([]);
  }, [draft, trigger?.kind, trigger?.query, workspaceId]);

  useEffect(() => {
    if (mentionOpen !== "/" && mentionOpen !== "$") return;
    let cancelled = false;
    const labels = {
      init: "AGENTS.md",
      fork: "checkpoint",
      compact: "context",
    };
    void Promise.all([
      listNativeSlashCommands(workspaceId).catch(() => []),
      listNativeSkills(workspaceId).catch(() => null),
      listNativeSubagents(workspaceId).catch(() => []),
    ]).then(([commands, skillsView, subagents]) => {
      if (cancelled) return;
      const disabled = new Set(
        (skillsView?.disabled_paths ?? []).map((path) => path.replace(/\\/g, "/")),
      );
      const commandItems: ComposerSlashItem[] = [
        ...builtinSlashCommands(labels),
        ...commands.map((command) => ({
          group: "commands" as const,
          key: `command:${command.path}`,
          name: command.name,
          description: command.description,
          sourceLabel: command.plugin ?? command.source,
          token: `/${command.name}`,
        })),
      ];
      const skillItems: ComposerSlashItem[] = (skillsView?.skills ?? [])
        .filter((skill) => !disabled.has(skill.skill_md_path.replace(/\\/g, "/")))
        .map((skill) => ({
          group: "skills" as const,
          key: `skill:${skill.skill_md_path}`,
          name: skill.name,
          description: skill.description,
          sourceLabel: skill.plugin ?? skill.source,
          token: `$${skill.name}`,
        }));
      const agentItems: ComposerSlashItem[] = subagents.map((agent) => ({
        group: "subagents" as const,
        key: `subagent:${agent.id}`,
        name: agent.name,
        description: agent.description,
        token: subagentDelegationPrompt(agent.name, agent.id),
      }));
      setSlashItems(
        mentionOpen === "$" ? skillItems : [...commandItems, ...skillItems, ...agentItems],
      );
    });
    return () => {
      cancelled = true;
    };
  }, [mentionOpen, workspaceId]);

  const mentionQuery = trigger?.query ?? "";
  useEffect(() => {
    setMentionIndex(0);
  }, [mentionOpen, mentionQuery]);

  const mentionItems =
    mentionOpen === "@" ? files.map((file) => ({ key: file, label: file, token: `@${file}` })) : [];
  const visibleSlashItems =
    mentionOpen === "/" || mentionOpen === "$"
      ? filterComposerSlashItems(slashItems, mentionQuery)
      : [];
  const pickerItems =
    mentionOpen === "@"
      ? mentionItems
      : visibleSlashItems.map((item) => ({ key: item.key, label: item.name, token: item.token }));
  const activeMentionIndex = clampMentionIndex(mentionIndex, pickerItems.length);

  useEffect(() => {
    const list = mentionListRef.current;
    if (!list) return;
    const active = list.querySelector("[data-mention-active='true']");
    if (active instanceof HTMLElement) active.scrollIntoView({ block: "nearest" });
  }, [activeMentionIndex, pickerItems.length]);

  const working = Boolean(live) && turnState !== "waiting_input" && turnState !== "ended";
  const sendBusy = sending || working;

  const skipMessage = (skip: ComposerImageSkip) => {
    if (skip.reason === "size") return t("sessions:imageTooLarge", { name: skip.name });
    if (skip.reason === "limit") return t("sessions:imageLimit");
    return t("sessions:imageTypeUnsupported", { name: skip.name });
  };

  const applyIncomingAttachments = (incoming: ComposerImageItem[]) => {
    const merged = mergeComposerImageItems(attachmentsRef.current, incoming);
    const kept = new Set(merged.items.map((item) => item.path));
    const unused = incoming.filter((item) => !kept.has(item.path)).map((item) => item.path);
    if (unused.length > 0) {
      void deleteComposerImages(unused).catch(() => undefined);
    }
    if (merged.skipped.length > 0) setError(skipMessage(merged.skipped[0]));
    else if (incoming.length > 0) setError(null);
    setAttachments(merged.items);
  };

  const addImageFiles = async (files: ComposerImageFileLike[]) => {
    const { accepted, skipped } = filterComposerImageFiles(files);
    if (skipped.length > 0) setError(skipMessage(skipped[0]));
    const incoming: ComposerImageItem[] = [];
    for (const file of accepted) {
      if (!(file instanceof File)) continue;
      try {
        const dataBase64 = await readFileAsBase64(file);
        const path = await stageComposerImage(file.name || "image.png", dataBase64);
        incoming.push({
          id: crypto.randomUUID(),
          name: file.name || "image.png",
          path,
          previewUrl: convertFileSrc(path),
          selected: false,
        });
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    }
    if (incoming.length > 0) applyIncomingAttachments(incoming);
  };

  const addImagePaths = async (paths: string[]) => {
    const incoming: ComposerImageItem[] = [];
    for (const source of paths) {
      try {
        const path = await stageComposerImageFromPath(source);
        incoming.push({
          id: crypto.randomUUID(),
          name: fileNameFromPath(source),
          path,
          previewUrl: convertFileSrc(path),
          selected: false,
        });
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    }
    if (incoming.length > 0) applyIncomingAttachments(incoming);
  };

  const applyDroppedPaths = (paths: string[]) => {
    const { accepted, skipped } = filterComposerImagePaths(paths);
    if (skipped.length > 0) setError(skipMessage(skipped[0]));
    if (accepted.length > 0) void addImagePaths(accepted);
  };
  applyDroppedPathsRef.current = applyDroppedPaths;

  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) => {
        if (cancelled) return undefined;
        return getCurrentWebview().onDragDropEvent((event) => {
          const payload = event.payload;
          if (payload.type === "enter" || payload.type === "over") {
            setDragging(true);
            return;
          }
          if (payload.type === "leave") {
            setDragging(false);
            return;
          }
          if (payload.type === "drop") {
            setDragging(false);
            applyDroppedPathsRef.current(payload.paths);
          }
        });
      })
      .then((stop) => {
        if (!stop) return;
        if (cancelled) stop();
        else unlisten = stop;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const pickAttachments = async () => {
    try {
      if (isTauri()) {
        const selected = await open({
          multiple: true,
          filters: IMAGE_DIALOG_FILTERS,
        });
        const paths = selected == null ? [] : Array.isArray(selected) ? selected : [selected];
        if (paths.length > 0) await addImagePaths(paths);
        return;
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      return;
    }
    fileInputRef.current?.click();
  };

  const deleteStaged = (items: ComposerImageItem[]) => {
    if (items.length > 0) {
      void deleteComposerImages(items.map((item) => item.path)).catch(() => undefined);
    }
  };

  const send = async () => {
    const prompt = draft.trim();
    if (!prompt && attachments.length === 0) {
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
    let nextPrompt = prompt;
    const skillCall = parseSkillInvocation(prompt);
    if (skillCall) {
      nextPrompt = skillInvocationPrompt(skillCall.name, skillCall.args);
    } else {
      const slash = parseLeadingSlash(prompt);
      if (slash && !isBuiltinSlashName(slash.name)) {
        try {
          const expanded = await expandNativeSlashCommand(workspaceId, slash.name, slash.args);
          nextPrompt = expanded.prompt;
        } catch {
          // 未注册的自定义命令按普通文本发送。
        }
      }
    }
    setError(null);
    setSending(true);
    sendingRef.current = true;
    setEffort(resolvedEffort);
    try {
      const imagePaths = attachments.map((item) => item.path);
      await submitSessionPrompt({
        sessionId: selectedSessionId,
        workspaceId,
        channelId,
        prompt: nextPrompt,
        model: model || null,
        reasoningEffort: resolvedEffort || null,
        planMode: composerPlanMode,
        imagePaths,
      });
      attachmentsRef.current = [];
      setDraft("");
      setAttachments([]);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      sendingRef.current = false;
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

  const insertTrigger = (trigger: ComposerTriggerChar) => {
    setDraft(appendComposerTrigger(draft, trigger));
    textareaRef.current?.focus();
  };

  const handlePaste = (event: ClipboardEvent<HTMLDivElement>) => {
    const files = collectFilesFromDataTransfer(event.clipboardData);
    const { accepted } = filterComposerImageFiles(files);
    if (accepted.length === 0) return;
    const text = event.clipboardData.getData("text/plain");
    if (!text) event.preventDefault();
    void addImageFiles(accepted);
  };

  const dataTransferHasFiles = (event: DragEvent<HTMLDivElement>) =>
    Array.from(event.dataTransfer.types).includes("Files");

  const togglePlanMode = () => {
    applyComposerPlanMode({
      enabled: !composerPlanMode,
      sessionId: selectedSessionId,
      setDefault: useUiStore.getState().setComposerPlanMode,
      setSession: (id, enabled) => useSessionStore.getState().onPlanMode(id, enabled),
    });
  };

  return (
    <div className="mx-auto w-full max-w-3xl">
      {!compact ? (
        <div className="mb-2 flex items-center gap-2">
          <WorkspacePicker />
          <BranchPicker />
        </div>
      ) : null}
      <div
        className={cn(
          "rounded-2xl border border-border/70 bg-card/95 shadow-sm transition-all duration-150 focus-within:border-ring/60 focus-within:ring-2 focus-within:ring-ring/10",
          dragging && "border-ring ring-2 ring-ring/20",
        )}
        onPaste={handlePaste}
        onDragEnter={(event) => {
          if (!dataTransferHasFiles(event)) return;
          event.preventDefault();
          dragDepthRef.current += 1;
          setDragging(true);
        }}
        onDragOver={(event) => {
          if (!dataTransferHasFiles(event)) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "copy";
        }}
        onDragLeave={(event) => {
          if (!dataTransferHasFiles(event)) return;
          event.preventDefault();
          dragDepthRef.current -= 1;
          if (dragDepthRef.current <= 0) {
            dragDepthRef.current = 0;
            setDragging(false);
          }
        }}
        onDrop={(event) => {
          if (!dataTransferHasFiles(event)) return;
          event.preventDefault();
          dragDepthRef.current = 0;
          setDragging(false);
          void addImageFiles(collectFilesFromDataTransfer(event.dataTransfer));
        }}
      >
        <input
          ref={fileInputRef}
          type="file"
          accept="image/png,image/jpeg,image/gif,image/webp"
          multiple
          className="hidden"
          onChange={(event) => {
            const files = Array.from(event.target.files ?? []);
            event.target.value = "";
            if (files.length > 0) void addImageFiles(files);
          }}
        />
        <ComposerImageStrip
          images={attachments}
          onToggle={(id) => setAttachments((items) => toggleComposerImageSelected(items, id))}
          onRemove={(id) => {
            const next = removeComposerImagesByIds(attachments, [id]);
            deleteStaged(attachments.filter((item) => item.id === id));
            setAttachments(next);
          }}
          onRemoveSelected={() => {
            const ids = selectedComposerImageIds(attachments);
            deleteStaged(attachments.filter((item) => ids.includes(item.id)));
            setAttachments(removeComposerImagesByIds(attachments, ids));
          }}
        />
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            const action = resolveComposerMentionKey({
              key: event.key,
              shiftKey: event.shiftKey,
              isComposing: event.nativeEvent.isComposing || event.keyCode === 229,
              mentionVisible: pickerItems.length > 0,
              itemCount: pickerItems.length,
              activeIndex: activeMentionIndex,
            });
            if (action.type === "move") {
              event.preventDefault();
              setMentionIndex(action.nextIndex);
              return;
            }
            if (action.type === "confirm") {
              const item = pickerItems[activeMentionIndex];
              if (!item) return;
              event.preventDefault();
              insertToken(item.token);
              return;
            }
            if (action.type === "dismiss") {
              event.preventDefault();
              setMentionOpen(null);
              return;
            }
            if (action.type === "togglePlanMode") {
              event.preventDefault();
              togglePlanMode();
              return;
            }
            if (action.type === "send") {
              event.preventDefault();
              if (!sendBusy) void send();
            }
          }}
          placeholder={t("layout:composerPlaceholder")}
          className="min-h-24 w-full resize-none bg-transparent px-4 py-3 text-sm leading-relaxed outline-none placeholder:text-muted-foreground/60"
        />
        {mentionOpen === "@" && mentionItems.length > 0 ? (
          <div
            ref={mentionListRef}
            className="mx-3 mb-2 max-h-40 overflow-y-auto rounded-xl border border-border/60 bg-popover p-1 text-sm shadow-md"
          >
            {mentionItems.map((item, index) => (
              <button
                key={item.key}
                type="button"
                data-mention-active={index === activeMentionIndex ? "true" : undefined}
                className={cn(
                  "block w-full truncate rounded-lg px-2.5 py-1.5 text-left text-xs font-medium transition-colors",
                  index === activeMentionIndex
                    ? "bg-accent text-accent-foreground"
                    : "hover:bg-accent/70",
                )}
                onMouseEnter={() => setMentionIndex(index)}
                onClick={() => insertToken(item.token)}
              >
                {item.label}
              </button>
            ))}
          </div>
        ) : mentionOpen === "/" || mentionOpen === "$" ? (
          <ComposerSlashMenu
            items={visibleSlashItems}
            activeIndex={activeMentionIndex}
            listRef={mentionListRef}
            onHover={setMentionIndex}
            onPick={(item) => insertToken(item.token)}
          />
        ) : null}
        <div className="flex flex-wrap items-center gap-1.5 border-t border-border/50 px-3 py-2 text-xs">
          <ComposerPlusMenu
            onAddAttachment={() => void pickAttachments()}
            onInsertTrigger={insertTrigger}
          />
          <PermissionModePicker />
          <ChannelModelPicker />
          <ThinkingLevelPicker value={resolvedEffort} levels={efforts} onChange={setEffort} />
          <ContextCapacity usage={usage} />
          <span className="flex-1" />
          {working && live ? (
            <Button
              size="sm"
              variant="outline"
              className="h-7 gap-1.5 rounded-lg border-destructive/40 px-2.5 text-xs text-destructive hover:bg-destructive/10"
              onClick={() => void stopNativeSession(live.session_record_id)}
            >
              <Square className="size-3.5" />
              {t("sessions:stop")}
            </Button>
          ) : null}
          <Button
            size="sm"
            className="h-7 cursor-pointer gap-1.5 rounded-lg px-3 text-xs font-medium shadow-2xs transition-all hover:opacity-95 active:scale-[0.98]"
            onClick={() => void send()}
            disabled={sendBusy}
          >
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
