import { AtSign, CircleSlash, DollarSign, Paperclip, Plus } from "lucide-react";
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
          <Paperclip className="size-3.5" />
          {t("addAttachment")}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onInsertTrigger("@")}>
          <AtSign className="size-3.5" />
          {t("useAtContext")}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onInsertTrigger("/")}>
          <CircleSlash className="size-3.5" />
          {t("useSlashCapability")}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onInsertTrigger("$")}>
          <DollarSign className="size-3.5" />
          {t("useDollarSkill")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
