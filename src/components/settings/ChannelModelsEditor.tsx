import { useState } from "react";
import { ChevronRight, Lock, Plus, Sparkles, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  applyCatalogToModel,
  catalogThinkingLevels,
  displayedThinkingLevels,
  emptyChannelModel,
  lookupModelCatalog,
  selectedInputTypes,
  selectedThinkingLevels,
  toggleInputType,
  withThinkingLevels,
} from "@/lib/modelCatalog";
import { CHANNEL_INPUT_TYPES, type AiChannelModel, type ModelCatalogEntry } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

interface ChannelModelItemProps {
  model: AiChannelModel;
  index: number;
  catalog: ModelCatalogEntry[];
  disabled?: boolean;
  defaultOpen?: boolean;
  onUpdate: (updated: AiChannelModel) => void;
  onDelete: () => void;
}

function ChannelModelItem({
  model,
  index,
  catalog,
  disabled = false,
  defaultOpen = false,
  onUpdate,
  onDelete,
}: ChannelModelItemProps) {
  const { t } = useTranslation("settings");
  const [isOpen, setIsOpen] = useState(defaultOpen || !model.id.trim());

  const entry = lookupModelCatalog(catalog, model.id);
  const thinkingOn = model.thinking_enabled === true;
  const optionLevels = displayedThinkingLevels(model, catalog);
  const selectedLevels = selectedThinkingLevels(model, entry);
  const emptySelection = thinkingOn && selectedLevels.length === 0;
  const inputTypes = selectedInputTypes(model, entry);

  return (
    <div className="rounded-xl border border-border/70 bg-card shadow-2xs transition-all hover:border-border overflow-hidden">
      <div
        className="flex items-center justify-between gap-2 px-3 py-2 cursor-pointer select-none bg-muted/20 hover:bg-muted/30 transition-colors"
        onClick={() => setIsOpen((prev) => !prev)}
      >
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <ChevronRight
            className={cn(
              "size-3.5 text-muted-foreground transition-transform duration-150 shrink-0",
              isOpen && "rotate-90",
            )}
          />
          <span className="font-mono text-xs font-semibold text-foreground truncate">
            {model.id.trim() || t("channels.fields.modelId", { defaultValue: "未命名模型" })}
          </span>
          <span
            className={cn(
              "rounded-md border px-1.5 py-0.5 text-[10px] font-sans shrink-0",
              thinkingOn
                ? "border-primary/30 bg-primary/10 text-primary"
                : "border-border/60 bg-muted/40 text-muted-foreground",
            )}
          >
            {thinkingOn ? t("channels.status.thinkingOn") : t("channels.status.thinkingOff")}
          </span>
        </div>

        <div className="flex items-center gap-1 shrink-0" onClick={(e) => e.stopPropagation()}>
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            className="size-7 text-muted-foreground opacity-70 hover:text-destructive hover:opacity-100"
            disabled={disabled}
            title={t("channels.actions.delete")}
            onClick={onDelete}
          >
            <Trash2 className="size-3.5" />
          </Button>
        </div>
      </div>

      {isOpen ? (
        <div className="space-y-3 p-3 pt-2.5 border-t border-border/40">
          <div className="flex items-center gap-1.5">
            <Input
              className="h-8 text-xs font-mono"
              value={model.id}
              disabled={disabled}
              placeholder={t("channels.fields.modelId")}
              onChange={(event) => {
                const id = event.target.value;
                onUpdate(applyCatalogToModel(catalog, { ...model, id }));
              }}
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 px-2"
              disabled={disabled || !model.id.trim()}
              title={t("channels.actions.fillFromCatalog")}
              onClick={() => onUpdate(applyCatalogToModel(catalog, model, true))}
            >
              <Sparkles className="size-3.5 text-primary" />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              className="size-8 text-muted-foreground opacity-70 hover:text-destructive hover:opacity-100"
              disabled={disabled}
              title={t("channels.actions.delete")}
              onClick={onDelete}
            >
              <Trash2 className="size-3.5" />
            </Button>
          </div>

          <div className="grid grid-cols-2 gap-2.5">
            <div>
              <label className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider">
                {t("channels.fields.contextTokens")}
              </label>
              <Input
                type="number"
                min={1}
                className="mt-0.5 h-7 text-xs font-mono"
                disabled={disabled}
                value={model.context_tokens ?? ""}
                placeholder={entry ? String(entry.context_tokens) : "128000"}
                onChange={(event) => {
                  const parsed = Number(event.target.value);
                  onUpdate({
                    ...model,
                    context_tokens:
                      event.target.value && Number.isFinite(parsed) && parsed > 0 ? parsed : null,
                  });
                }}
              />
            </div>
            <div>
              <label className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider">
                {t("channels.fields.maxOutputTokens")}
              </label>
              <Input
                type="number"
                min={1}
                className="mt-0.5 h-7 text-xs font-mono"
                disabled={disabled}
                value={model.max_output_tokens ?? ""}
                placeholder={entry ? String(entry.max_output_tokens) : "8192"}
                onChange={(event) => {
                  const parsed = Number(event.target.value);
                  onUpdate({
                    ...model,
                    max_output_tokens:
                      event.target.value && Number.isFinite(parsed) && parsed > 0 ? parsed : null,
                  });
                }}
              />
            </div>
          </div>

          <div className="space-y-1.5 border-t border-border/40 pt-2">
            <label className="text-xs text-muted-foreground">
              {t("channels.fields.inputTypes")}
            </label>
            <div
              className="flex flex-wrap gap-2"
              role="group"
              aria-label={t("channels.fields.inputTypes")}
            >
              {CHANNEL_INPUT_TYPES.map((kind) => {
                const checkboxId = `channel-model-${index}-input-${kind}`;
                const locked = kind === "text";
                const checked = inputTypes.includes(kind);
                return (
                  <label
                    key={kind}
                    htmlFor={checkboxId}
                    title={locked ? t("channels.fields.inputTypeLocked") : undefined}
                    className={`flex items-center gap-1.5 rounded-md border border-border/60 bg-muted/30 px-2 py-1 text-xs text-foreground transition-colors ${
                      locked || disabled ? "cursor-default" : "cursor-pointer hover:bg-muted/60"
                    }`}
                  >
                    <Checkbox
                      id={checkboxId}
                      checked={checked}
                      disabled={disabled || locked}
                      aria-label={t(`channels.inputTypes.${kind}`)}
                      onCheckedChange={(nextChecked) => {
                        if (locked) return;
                        onUpdate(toggleInputType(model, entry, kind, nextChecked === true));
                      }}
                    />
                    <span>{t(`channels.inputTypes.${kind}`)}</span>
                    {locked ? (
                      <Lock className="size-3 text-muted-foreground" aria-hidden="true" />
                    ) : null}
                  </label>
                );
              })}
            </div>
          </div>

          <div className="space-y-2 border-t border-border/40 pt-2">
            <div className="flex items-center justify-between">
              <label className="text-xs text-muted-foreground">
                {t("channels.fields.thinking")}
              </label>
              <Select
                value={thinkingOn ? "on" : "off"}
                disabled={disabled}
                onValueChange={(value) => {
                  if (value !== "on" && value !== "off") return;
                  const levels =
                    selectedLevels.length > 0 ? selectedLevels : catalogThinkingLevels(entry);
                  onUpdate(
                    withThinkingLevels(model, levels, {
                      thinkingEnabled: value === "on",
                    }),
                  );
                }}
              >
                <SelectTrigger className="h-7 w-28 text-xs bg-background">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="off" className="text-xs">
                    {t("channels.status.thinkingOff")}
                  </SelectItem>
                  <SelectItem value="on" className="text-xs">
                    {t("channels.status.thinkingOn")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>

            {thinkingOn ? (
              <fieldset disabled={disabled} className="min-w-0 space-y-1.5 disabled:opacity-40">
                <div
                  className="flex flex-wrap gap-2 pt-0.5"
                  role="group"
                  aria-label={t("channels.fields.thinkingLevels")}
                >
                  {optionLevels.map((level) => {
                    const checkboxId = `channel-model-${index}-thinking-${level}`;
                    const checked = selectedLevels.includes(level);
                    return (
                      <label
                        key={level}
                        htmlFor={checkboxId}
                        className="flex cursor-pointer items-center gap-1.5 rounded-md border border-border/60 bg-muted/30 px-2 py-1 text-xs text-foreground transition-colors hover:bg-muted/60"
                      >
                        <Checkbox
                          id={checkboxId}
                          checked={checked}
                          disabled={disabled}
                          aria-label={t(`channels.thinkingLevels.${level}`, {
                            defaultValue: level,
                          })}
                          onCheckedChange={(nextChecked) => {
                            const next = nextChecked
                              ? [...selectedLevels, level]
                              : selectedLevels.filter((item) => item !== level);
                            onUpdate(
                              withThinkingLevels(model, next, {
                                thinkingEnabled: model.thinking_enabled,
                              }),
                            );
                          }}
                        />
                        <span className="font-mono text-[11px]">
                          {t(`channels.thinkingLevels.${level}`, { defaultValue: level })}
                        </span>
                      </label>
                    );
                  })}
                </div>
                {emptySelection ? (
                  <p className="text-[11px] text-destructive">
                    {t("channels.fields.thinkingLevelsEmpty")}
                  </p>
                ) : null}
              </fieldset>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

interface ChannelModelsEditorProps {
  models: AiChannelModel[];
  catalog: ModelCatalogEntry[];
  disabled?: boolean;
  defaultOpen?: boolean;
  onChange: (models: AiChannelModel[]) => void;
}

export function ChannelModelsEditor({
  models,
  catalog,
  disabled = false,
  defaultOpen = false,
  onChange,
}: ChannelModelsEditorProps) {
  const { t } = useTranslation("settings");

  const updateAt = (index: number, next: AiChannelModel) => {
    onChange(models.map((item, itemIndex) => (itemIndex === index ? next : item)));
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <label className="text-xs font-semibold text-foreground tracking-tight">
          {t("channels.fields.models")}
          <span className="ml-1.5 font-normal text-muted-foreground font-mono text-[10px]">
            ({models.length})
          </span>
        </label>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          onClick={() => onChange([...models, emptyChannelModel()])}
          className="h-6 text-xs gap-1 px-2"
        >
          <Plus className="size-3" />
          {t("channels.actions.addModel")}
        </Button>
      </div>

      {models.length === 0 ? (
        <p className="rounded-lg border border-dashed border-border/80 py-4 text-center text-xs text-muted-foreground">
          {t("channels.fields.modelsEmpty")}
        </p>
      ) : (
        models.map((model, index) => (
          <ChannelModelItem
            key={`${model.id}-${index}`}
            model={model}
            index={index}
            catalog={catalog}
            disabled={disabled}
            defaultOpen={defaultOpen}
            onUpdate={(next) => updateAt(index, next)}
            onDelete={() => onChange(models.filter((_, itemIndex) => itemIndex !== index))}
          />
        ))
      )}
      <p className="text-[11px] text-muted-foreground">{t("channels.fields.modelsHint")}</p>
    </div>
  );
}
