import { Plus, Sparkles, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  applyCatalogToModel,
  catalogThinkingLevels,
  displayedThinkingLevels,
  emptyChannelModel,
  lookupModelCatalog,
  selectedThinkingLevels,
  withThinkingLevels,
} from "@/lib/modelCatalog";
import type { AiChannelModel, ModelCatalogEntry } from "@/lib/types";
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

interface ChannelModelsEditorProps {
  models: AiChannelModel[];
  catalog: ModelCatalogEntry[];
  disabled?: boolean;
  onChange: (models: AiChannelModel[]) => void;
}

export function ChannelModelsEditor({
  models,
  catalog,
  disabled = false,
  onChange,
}: ChannelModelsEditorProps) {
  const { t } = useTranslation("settings");

  const updateAt = (index: number, next: AiChannelModel) => {
    onChange(models.map((item, itemIndex) => (itemIndex === index ? next : item)));
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <label className="text-xs font-medium text-muted-foreground">
          {t("channels.fields.models")}
        </label>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          onClick={() => onChange([...models, emptyChannelModel()])}
        >
          <Plus className="mr-1 h-3.5 w-3.5" />
          {t("channels.actions.addModel")}
        </Button>
      </div>
      {models.length === 0 ? (
        <p className="text-[11px] text-muted-foreground">{t("channels.fields.modelsEmpty")}</p>
      ) : (
        models.map((model, index) => {
          const entry = lookupModelCatalog(catalog, model.id);
          const thinkingOn = model.thinking_enabled === true;
          const optionLevels = displayedThinkingLevels(model, catalog);
          const selectedLevels = selectedThinkingLevels(model, entry);
          const emptySelection = thinkingOn && selectedLevels.length === 0;
          return (
            <div
              key={`${model.id}-${index}`}
              className="space-y-2 rounded-md border border-border p-3"
            >
              <div className="flex gap-2">
                <Input
                  value={model.id}
                  disabled={disabled}
                  placeholder={t("channels.fields.modelId")}
                  onChange={(event) => {
                    const id = event.target.value;
                    updateAt(index, applyCatalogToModel(catalog, { ...model, id }));
                  }}
                />
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={disabled || !model.id.trim()}
                  title={t("channels.actions.fillFromCatalog")}
                  onClick={() => updateAt(index, applyCatalogToModel(catalog, model, true))}
                >
                  <Sparkles className="h-3.5 w-3.5" />
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={disabled}
                  onClick={() => onChange(models.filter((_, itemIndex) => itemIndex !== index))}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="text-[11px] text-muted-foreground">
                    {t("channels.fields.contextTokens")}
                  </label>
                  <Input
                    type="number"
                    min={1}
                    disabled={disabled}
                    value={model.context_tokens ?? ""}
                    placeholder={entry ? String(entry.context_tokens) : "128000"}
                    onChange={(event) => {
                      const parsed = Number(event.target.value);
                      updateAt(index, {
                        ...model,
                        context_tokens:
                          event.target.value && Number.isFinite(parsed) && parsed > 0
                            ? parsed
                            : null,
                      });
                    }}
                  />
                </div>
                <div>
                  <label className="text-[11px] text-muted-foreground">
                    {t("channels.fields.maxOutputTokens")}
                  </label>
                  <Input
                    type="number"
                    min={1}
                    disabled={disabled}
                    value={model.max_output_tokens ?? ""}
                    placeholder={entry ? String(entry.max_output_tokens) : "8192"}
                    onChange={(event) => {
                      const parsed = Number(event.target.value);
                      updateAt(index, {
                        ...model,
                        max_output_tokens:
                          event.target.value && Number.isFinite(parsed) && parsed > 0
                            ? parsed
                            : null,
                      });
                    }}
                  />
                </div>
              </div>
              <div className="space-y-2">
                <div>
                  <label className="text-[11px] text-muted-foreground">
                    {t("channels.fields.thinking")}
                  </label>
                  <Select
                    value={thinkingOn ? "on" : "off"}
                    disabled={disabled}
                    onValueChange={(value) => {
                      if (value !== "on" && value !== "off") return;
                      const levels =
                        selectedLevels.length > 0 ? selectedLevels : catalogThinkingLevels(entry);
                      updateAt(
                        index,
                        withThinkingLevels(model, levels, {
                          thinkingEnabled: value === "on",
                        }),
                      );
                    }}
                  >
                    <SelectTrigger className="mt-1 bg-background">
                      <SelectValue>
                        {(value) =>
                          value === "on"
                            ? t("channels.status.thinkingOn")
                            : t("channels.status.thinkingOff")
                        }
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="off">{t("channels.status.thinkingOff")}</SelectItem>
                      <SelectItem value="on">{t("channels.status.thinkingOn")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <fieldset
                  disabled={disabled || !thinkingOn}
                  className="min-w-0 space-y-1.5 disabled:opacity-50"
                >
                  <legend className="text-[11px] text-muted-foreground">
                    {t("channels.fields.thinkingLevels")}
                  </legend>
                  <div
                    className="flex flex-wrap gap-x-3 gap-y-1.5"
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
                          className="flex cursor-pointer items-center gap-1.5 text-[11px] text-foreground"
                        >
                          <Checkbox
                            id={checkboxId}
                            checked={checked}
                            disabled={disabled || !thinkingOn}
                            aria-label={t(`channels.thinkingLevels.${level}`, {
                              defaultValue: level,
                            })}
                            onCheckedChange={(nextChecked) => {
                              const next = nextChecked
                                ? [...selectedLevels, level]
                                : selectedLevels.filter((item) => item !== level);
                              updateAt(
                                index,
                                withThinkingLevels(model, next, {
                                  thinkingEnabled: model.thinking_enabled,
                                }),
                              );
                            }}
                          />
                          <span>
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
                  ) : (
                    <p className="text-[11px] text-muted-foreground">
                      {t("channels.fields.thinkingLevelsHint")}
                    </p>
                  )}
                </fieldset>
              </div>
            </div>
          );
        })
      )}
      <p className="text-[11px] text-muted-foreground">{t("channels.fields.modelsHint")}</p>
    </div>
  );
}
