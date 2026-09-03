import { Check, ChevronsUpDown } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { useChannelStore } from "@/stores/channelStore";
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

export function ChannelModelPicker() {
  const { t } = useTranslation("sessions");
  const navigate = useNavigate();
  const channels = useChannelStore((state) => state.channels);
  const activeChannelId = useChannelStore((state) => state.activeChannelId);
  const activeModelId = useChannelStore((state) => state.activeModelId);
  const setSelection = useChannelStore((state) => state.setSelection);

  const enabled = channels.filter((channel) => channel.enabled);
  const channel = enabled.find((item) => item.id === activeChannelId);
  const label = channel && activeModelId ? `${channel.name}/${activeModelId}` : t("needChannel");

  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="inline-flex h-7 max-w-48 items-center justify-between gap-1 rounded-md border bg-background px-2 text-xs outline-none">
        <span className="truncate">{label}</span>
        <ChevronsUpDown className="size-3.5 shrink-0 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-48">
        {enabled.length === 0 ? (
          <DropdownMenuItem disabled>{t("needChannel")}</DropdownMenuItem>
        ) : (
          enabled.map((item) => (
            <DropdownMenuSub key={item.id}>
              <DropdownMenuSubTrigger>
                {item.id === activeChannelId ? <Check className="size-3.5" /> : null}
                <span className={cn(item.id !== activeChannelId && "pl-5")}>{item.name}</span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                {item.models.length === 0 ? (
                  <DropdownMenuItem disabled>{t("needChannel")}</DropdownMenuItem>
                ) : (
                  item.models.map((model) => (
                    <DropdownMenuItem
                      key={model.id}
                      onClick={() => setSelection(item.id, model.id)}
                    >
                      {item.id === activeChannelId && model.id === activeModelId ? (
                        <Check className="size-3.5" />
                      ) : null}
                      <span
                        className={cn(
                          item.id === activeChannelId && model.id === activeModelId
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
