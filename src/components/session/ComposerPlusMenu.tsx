import { Bot, FileText, Package, Paperclip, Plus, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { ComposerTriggerChar } from "@/lib/composerImages";

interface ComposerPlusMenuProps {
  onAddAttachment: () => void;
  onInsertTrigger: (trigger: ComposerTriggerChar) => void;
}

export function ComposerPlusMenu({ onAddAttachment, onInsertTrigger }: ComposerPlusMenuProps) {
  const { t } = useTranslation("sessions");

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={t("plusMenu")}
        title={t("plusMenu")}
        className="inline-flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-lg border border-border/70 bg-background/80 text-foreground/90 shadow-2xs transition-all duration-150 outline-none hover:bg-muted/40"
      >
        <Plus className="size-3.5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top" sideOffset={6} className="min-w-56">
        <DropdownMenuItem onClick={onAddAttachment}>
          <Paperclip className="size-3.5 text-muted-foreground" />
          {t("addAttachment")}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onInsertTrigger("@")}>
          <FileText className="size-3.5 text-muted-foreground" />
          {t("useAtContext")}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onInsertTrigger("$")}>
          <Package className="size-3.5 text-cyan-500" />
          {t("useDollarSkill")}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onInsertTrigger("/")}>
          <Bot className="size-3.5 text-purple-500" />
          {t("slashSubagents", { defaultValue: "子智能体" })}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onInsertTrigger("/")}>
          <Zap className="size-3.5 text-amber-500" />
          {t("useSlashCapability")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
