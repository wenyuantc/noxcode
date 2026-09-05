import { Check, ChevronDown } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { useState } from "react";

import { cn } from "@/lib/utils";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
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
  disabled = false,
}: {
  onError?: (error: string) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const navigate = useNavigate();
  const channels = useChannelStore((state) => state.channels);
  const activeChannelId = useChannelStore((state) => state.activeChannelId);
  const activeModelId = useChannelStore((state) => state.activeModelId);
  const setSelection = useChannelStore((state) => state.setSelection);
  const sessionId = useSessionStore((state) => state.selectedSessionId);
  const runtime = useSessionStore((state) =>
    sessionId ? state.configurationBySession[sessionId] : undefined,
  );
  const [busy, setBusy] = useState(false);
  const selectedChannelId = runtime?.ai_channel_id ?? activeChannelId;
  const selectedModelId = runtime?.model ?? activeModelId;

  const enabled = channels.filter((channel) => channel.enabled);
  const channel = channels.find((item) => item.id === selectedChannelId);
  const label = selectedModelId
    ? `${channel?.name ?? selectedChannelId}/${selectedModelId}`
    : t("needChannel");
  const select = async (channelId: string, modelId: string) => {
    if (busy || disabled) return;
    setBusy(true);
    try {
      await changeSessionConfiguration(sessionId, { ai_channel_id: channelId, model: modelId });
      setSelection(channelId, modelId);
    } catch (reason) {
      onError?.(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled || busy}
        className="inline-flex h-7 min-w-0 max-w-[min(12rem,100%)] cursor-pointer items-center justify-between gap-1.5 rounded-lg border border-border/70 bg-background/80 px-2 text-xs font-medium text-foreground/90 shadow-2xs transition-all duration-150 outline-none hover:bg-muted/40 disabled:opacity-60"
      >
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
                {item.id === selectedChannelId ? <Check className="size-3.5" /> : null}
                <span className={cn(item.id !== selectedChannelId && "pl-5")}>{item.name}</span>
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
                      {item.id === selectedChannelId && model.id === selectedModelId ? (
                        <Check className="size-3.5" />
                      ) : null}
                      <span
                        className={cn(
                          item.id === selectedChannelId && model.id === selectedModelId
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
