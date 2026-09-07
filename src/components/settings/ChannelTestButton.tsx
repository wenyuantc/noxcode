import { ChevronDown, Loader2, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export interface ChannelTestButtonProps {
  models: Array<{ id: string }>;
  defaultModelId?: string | null;
  disabled?: boolean;
  isTesting?: boolean;
  onTest: (modelId: string | null) => void;
  className?: string;
}

export function ChannelTestButton({
  models,
  defaultModelId,
  disabled = false,
  isTesting = false,
  onTest,
  className,
}: ChannelTestButtonProps) {
  const { t } = useTranslation("settings");

  const validModels = models.filter((m) => Boolean(m.id && m.id.trim().length > 0));
  const hasMultipleModels = validModels.length > 1;

  const effectiveDefaultModel =
    defaultModelId && validModels.some((m) => m.id === defaultModelId)
      ? defaultModelId
      : (validModels[0]?.id ?? null);

  const isDisabled = disabled || isTesting;

  if (!hasMultipleModels) {
    return (
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={isDisabled}
        onClick={() => onTest(effectiveDefaultModel)}
        className={cn("h-7 text-xs gap-1", className)}
      >
        {isTesting ? <Loader2 className="size-3 animate-spin" /> : <Zap className="size-3" />}
        {t("channels.actions.test")}
      </Button>
    );
  }

  return (
    <div className={cn("inline-flex items-center", className)}>
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={isDisabled}
        onClick={() => onTest(effectiveDefaultModel)}
        title={
          effectiveDefaultModel
            ? `${t("channels.actions.test")} (${effectiveDefaultModel})`
            : t("channels.actions.test")
        }
        className={cn(
          "h-7 text-xs gap-1 rounded-r-none border-r-0 focus-visible:z-10",
          isTesting && "opacity-80",
        )}
      >
        {isTesting ? <Loader2 className="size-3 animate-spin" /> : <Zap className="size-3" />}
        {t("channels.actions.test")}
      </Button>

      <DropdownMenu>
        <DropdownMenuTrigger
          disabled={isDisabled}
          aria-label={t("channels.actions.testModel")}
          title={t("channels.actions.testModel")}
          className={cn(
            "inline-flex h-7 w-6 shrink-0 cursor-pointer items-center justify-center rounded-r-lg border border-border bg-background text-muted-foreground transition-all outline-none select-none hover:bg-muted hover:text-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:z-10 disabled:pointer-events-none disabled:opacity-50 dark:border-input dark:bg-input/30 dark:hover:bg-input/50",
            isTesting && "opacity-80 pointer-events-none",
          )}
        >
          <ChevronDown className="size-3" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" side="bottom" sideOffset={4} className="min-w-44 max-w-72">
          {validModels.map((model) => {
            const isDefault = model.id === effectiveDefaultModel;
            return (
              <DropdownMenuItem
                key={model.id}
                onClick={() => onTest(model.id)}
                className="text-xs font-mono justify-between gap-2 cursor-pointer"
              >
                <span className="truncate">{model.id}</span>
                {isDefault ? (
                  <span className="rounded bg-primary/10 px-1 py-0.2 text-[10px] font-sans text-primary shrink-0">
                    {t("channels.actions.defaultModel")}
                  </span>
                ) : null}
              </DropdownMenuItem>
            );
          })}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
