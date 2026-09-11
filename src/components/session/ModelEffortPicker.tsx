import { Check, ChevronDown, Clock, Settings2 } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { resolveSessionSelection, type SessionModelSelection } from "@/lib/sessionModel";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { changeSessionConfiguration } from "@/lib/sessionConfiguration";
import {
  composerThinkingEnabled,
  composerThinkingLevels,
  resolveComposerThinkingLevel,
} from "@/lib/modelCatalog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EffortIcon } from "./ThinkingLevelPicker";

export function ModelEffortPicker({
  onError,
  onInfo,
  disabled = false,
  selection,
  reasoningEffort,
  onSelectionChange,
  onReasoningEffortChange,
  className,
}: {
  onError?: (error: string) => void;
  onInfo?: (message: string) => void;
  disabled?: boolean;
  selection?: SessionModelSelection;
  reasoningEffort?: string;
  onSelectionChange?: (channelId: string, modelId: string) => void;
  onReasoningEffortChange?: (effort: string) => void;
  className?: string;
}) {
  const navigate = useNavigate();
  const { t, i18n } = useTranslation("sessions");
  const storeChannels = useChannelStore((state) => state.channels);
  const channels = storeChannels.length > 0 ? storeChannels : useChannelStore.getState().channels;
  const activeChannelId = useChannelStore((state) => state.activeChannelId);
  const activeModelId = useChannelStore((state) => state.activeModelId);
  const sessionId = useSessionStore((state) => state.selectedSessionId);
  const runtime = useSessionStore((state) =>
    sessionId ? state.configurationBySession[sessionId] : undefined,
  );
  const pending = useSessionStore((state) =>
    sessionId ? state.pendingConfigurationBySession[sessionId] : undefined,
  );
  const session = useWorkspaceStore((state) =>
    sessionId ? state.sessions.find((item) => item.id === sessionId) : undefined,
  );
  const resolved = resolveSessionSelection({
    sessionId,
    runtime,
    session,
    fallbackChannelId: activeChannelId,
    fallbackModelId: activeModelId,
  });
  const controlled = Boolean(onSelectionChange);
  const displayChannelId = selection?.channelId ?? pending?.ai_channel_id ?? resolved.channelId;
  const displayModelId = selection?.modelId ?? pending?.model ?? resolved.modelId;
  const showPending = !controlled && Boolean(pending);

  const enabledChannels = channels.filter((channel) => channel.enabled);
  const currentChannel = channels.find((item) => item.id === displayChannelId);
  const currentModel = currentChannel?.models.find((item) => item.id === displayModelId);

  // Thinking level
  const thinkingOn = composerThinkingEnabled(currentModel);
  const efforts = composerThinkingLevels(currentModel);
  const storeEffort = useUiStore((state) => state.composerThinkingLevel);
  const currentEffort = reasoningEffort ?? runtime?.reasoning_effort ?? storeEffort ?? "high";
  const resolvedEffort = resolveComposerThinkingLevel(
    efforts,
    currentEffort,
    currentModel?.thinking_level,
  );

  const effortTitleOf = (level: string) =>
    t(`effortLevels.${level}.title`, { defaultValue: level });
  const effortDescOf = (level: string) => {
    const key = `effortLevels.${level}.description`;
    return i18n.exists(key, { ns: "sessions" }) ? t(key) : "";
  };

  const modelLabel = displayModelId
    ? currentChannel?.name
      ? `${currentChannel.name}/${displayModelId}`
      : displayModelId
    : t("needChannel");

  const selectModel = async (channelId: string, modelId: string) => {
    if (disabled) return;
    if (channelId === displayChannelId && modelId === displayModelId) return;
    if (onSelectionChange) {
      onSelectionChange(channelId, modelId);
      return;
    }
    try {
      const result = await changeSessionConfiguration(sessionId, {
        ai_channel_id: channelId,
        model: modelId,
      });
      if (result?.compacted) onInfo?.(t("modelCompacted"));
    } catch (reason) {
      onError?.(`${t("modelSwitchFailed")}: ${String(reason)}`);
    }
  };

  const selectEffort = async (effort: string) => {
    if (disabled) return;
    if (onReasoningEffortChange) {
      onReasoningEffortChange(effort);
      return;
    }
    try {
      if (sessionId) {
        await changeSessionConfiguration(sessionId, { reasoning_effort: effort });
        onInfo?.(t("slashEffortSet", { level: effortTitleOf(effort) }));
      } else {
        useUiStore.getState().setComposerThinkingLevel(effort);
      }
    } catch (reason) {
      onError?.(String(reason));
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled}
        title={showPending ? t("modelPending") : undefined}
        className={cn(
          "inline-flex h-7 min-w-0 max-w-64 cursor-pointer items-center gap-1.5 rounded-lg border border-border/70 bg-background/80 px-2 text-xs font-medium text-foreground/90 shadow-2xs transition-all duration-150 outline-none hover:bg-muted/40 disabled:opacity-60",
          className,
        )}
      >
        {showPending ? (
          <Clock className="size-3 shrink-0 text-amber-500" />
        ) : thinkingOn && resolvedEffort ? (
          <EffortIcon level={resolvedEffort} className="size-3 shrink-0 text-muted-foreground" />
        ) : null}
        <span className="truncate">{modelLabel}</span>
        {thinkingOn && resolvedEffort ? (
          <span className="shrink-0 text-muted-foreground font-normal">
            · {effortTitleOf(resolvedEffort)}
          </span>
        ) : null}
        <ChevronDown className="size-3 shrink-0 text-muted-foreground/70" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuLabel className="text-[11px] font-semibold text-muted-foreground/70 uppercase">
          {t("modelSelectorTitle", { defaultValue: "模型与思考深度" })}
        </DropdownMenuLabel>
        {enabledChannels.length === 0 ? (
          <DropdownMenuItem disabled>{t("noEnabledChannels")}</DropdownMenuItem>
        ) : (
          enabledChannels.map((c) => (
            <DropdownMenuSub key={c.id}>
              <DropdownMenuSubTrigger className="text-xs">
                <span className="truncate">{c.name}</span>
                {c.id === displayChannelId ? (
                  <span className="ml-auto text-[10px] text-muted-foreground">当前</span>
                ) : null}
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="max-h-72 min-w-48 overflow-y-auto">
                {c.models.map((m) => (
                  <DropdownMenuItem
                    key={m.id}
                    onClick={() => void selectModel(c.id, m.id)}
                    className="flex items-center justify-between text-xs"
                  >
                    <span className="truncate">{m.id}</span>
                    {c.id === displayChannelId && m.id === displayModelId ? (
                      <Check className="size-3.5 shrink-0 text-primary" />
                    ) : null}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          ))
        )}

        {thinkingOn && efforts.length > 0 ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuLabel className="text-[11px] font-semibold text-muted-foreground/70 uppercase">
              {t("thinkingSelectorTitle", { defaultValue: "思考深度" })}
            </DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={resolvedEffort}
              onValueChange={(next) => next && void selectEffort(next)}
            >
              {efforts.map((level) => {
                const desc = effortDescOf(level);
                return (
                  <DropdownMenuRadioItem
                    key={level}
                    value={level}
                    closeOnClick
                    className="items-start py-1.5 text-xs"
                  >
                    <EffortIcon level={level} className="mt-0.5" />
                    <span className="flex min-w-0 flex-col gap-0.5">
                      <span className="font-medium">{effortTitleOf(level)}</span>
                      {desc ? (
                        <span className="text-[10px] text-muted-foreground">{desc}</span>
                      ) : null}
                    </span>
                  </DropdownMenuRadioItem>
                );
              })}
            </DropdownMenuRadioGroup>
          </>
        ) : null}

        <DropdownMenuSeparator />
        <DropdownMenuItem
          onClick={() => void navigate("/settings/channels")}
          className="text-xs text-muted-foreground hover:text-foreground"
        >
          <Settings2 className="size-3.5" />
          {t("manageChannels")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
