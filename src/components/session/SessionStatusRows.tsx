import { Bot, Plug, Shield } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  parseAgentBanner,
  parseMcpStatus,
  permissionHint,
  stripAgentPrefix,
} from "@/lib/sessionLines";

export function PermissionStatusRow({ text }: { text: string }) {
  const hint = permissionHint(text);
  if (!hint) return null;
  return (
    <p className="flex items-start gap-2 text-xs text-muted-foreground">
      <Shield className="mt-0.5 size-3.5 shrink-0" />
      <span>{hint}</span>
    </p>
  );
}

export function AgentStatusRow({ text }: { text: string }) {
  const { t } = useTranslation("sessions");
  const banner = parseAgentBanner(text);
  if (!banner) {
    return (
      <p className="flex items-start gap-2 text-xs text-muted-foreground">
        <Bot className="mt-0.5 size-3.5 shrink-0" />
        <span>{stripAgentPrefix(text)}</span>
      </p>
    );
  }
  const effortTitle =
    banner.effort === "默认"
      ? banner.effort
      : t(`effortLevels.${banner.effort}.title`, { defaultValue: banner.effort });
  return (
    <div className="space-y-1 text-xs text-muted-foreground">
      <p className="flex items-center gap-2">
        <Bot className="size-3.5 shrink-0" />
        <span>{t("agentStarted")}</span>
      </p>
      <p className="flex flex-wrap gap-x-3 gap-y-0.5 pl-[22px]">
        <span>
          {t("agentChannel")} {banner.channel}
        </span>
        <span>
          {t("model")} {banner.model}
        </span>
        <span>
          {t("effort")} {effortTitle}
        </span>
        <span>
          {t("thinking")} {banner.thinking ? t("thinkingOn") : t("thinkingOff")}
        </span>
      </p>
    </div>
  );
}

export function McpStatusRow({ text }: { text: string }) {
  const { t } = useTranslation("sessions");
  const parsed = parseMcpStatus(text);
  if (!parsed) return null;
  let label = t("mcpLabel");
  let tip = parsed.kind === "off" ? t("mcpOff") : text.replace(/^\[MCP\]\s*/, "");
  if (parsed.kind === "off") {
    label = t("mcpOff");
  } else if (parsed.kind === "on") {
    const names = parsed.servers.join("、");
    label = names ? `${t("mcpOn")} ${names}` : t("mcpOn");
    tip = names || t("mcpOn");
  } else if (parsed.kind === "pending") {
    label = t("mcpPending");
    tip = text.replace(/^\[MCP\]\s*/, "");
  } else if (parsed.kind === "error") {
    label = t("mcpError");
    tip = parsed.detail;
  } else {
    tip = parsed.detail;
  }
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger className="flex max-w-full items-start gap-2 text-left text-xs text-muted-foreground">
          <Plug className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 truncate">{label}</span>
        </TooltipTrigger>
        <TooltipContent>{tip}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
