import { Check, ChevronDown, Clock } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { resolveSessionSelection, type SessionModelSelection } from "@/lib/sessionModel";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { changeSessionConfiguration } from "@/lib/sessionConfiguration";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

export function ChannelModelPicker({
  onError,
  onInfo,
  disabled = false,
  selection,
  onSelectionChange,
  className,
}: {
  onError?: (error: string) => void;
  onInfo?: (message: string) => void;
  disabled?: boolean;
  selection?: SessionModelSelection;
  onSelectionChange?: (channelId: string, modelId: string) => void;
  className?: string;
}) {
  const { t } = useTranslation("sessions");
  const navigate = useNavigate();
  const channels = useChannelStore((state) => state.channels);
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

  const enabled = channels.filter((channel) => channel.enabled);
  const channel = channels.find((item) => item.id === displayChannelId);
  const label = displayModelId
    ? `${channel?.name ?? displayChannelId}/${displayModelId}`
    : t("needChannel");
  const select = async (channelId: string, modelId: string) => {
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

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled}
        title={showPending ? t("modelPending") : undefined}
        className={cn(
          "inline-flex h-7 min-w-0 max-w-full cursor-pointer items-center justify-between gap-1.5 rounded-lg border border-border/70 bg-background/80 px-2 text-xs font-medium text-foreground/90 shadow-2xs transition-all duration-150 outline-none hover:bg-muted/40 disabled:opacity-60",
          className,
        )}
      >
        {showPending ? <Clock className="size-3 shrink-0 text-amber-500" /> : null}
        <span className="truncate">{label}</span>
        <ChevronDown className="size-3 shrink-0 text-muted-foreground/70" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-48">
        {enabled.length === 0 ? (
          <DropdownMenuItem disabled>{t("needChannel")}</DropdownMenuItem>
        ) : (
          enabled.map((item) => (
            <DropdownMenuSub key={item.id}>
              <DropdownMenuSubTrigger>
                {item.id === displayChannelId ? <Check className="size-3.5" /> : null}
                <span className={cn(item.id !== displayChannelId && "pl-5")}>{item.name}</span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                {item.models.length === 0 ? (
                  <DropdownMenuItem disabled>{t("needChannel")}</DropdownMenuItem>
                ) : (
                  item.models.map((model) => (
                    <DropdownMenuItem
                      key={model.id}
                      onClick={() => {
                        void select(item.id, model.id);
                      }}
                    >
                      {item.id === displayChannelId && model.id === displayModelId ? (
                        <Check className="size-3.5" />
                      ) : null}
                      <span
                        className={cn(
                          item.id === displayChannelId && model.id === displayModelId
                            ? undefined
                            : "pl-5",
                        )}
                      >
                        {model.id}
                      </span>
                    </DropdownMenuItem>
                  ))
                )}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          ))
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={() => void navigate("/settings/channels")}>
          {t("manageModels")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
