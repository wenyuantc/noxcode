import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowUp,
  Bot,
  FileText,
  Laptop,
  Loader2,
  Server,
  Square,
  WandSparkles,
} from "lucide-react";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type DragEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { SkillCreateDialog } from "@/components/settings/SkillCreateDialog";
import { SubagentEditorDialog } from "@/components/settings/SubagentEditorDialog";
import {
  compactNativeSession,
  deleteComposerImages,
  enhancePrompt,
  expandNativeSlashCommand,
  forkNativeSession,
  listGitFiles,
  listNativePlugins,
  listNativeSkills,
  listNativeSlashCommands,
  listNativeSubagents,
  openNativePluginsDir,
  stageComposerImage,
  stageComposerImageFromPath,
  stopNativeSession,
  sendNativeInput,
  updateNativeSettings,
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
  addFilePill,
  clearTargetPill,
  hasPills,
  initialComposerPills,
  popLastPill,
  removeFilePill,
  removeTrailingTrigger,
  setTargetPill,
  type ComposerPillsState,
} from "@/lib/composerPills";
import { assembleComposerPrompt } from "@/lib/composerPromptAssembly";
import {
  builtinSlashCommands,
  filterComposerSlashItems,
  parseComposerTrigger,
  skillInvocationPrompt,
  subagentDelegationPrompt,
  type BuiltinSlashName,
  type ComposerSlashItem,
} from "@/lib/composerSlash";
import {
  isExpandingSlashIntent,
  isLocalSlashIntent,
  matchComposerEffort,
  matchComposerModel,
  type SlashIntent,
} from "@/lib/composerSlashActions";
import { applyComposerPlanMode, resolveComposerPlanMode } from "@/lib/planMode";
import { sessionUsageTotal } from "@/lib/sessionLines";
import { resolveSessionSelection } from "@/lib/sessionModel";
import { submitSessionPrompt } from "@/lib/sessionSubmission";
import { changeSessionConfiguration, finishIdleSession } from "@/lib/sessionConfiguration";
import { isNativePermissionMode, type NativeSubagent } from "@/lib/types";
import {
  composerThinkingEnabled,
  composerThinkingLevels,
  resolveComposerThinkingLevel,
} from "@/lib/modelCatalog";
import { cn } from "@/lib/utils";
import { ComposerSlashMenu } from "./ComposerSlashMenu";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { BranchPicker } from "./BranchPicker";
import { ComposerImageStrip } from "./ComposerImageStrip";
import { ComposerMentionMenu, ComposerMentionOption } from "./ComposerMentionMenu";
import { ComposerPillStrip } from "./ComposerPillStrip";
import { ComposerPlusMenu } from "./ComposerPlusMenu";
import { ContextCapacity } from "./ContextCapacity";
import { ModelEffortPicker } from "./ModelEffortPicker";
import { PermissionModePicker } from "./PermissionModePicker";
import { WorkspacePicker } from "./WorkspacePicker";
import { WorktreeToggle } from "./WorktreeToggle";
import { QueuedInputs } from "./QueuedInputs";

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

export function Composer({ compact = false }: { compact?: boolean }) {
  const { t } = useTranslation(["sessions", "layout", "git"]);
  const navigate = useNavigate();
  const draft = useUiStore((state) => state.composerDraft);
  const setDraft = useUiStore((state) => state.setComposerDraft);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const sessionWorkspace = workspaces.find((item) => item.id === workspaceId);
  const channels = useChannelStore((state) => state.channels);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const activeModelId = useChannelStore((state) => state.activeModelId);
  const defaultPlanMode = useUiStore((state) => state.composerPlanMode);
  const isolateWorktree = useUiStore((state) => state.composerIsolateWorktree);
  const effort = useUiStore((state) => state.composerThinkingLevel);
  const setEffort = useUiStore((state) => state.setComposerThinkingLevel);
  const native = useSettingsStore((state) => state.native);
  const ai = useSettingsStore((state) => state.ai);
  const setNative = useSettingsStore((state) => state.setNative);
  const selectedSessionId = useSessionStore((state) => state.selectedSessionId);
  const runtime = useSessionStore((state) =>
    selectedSessionId ? state.configurationBySession[selectedSessionId] : undefined,
  );
  const session = useWorkspaceStore((state) =>
    selectedSessionId ? state.sessions.find((item) => item.id === selectedSessionId) : undefined,
  );
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
  const lines = useSessionStore((state) =>
    selectedSessionId ? state.lines[selectedSessionId] : undefined,
  );
  const totalTokens = sessionUsageTotal(lines ?? []);
  const turnState = useSessionStore((state) =>
    live ? state.turnState[live.session_record_id] : undefined,
  );
  const pendingCount = useSessionStore((state) =>
    selectedSessionId ? (state.inputQueueBySession[selectedSessionId]?.items.length ?? 0) : 0,
  );
  const pendingConfiguration = useSessionStore((state) =>
    selectedSessionId ? state.pendingConfigurationBySession[selectedSessionId] : undefined,
  );
  const applyingConfig = Boolean(pendingConfiguration);
  const { channelId: effectiveChannelId, modelId: selectedModelId } = resolveSessionSelection({
    sessionId: selectedSessionId,
    runtime,
    session,
    fallbackChannelId: channelId,
    fallbackModelId: activeModelId,
  });
  const channel = channels.find((item) => item.id === effectiveChannelId);
  const model = selectedModelId ?? "";
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [skillDialogOpen, setSkillDialogOpen] = useState(false);
  const [subagentDialogOpen, setSubagentDialogOpen] = useState(false);
  const [files, setFiles] = useState<string[]>([]);
  const [mentionSubagents, setMentionSubagents] = useState<NativeSubagent[]>([]);
  const [slashItems, setSlashItems] = useState<ComposerSlashItem[]>([]);
  const [mentionOpen, setMentionOpen] = useState<"@" | "/" | "$" | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [pills, setPills] = useState<ComposerPillsState>(initialComposerPills());
  const [sending, setSending] = useState(false);
  const [enhancing, setEnhancing] = useState(false);
  const [attachments, setAttachments] = useState<ComposerImageItem[]>([]);
  const [dragging, setDragging] = useState(false);
  const composerRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const mentionListRef = useRef<HTMLDivElement>(null);
  const mentionListId = useId();
  const focusAfterInsertRef = useRef(false);
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
    runtime?.reasoning_effort ?? effort,
    selectedModel?.thinking_level,
  );
  const promptEnhancement = ai?.prompt_enhancement;
  const enhancementChannel = channels.find(
    (item) => item.enabled && item.id === promptEnhancement?.channel_id,
  );
  const enhancementReady = Boolean(
    promptEnhancement?.enabled &&
    enhancementChannel?.models.some((item) => item.id === promptEnhancement.model),
  );

  useEffect(() => {
    if (sendingRef.current) return;
    const stale = attachmentsRef.current;
    if (stale.length > 0) {
      void deleteComposerImages(stale.map((item) => item.path)).catch(() => undefined);
    }
    setAttachments([]);
    setPills(initialComposerPills());
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

  useLayoutEffect(() => {
    const input = textareaRef.current;
    if (!focusAfterInsertRef.current || !input) return;
    focusAfterInsertRef.current = false;
    input.focus();
    input.setSelectionRange(draft.length, draft.length);
  }, [draft]);

  useEffect(() => {
    if (trigger?.kind === "@" && workspaceId) {
      let cancelled = false;
      setMentionOpen("@");
      void Promise.all([
        listGitFiles(workspaceId, trigger.query, 30).catch(() => []),
        listNativeSubagents(workspaceId).catch(() => []),
      ]).then(([items, allAgents]) => {
        if (!cancelled) {
          setFiles(items);
          const q = trigger.query.toLowerCase().trim();
          const matchedAgents = allAgents.filter(
            (a) =>
              !q || a.name.toLowerCase().includes(q) || a.description?.toLowerCase().includes(q),
          );
          setMentionSubagents(matchedAgents);
        }
      });
      return () => {
        cancelled = true;
      };
    }
    if (trigger?.kind === "/" || trigger?.kind === "$") {
      setMentionOpen(trigger.kind);
      return;
    }
    setMentionOpen(null);
    setSlashItems([]);
    setMentionSubagents([]);
  }, [draft, trigger?.kind, trigger?.query, workspaceId]);

  useEffect(() => {
    if (mentionOpen !== "/" && mentionOpen !== "$") return;
    let cancelled = false;
    const labels = (name: BuiltinSlashName) => ({
      description: t(`slashBuiltin.${name}.description`),
      hint: t(`slashBuiltin.${name}.hint`),
    });
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
          argumentHint: command.argument_hint ?? undefined,
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
        subagentId: agent.id,
      }));
      setSlashItems(
        mentionOpen === "$" ? skillItems : [...commandItems, ...skillItems, ...agentItems],
      );
    });
    return () => {
      cancelled = true;
    };
  }, [mentionOpen, workspaceId, t]);

  const mentionQuery = trigger?.query ?? "";
  useEffect(() => {
    setMentionIndex(0);
  }, [mentionOpen, mentionQuery]);

  const atAgentItems = mentionSubagents.map((agent) => ({
    key: `subagent:${agent.id}`,
    kind: "subagent" as const,
    label: agent.name,
    description: agent.description,
    subagent: agent,
    token: agent.name,
  }));
  const atFileItems = files.map((file) => ({
    key: `file:${file}`,
    kind: "file" as const,
    label: file,
    file,
    token: file,
  }));
  const atItems = [...atAgentItems, ...atFileItems];

  const visibleSlashItems =
    mentionOpen === "/" || mentionOpen === "$"
      ? filterComposerSlashItems(slashItems, mentionQuery)
      : [];
  const pickerItems =
    mentionOpen === "@"
      ? atItems
      : visibleSlashItems.map((item) => ({ key: item.key, label: item.name, token: item.token }));
  const activeMentionIndex = clampMentionIndex(mentionIndex, pickerItems.length);

  const pickMentionItem = (item: {
    kind: "file" | "subagent" | "skill" | "command";
    file?: string;
    subagent?: { id: string; name: string; description?: string };
    skill?: ComposerSlashItem;
    command?: ComposerSlashItem;
  }) => {
    if (item.kind === "file" && item.file) {
      setPills((prev) => addFilePill(prev, item.file!));
    } else if (item.kind === "subagent" && item.subagent) {
      setPills((prev) =>
        setTargetPill(prev, {
          kind: "subagent",
          id: item.subagent!.id,
          name: item.subagent!.name,
          description: item.subagent!.description,
          token: item.subagent!.name,
        }),
      );
    } else if (item.kind === "skill" && item.skill) {
      setPills((prev) =>
        setTargetPill(prev, {
          kind: "skill",
          name: item.skill!.name,
          description: item.skill!.description,
          sourceLabel: item.skill!.sourceLabel,
          token: item.skill!.token,
        }),
      );
    } else if (item.kind === "command" && item.command) {
      setPills((prev) =>
        setTargetPill(prev, {
          kind: "command",
          name: item.command!.name,
          description: item.command!.description,
          argumentHint: item.command!.argumentHint,
          token: item.command!.token,
        }),
      );
    }
    setDraft(removeTrailingTrigger(draft));
    setMentionOpen(null);
    focusAfterInsertRef.current = true;
  };

  useEffect(() => {
    const list = mentionListRef.current;
    if (!list) return;
    const active = list.querySelector("[data-mention-active='true']");
    if (active instanceof HTMLElement) active.scrollIntoView({ block: "nearest" });
  }, [activeMentionIndex, pickerItems.length]);

  const working =
    Boolean(live) && (pendingCount > 0 || (turnState !== "waiting_input" && turnState !== "ended"));
  const sendBusy = sending || applyingConfig;

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

  const fail = (message: string) => {
    setInfo(null);
    setError(message);
  };

  const note = (message: string) => {
    setError(null);
    setInfo(message);
  };

  const writePlanMode = (enabled: boolean) => {
    applyComposerPlanMode({
      enabled,
      sessionId: selectedSessionId,
      setDefault: useUiStore.getState().setComposerPlanMode,
      setSession: (id, next) => useSessionStore.getState().onPlanMode(id, next),
    });
  };

  const applyPermissionMode = async (mode: "default" | "edit" | "build" | "plan" | "yolo") => {
    const persisted = isNativePermissionMode(runtime?.permission_mode ?? native?.permission_mode)
      ? (runtime?.permission_mode ?? native?.permission_mode)
      : "default";
    if (selectedSessionId) await finishIdleSession(selectedSessionId);
    if (mode === "plan") {
      await changeSessionConfiguration(selectedSessionId, { plan_mode: true });
      writePlanMode(true);
      return;
    }
    if (!runtime && mode !== persisted) {
      setNative(await updateNativeSettings({ permission_mode: mode }));
    }
    await changeSessionConfiguration(selectedSessionId, {
      permission_mode: mode,
      plan_mode: false,
    });
    writePlanMode(false);
  };

  const expandIntentPrompt = async (intent: SlashIntent, fallback: string) => {
    if (intent.type === "expand") return intent.prompt;
    if (intent.type === "skill") return skillInvocationPrompt(intent.name, intent.args);
    if (intent.type === "custom") {
      try {
        const expanded = await expandNativeSlashCommand(workspaceId, intent.name, intent.args);
        return expanded.prompt;
      } catch {
        return fallback;
      }
    }
    return fallback;
  };

  const runLocalIntent = async (intent: SlashIntent): Promise<boolean> => {
    switch (intent.type) {
      case "new-session":
        useSessionStore.getState().selectSession(null);
        setDraft("");
        setHelpOpen(false);
        setError(null);
        setInfo(null);
        return true;
      case "mode-help":
        fail(t("sessions:slashModeOptions"));
        return true;
      case "set-mode":
        try {
          await applyPermissionMode(intent.mode);
          note(
            t("sessions:slashModeSet", {
              mode: t(`sessions:permission.${intent.mode}.title`),
            }),
          );
          setDraft("");
        } catch (reason) {
          fail(String(reason));
        }
        return true;
      case "open-models":
        setDraft("");
        void navigate("/settings/channels");
        return true;
      case "set-model": {
        const matched = matchComposerModel(channels, intent.query);
        if (!matched) {
          fail(t("sessions:slashModelMissing", { query: intent.query }));
          return true;
        }
        try {
          if (live && turnState !== "waiting_input" && turnState !== "ended") {
            note(t("sessions:modelPending"));
          }
          const result = await changeSessionConfiguration(selectedSessionId, {
            ai_channel_id: matched.channelId,
            model: matched.modelId,
          });
          if (result?.compacted) note(t("sessions:modelCompacted"));
          else note(t("sessions:slashModelSet", { model: matched.modelId }));
          setDraft("");
        } catch (reason) {
          fail(`${t("sessions:modelSwitchFailed")}: ${String(reason)}`);
        }
        return true;
      }
      case "effort-help":
        fail(
          efforts.length === 0
            ? t("sessions:slashEffortUnavailable")
            : t("sessions:slashEffortOptions", { levels: efforts.join(", ") }),
        );
        return true;
      case "set-effort": {
        if (efforts.length === 0) {
          fail(t("sessions:slashEffortUnavailable"));
          return true;
        }
        const matched = matchComposerEffort(efforts, intent.level);
        if (!matched) {
          fail(t("sessions:slashEffortOptions", { levels: efforts.join(", ") }));
          return true;
        }
        try {
          await changeSessionConfiguration(selectedSessionId, { reasoning_effort: matched });
          setEffort(matched);
          note(t("sessions:slashEffortSet", { level: matched }));
          setDraft("");
        } catch (reason) {
          fail(String(reason));
        }
        return true;
      }
      case "plan":
        if (intent.task) return false;
        try {
          await applyPermissionMode("plan");
          note(t("sessions:slashPlanOn"));
          setDraft("");
        } catch (reason) {
          fail(String(reason));
        }
        return true;
      case "navigate":
        setDraft("");
        void navigate(intent.path);
        return true;
      case "plugins":
        try {
          const view = await listNativePlugins(workspaceId);
          const enabled = view.plugins.filter((item) => item.enabled).length;
          note(
            view.plugins.length === 0
              ? t("sessions:slashPluginsNone")
              : t("sessions:slashPluginsStatus", { enabled, total: view.plugins.length }),
          );
          await openNativePluginsDir().catch(() => undefined);
          setDraft("");
        } catch (reason) {
          fail(String(reason));
        }
        return true;
      case "diff":
        useUiStore.getState().openGitPreview(null);
        setDraft("");
        return true;
      case "context":
        if (!usage || usage.limit_tokens <= 0) {
          fail(t("sessions:slashContextEmpty"));
        } else {
          setContextOpen(true);
          setDraft("");
        }
        return true;
      case "help":
        setHelpOpen(true);
        setDraft("");
        return true;
      case "open-dialog":
        setDraft("");
        if (intent.dialog === "skill") setSkillDialogOpen(true);
        else setSubagentDialogOpen(true);
        return true;
      case "skill-help":
        fail(t("sessions:slashSkillNeedName"));
        return true;
      default:
        return false;
    }
  };

  const send = async () => {
    if (sendingRef.current || sending || applyingConfig) return;
    const assembled = assembleComposerPrompt(pills, draft);
    const prompt = assembled.prompt.trim();
    const intent = assembled.intent;
    if (!prompt && attachments.length === 0) {
      fail(t("sessions:emptyPrompt"));
      return;
    }
    if (!workspaceId) {
      fail(t("sessions:needWorkspace"));
      return;
    }
    if (prompt && isLocalSlashIntent(intent)) {
      await runLocalIntent(intent);
      setPills(initialComposerPills());
      return;
    }
    if (intent.type === "fork") {
      if (!selectedSessionId) {
        fail(t("sessions:forkNeedsSession"));
        return;
      }
      setError(null);
      setSending(true);
      try {
        await finishIdleSession(selectedSessionId);
        const forked = await forkNativeSession(selectedSessionId, intent.checkpointId);
        await useWorkspaceStore.getState().refreshSessions();
        await useSessionStore.getState().loadHistory(forked);
        setDraft("");
        setPills(initialComposerPills());
      } catch (err) {
        fail(err instanceof Error ? err.message : String(err));
      } finally {
        setSending(false);
      }
      return;
    }
    if (intent.type === "compact") {
      if (!live) {
        fail(t("sessions:compactNeedsLiveSession"));
        return;
      }
      setError(null);
      setSending(true);
      try {
        const accepted = await compactNativeSession(live.session_record_id, intent.instructions);
        if (!accepted) fail(t("sessions:compactNeedsLiveSession"));
        else {
          setDraft("");
          setPills(initialComposerPills());
        }
      } catch (err) {
        fail(err instanceof Error ? err.message : String(err));
      } finally {
        setSending(false);
      }
      return;
    }

    let nextPrompt = prompt;
    if (intent.type === "plan" && intent.task) {
      try {
        await applyPermissionMode("plan");
      } catch (reason) {
        fail(String(reason));
        return;
      }
      nextPrompt = intent.task;
    } else if (intent.type === "expand") {
      nextPrompt = intent.prompt;
      if (!working && !pills.target) {
        setDraft(nextPrompt);
        return;
      }
    } else if (isExpandingSlashIntent(intent)) {
      nextPrompt = await expandIntentPrompt(intent, prompt);
    }

    if (working && live) {
      if (attachments.length) {
        fail("运行中追加指令暂不支持附件");
        return;
      }
      setSending(true);
      sendingRef.current = true;
      setError(null);
      setInfo(null);
      try {
        useSessionStore
          .getState()
          .onInputQueue(await sendNativeInput(live.session_record_id, nextPrompt));
        setDraft("");
        setPills(initialComposerPills());
      } catch (reason) {
        fail(String(reason));
      } finally {
        setSending(false);
        sendingRef.current = false;
      }
      return;
    }
    if (sendBusy) return;
    if (!effectiveChannelId || !model) {
      fail(t("sessions:needChannel"));
      return;
    }
    setError(null);
    setInfo(null);
    setSending(true);
    sendingRef.current = true;
    const thinkingOn = composerThinkingEnabled(selectedModel);
    if (thinkingOn) setEffort(resolvedEffort);
    try {
      const imagePaths = attachments.map((item) => item.path);
      const started = await submitSessionPrompt({
        sessionId: selectedSessionId,
        workspaceId,
        channelId: effectiveChannelId,
        prompt: nextPrompt,
        model: model || null,
        reasoningEffort: thinkingOn ? resolvedEffort || null : null,
        planMode: composerPlanMode || intent.type === "plan",
        permissionMode: runtime?.permission_mode,
        imagePaths,
        isolateWorktree: selectedSessionId ? false : isolateWorktree,
      });
      if (started) {
        useSessionStore.getState().onStarted(started);
        if (useSessionStore.getState().selectedSessionId === selectedSessionId)
          useSessionStore.getState().selectSession(started.session_record_id);
        await useSessionStore.getState().ensureHistory(started.session_record_id);
      }
      attachmentsRef.current = [];
      setDraft("");
      setPills(initialComposerPills());
      setAttachments([]);
    } catch (err) {
      fail(err instanceof Error ? err.message : String(err));
    } finally {
      sendingRef.current = false;
      setSending(false);
    }
  };

  const insertTrigger = (trigger: ComposerTriggerChar) => {
    focusAfterInsertRef.current = true;
    setDraft(appendComposerTrigger(draft, trigger));
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

  const togglePlanMode = async () => {
    if (working || sending) return;
    try {
      await changeSessionConfiguration(selectedSessionId, { plan_mode: !composerPlanMode });
      applyComposerPlanMode({
        enabled: !composerPlanMode,
        sessionId: selectedSessionId,
        setDefault: useUiStore.getState().setComposerPlanMode,
        setSession: (id, enabled) => useSessionStore.getState().onPlanMode(id, enabled),
      });
    } catch (reason) {
      setError(String(reason));
    }
  };

  const enhanceDraft = async () => {
    if (enhancing || sending) return;
    const prompt = draft.trim();
    if (!prompt) {
      fail(t("sessions:promptEnhancementEmpty"));
      return;
    }
    if (!enhancementReady) {
      void navigate("/settings/ai?prompt-enhancement=missing-model");
      return;
    }
    setError(null);
    setInfo(null);
    setEnhancing(true);
    try {
      const enhanced = await enhancePrompt(prompt, workspaceId, selectedSessionId);
      setDraft(enhanced);
      setInfo(t("sessions:promptEnhanced"));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setEnhancing(false);
    }
  };

  return (
    <div className="mx-auto w-full max-w-3xl">
      {!compact ? (
        <div className="relative z-20 mb-2 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <WorkspacePicker />
          <div className="inline-flex h-7 items-center gap-1.5 rounded-lg border border-border/70 bg-background/80 px-2 text-xs font-medium text-foreground/80 shadow-2xs">
            {sessionWorkspace?.ssh_config_id ? (
              <>
                <Server className="size-3.5 text-muted-foreground" />
                <span>{t("git:remoteEnv", { defaultValue: "远程" })}</span>
              </>
            ) : (
              <>
                <Laptop className="size-3.5 text-muted-foreground" />
                <span>{t("git:localEnv", { defaultValue: "本地" })}</span>
              </>
            )}
          </div>
          <BranchPicker />
          <WorktreeToggle />
        </div>
      ) : null}
      {helpOpen ? (
        <div className="mb-2 rounded-xl border border-border/70 bg-card/95 p-3 text-xs shadow-sm">
          <div className="flex items-start justify-between gap-2">
            <div>
              <p className="font-medium text-foreground">{t("sessions:slashHelpTitle")}</p>
              <p className="mt-0.5 text-muted-foreground">{t("sessions:slashHelpHint")}</p>
            </div>
            <button
              type="button"
              className="text-muted-foreground hover:text-foreground"
              onClick={() => setHelpOpen(false)}
            >
              {t("sessions:planAskClose")}
            </button>
          </div>
          <ul className="mt-2 max-h-48 space-y-1 overflow-y-auto">
            {builtinSlashCommands((name) => ({
              description: t(`slashBuiltin.${name}.description`),
              hint: t(`slashBuiltin.${name}.hint`),
            })).map((item) => (
              <li key={item.key} className="flex gap-2">
                <span className="shrink-0 font-mono text-foreground">/{item.name}</span>
                <span className="min-w-0 truncate text-muted-foreground">
                  {item.argumentHint ? `${item.argumentHint} · ` : ""}
                  {item.description}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {selectedSessionId ? (
        <QueuedInputs key={selectedSessionId} sessionId={selectedSessionId} />
      ) : null}
      <div
        ref={composerRef}
        className={cn(
          "relative rounded-2xl border border-border/70 bg-card/95 shadow-sm transition-all duration-150 focus-within:border-ring/60 focus-within:ring-2 focus-within:ring-ring/10",
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
        <ComposerPillStrip
          pills={pills}
          onRemoveTarget={() => setPills((prev) => clearTargetPill(prev))}
          onRemoveFile={(file) => setPills((prev) => removeFilePill(prev, file))}
        />
        {promptEnhancement?.enabled ? (
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="absolute right-2 top-2 z-10 size-7 rounded-lg text-muted-foreground hover:text-foreground"
            title={t("sessions:promptEnhancement")}
            aria-label={t("sessions:promptEnhancement")}
            disabled={enhancing || sending}
            onClick={() => void enhanceDraft()}
          >
            {enhancing ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <WandSparkles className="size-3.5" />
            )}
          </Button>
        ) : null}
        <textarea
          ref={textareaRef}
          role="combobox"
          aria-label={t("layout:composerPlaceholder")}
          aria-autocomplete="list"
          aria-haspopup="listbox"
          aria-expanded={mentionOpen !== null}
          aria-controls={mentionOpen ? mentionListId : undefined}
          aria-activedescendant={
            mentionOpen && pickerItems.length > 0
              ? `${mentionListId}-${activeMentionIndex}`
              : undefined
          }
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            const action = resolveComposerMentionKey({
              key: event.key,
              shiftKey: event.shiftKey,
              isComposing: event.nativeEvent.isComposing || event.keyCode === 229,
              mentionVisible: mentionOpen !== null,
              itemCount: pickerItems.length,
              activeIndex: activeMentionIndex,
            });
            if (action.type === "move") {
              event.preventDefault();
              setMentionIndex(action.nextIndex);
              return;
            }
            if (action.type === "confirm") {
              event.preventDefault();
              if (mentionOpen === "@") {
                const picked = atItems[activeMentionIndex];
                if (picked) {
                  if (picked.kind === "subagent") {
                    pickMentionItem({ kind: "subagent", subagent: picked.subagent });
                  } else {
                    pickMentionItem({ kind: "file", file: picked.file });
                  }
                }
              } else {
                const picked = visibleSlashItems[activeMentionIndex];
                if (picked) {
                  if (picked.group === "skills") {
                    pickMentionItem({ kind: "skill", skill: picked });
                  } else if (picked.group === "subagents") {
                    pickMentionItem({
                      kind: "subagent",
                      subagent: {
                        id: picked.subagentId ?? picked.name,
                        name: picked.name,
                        description: picked.description,
                      },
                    });
                  } else {
                    pickMentionItem({ kind: "command", command: picked });
                  }
                }
              }
              return;
            }
            if (action.type === "dismiss") {
              event.preventDefault();
              setMentionOpen(null);
              return;
            }
            if (
              event.key === "Backspace" &&
              !event.nativeEvent.isComposing &&
              mentionOpen === null &&
              textareaRef.current?.selectionStart === 0 &&
              textareaRef.current?.selectionEnd === 0 &&
              hasPills(pills)
            ) {
              event.preventDefault();
              setPills((prev) => popLastPill(prev));
              return;
            }
            if (action.type === "togglePlanMode") {
              event.preventDefault();
              void togglePlanMode();
              return;
            }
            if (action.type === "send") {
              event.preventDefault();
              if (!sendBusy) void send();
            }
          }}
          placeholder={t("layout:composerPlaceholder")}
          className="min-h-24 w-full resize-none bg-transparent px-4 py-3 pr-12 text-sm leading-relaxed outline-none placeholder:text-muted-foreground/60"
        />
        <div className="flex items-center justify-between gap-2 border-t border-border/50 px-3 py-2 text-xs">
          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
            <ComposerPlusMenu
              onAddAttachment={() => void pickAttachments()}
              onInsertTrigger={insertTrigger}
            />
            <PermissionModePicker disabled={working || sending} onError={setError} />
          </div>
          <div className="flex shrink-0 items-center gap-1.5 self-end">
            <ModelEffortPicker
              onError={fail}
              onInfo={note}
              disabled={working || sending || applyingConfig}
            />
            <ContextCapacity
              usage={usage}
              totalTokens={totalTokens}
              open={contextOpen}
              onOpenChange={setContextOpen}
            />
            {working && live ? (
              <Button
                size="icon"
                variant="outline"
                className="size-8 rounded-lg border-destructive/40 text-destructive hover:bg-destructive/10"
                title={t("sessions:stop")}
                aria-label={t("sessions:stop")}
                onClick={() =>
                  void stopNativeSession(live.session_record_id).catch((reason) =>
                    setError(String(reason)),
                  )
                }
              >
                <Square className="size-3.5" />
              </Button>
            ) : null}
            <Button
              size="icon"
              className="size-8 cursor-pointer rounded-lg shadow-2xs transition-all hover:opacity-95 active:scale-[0.98]"
              title={working ? t("sessions:queuedInput.add") : t("sessions:send")}
              aria-label={working ? t("sessions:queuedInput.add") : t("sessions:send")}
              onClick={() => void send()}
              disabled={sendBusy || (!draft.trim() && attachments.length === 0 && !hasPills(pills))}
            >
              {sendBusy ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <ArrowUp className="size-3.5" />
              )}
            </Button>
          </div>
        </div>
      </div>
      <ComposerMentionMenu
        open={mentionOpen !== null}
        anchorRef={composerRef}
        inputRef={textareaRef}
        listRef={mentionListRef}
        id={mentionListId}
        label={t(
          mentionOpen === "@" ? "atContext" : mentionOpen === "$" ? "slashSkills" : "slashTitle",
        )}
        onDismiss={() => setMentionOpen(null)}
      >
        {mentionOpen === "@" ? (
          atItems.length > 0 ? (
            <div className="space-y-0.5">
              {atAgentItems.length > 0 ? (
                <div role="group" aria-label={t("slashSubagents", { defaultValue: "子智能体" })}>
                  <div className="flex items-center gap-1.5 px-2.5 py-1 text-[11px] font-semibold tracking-wider text-muted-foreground/70 uppercase select-none">
                    {t("slashSubagents", { defaultValue: "子智能体" })}
                  </div>
                  {atAgentItems.map((item, index) => (
                    <ComposerMentionOption
                      key={item.key}
                      id={`${mentionListId}-${index}`}
                      active={index === activeMentionIndex}
                      icon={Bot}
                      iconClassName="text-purple-500"
                      label={item.label}
                      description={item.description}
                      onMouseEnter={() => setMentionIndex(index)}
                      onClick={() => pickMentionItem({ kind: "subagent", subagent: item.subagent })}
                    />
                  ))}
                </div>
              ) : null}
              {atFileItems.length > 0 ? (
                <div
                  role="group"
                  aria-label={t("files", { defaultValue: "文件" })}
                  className={cn(atAgentItems.length > 0 && "border-t border-border/50 pt-1")}
                >
                  <div className="flex items-center gap-1.5 px-2.5 py-1 text-[11px] font-semibold tracking-wider text-muted-foreground/70 uppercase select-none">
                    {t("files", { defaultValue: "工作区文件" })}
                  </div>
                  {atFileItems.map((item, index) => {
                    const overallIndex = atAgentItems.length + index;
                    return (
                      <ComposerMentionOption
                        key={item.key}
                        id={`${mentionListId}-${overallIndex}`}
                        active={overallIndex === activeMentionIndex}
                        icon={FileText}
                        iconClassName="text-muted-foreground"
                        label={item.label}
                        onMouseEnter={() => setMentionIndex(overallIndex)}
                        onClick={() => pickMentionItem({ kind: "file", file: item.file })}
                      />
                    );
                  })}
                </div>
              ) : null}
            </div>
          ) : (
            <p role="status" className="px-2.5 py-2 text-xs text-muted-foreground">
              {t("noFiles")}
            </p>
          )
        ) : (
          <ComposerSlashMenu
            items={visibleSlashItems}
            activeIndex={activeMentionIndex}
            listId={mentionListId}
            emptyLabel={t(mentionOpen === "$" ? "slashEmptySkills" : "slashEmpty")}
            onHover={setMentionIndex}
            onPick={(item) => {
              if (item.group === "skills") {
                pickMentionItem({ kind: "skill", skill: item });
              } else if (item.group === "subagents") {
                pickMentionItem({
                  kind: "subagent",
                  subagent: {
                    id: item.subagentId ?? item.name,
                    name: item.name,
                    description: item.description,
                  },
                });
              } else {
                pickMentionItem({ kind: "command", command: item });
              }
            }}
          />
        )}
      </ComposerMentionMenu>
      {error ? <p className="mt-2 text-sm text-destructive">{error}</p> : null}
      {info ? <p className="mt-2 text-sm text-muted-foreground">{info}</p> : null}
      <SkillCreateDialog
        open={skillDialogOpen}
        onOpenChange={setSkillDialogOpen}
        onCreated={() => undefined}
      />
      <SubagentEditorDialog open={subagentDialogOpen} onOpenChange={setSubagentDialogOpen} />
    </div>
  );
}
