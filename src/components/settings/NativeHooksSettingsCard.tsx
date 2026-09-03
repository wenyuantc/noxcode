import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { updateNativeSettings } from "@/lib/backend";
import {
  NATIVE_HOOK_EVENTS,
  NATIVE_HOOK_HANDLER_TYPES,
  type NativeHook,
  type NativeHookEvent,
  type NativeHookHandlerType,
} from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard } from "./SettingCard";

const HOOK_MATCHER_ALL = "*";
const HOOK_MATCHER_TOOLS = [
  "Read",
  "Write",
  "Edit",
  "Bash",
  "Glob",
  "Grep",
  "TodoRead",
  "TodoWrite",
  "WebFetch",
  "WebSearch",
  "ApplyPatch",
  "Skill",
  "Agent",
  "AskUserQuestion",
  "EnterPlanMode",
  "ExitPlanMode",
] as const;
/// 只有工具类事件才需要匹配器。
const TOOL_EVENTS: NativeHookEvent[] = [
  "pre_tool_use",
  "post_tool_use",
  "post_tool_use_failure",
  "permission_request",
];

function normalizeHookEvent(event: string): NativeHookEvent {
  const value = event.trim();
  const found = NATIVE_HOOK_EVENTS.find((item) => item === value);
  if (found) return found;
  if (value === "PostToolUse") return "post_tool_use";
  return "pre_tool_use";
}

function normalizeHandlerType(value: string | undefined): NativeHookHandlerType {
  return value === "http" || value === "agent" ? value : "command";
}

function parseMatcher(matcher: string): string[] {
  const items = matcher
    .split(",")
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
  if (items.length === 0 || items.includes(HOOK_MATCHER_ALL)) {
    return [HOOK_MATCHER_ALL];
  }
  return items;
}

function matcherChoices(matcher: string): string[] {
  const known = new Set<string>([HOOK_MATCHER_ALL, ...HOOK_MATCHER_TOOLS]);
  const extra = parseMatcher(matcher).filter((item) => !known.has(item));
  return [HOOK_MATCHER_ALL, ...HOOK_MATCHER_TOOLS, ...extra];
}

function nextMatcher(previous: string[], selected: string[]): string {
  const previousAll = previous.includes(HOOK_MATCHER_ALL);
  const selectedAll = selected.includes(HOOK_MATCHER_ALL);
  const tools = selected.filter((item) => item !== HOOK_MATCHER_ALL);
  if (selectedAll && !previousAll) {
    return HOOK_MATCHER_ALL;
  }
  if (tools.length === 0) {
    return HOOK_MATCHER_ALL;
  }
  return tools.join(", ");
}

function formatMatcherValue(selected: string[], allLabel: string): string {
  if (selected.length === 0 || selected.includes(HOOK_MATCHER_ALL)) {
    return allLabel;
  }
  return selected.join(", ");
}

export function NativeHooksSettingsCard() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [hooks, setHooks] = useState<NativeHook[]>(native?.hooks ?? []);

  useEffect(() => {
    if (native) setHooks(native.hooks);
  }, [native]);

  const patchHook = (index: number, patch: Partial<NativeHook>) => {
    const current = hooks[index];
    if (!current) return;
    const next = [...hooks];
    next[index] = { ...current, ...patch };
    setHooks(next);
  };

  const eventLabel = (event: NativeHookEvent) => t(`settings:hooks.events.${event}`);
  const handlerLabel = (handler: NativeHookHandlerType) => t(`settings:hooks.handlers.${handler}`);

  return (
    <SettingCard title={t("settings:hooks.title")} description={t("settings:hooks.hint")}>
      <div className="space-y-3">
        {hooks.map((hook, index) => {
          const event = normalizeHookEvent(hook.event);
          const handler = normalizeHandlerType(hook.handler_type);
          const needsMatcher = TOOL_EVENTS.includes(event);
          return (
            <div key={hook.id} className="space-y-3 rounded-md border p-3">
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label
                    className="text-xs font-medium text-muted-foreground"
                    htmlFor={`hook-event-${hook.id}`}
                  >
                    {t("settings:hooks.fields.event")}
                  </label>
                  <Select
                    value={event}
                    onValueChange={(value) => {
                      const found = NATIVE_HOOK_EVENTS.find((item) => item === value);
                      if (found) patchHook(index, { event: found });
                    }}
                  >
                    <SelectTrigger id={`hook-event-${hook.id}`} className="mt-1 bg-background">
                      <SelectValue>
                        {(value) => eventLabel(normalizeHookEvent(String(value)))}
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      {NATIVE_HOOK_EVENTS.map((item) => (
                        <SelectItem key={item} value={item}>
                          {eventLabel(item)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <label
                    className="text-xs font-medium text-muted-foreground"
                    htmlFor={`hook-handler-${hook.id}`}
                  >
                    {t("settings:hooks.fields.handler")}
                  </label>
                  <Select
                    value={handler}
                    onValueChange={(value) => {
                      const next = normalizeHandlerType(String(value));
                      patchHook(index, { handler_type: next });
                    }}
                  >
                    <SelectTrigger id={`hook-handler-${hook.id}`} className="mt-1 bg-background">
                      <SelectValue>
                        {(value) => handlerLabel(normalizeHandlerType(String(value)))}
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      {NATIVE_HOOK_HANDLER_TYPES.map((item) => (
                        <SelectItem key={item} value={item}>
                          {handlerLabel(item)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                {needsMatcher ? (
                  <div className="col-span-2 min-w-0">
                    <label
                      className="text-xs font-medium text-muted-foreground"
                      htmlFor={`hook-matcher-${hook.id}`}
                    >
                      {t("settings:hooks.fields.matcher")}
                    </label>
                    <Select
                      multiple
                      value={parseMatcher(hook.matcher)}
                      onValueChange={(value) => {
                        if (!Array.isArray(value)) return;
                        patchHook(index, {
                          matcher: nextMatcher(parseMatcher(hook.matcher), value),
                        });
                      }}
                    >
                      <SelectTrigger id={`hook-matcher-${hook.id}`} className="mt-1 bg-background">
                        <SelectValue>
                          {(value) =>
                            formatMatcherValue(
                              Array.isArray(value) ? value : parseMatcher(hook.matcher),
                              t("settings:hooks.matchers.all"),
                            )
                          }
                        </SelectValue>
                      </SelectTrigger>
                      <SelectContent>
                        {matcherChoices(hook.matcher).map((tool) => (
                          <SelectItem key={tool} value={tool}>
                            {tool === HOOK_MATCHER_ALL ? t("settings:hooks.matchers.all") : tool}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {t("settings:hooks.fieldHints.matcher")}
                    </p>
                  </div>
                ) : null}
              </div>
              {handler === "command" ? (
                <div>
                  <label
                    className="text-xs font-medium text-muted-foreground"
                    htmlFor={`hook-command-${hook.id}`}
                  >
                    {t("settings:hooks.fields.command")}
                  </label>
                  <Input
                    id={`hook-command-${hook.id}`}
                    className="mt-1"
                    value={hook.command}
                    placeholder={t("settings:hooks.placeholders.command")}
                    onChange={(e) => patchHook(index, { command: e.target.value })}
                  />
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("settings:hooks.fieldHints.command")}
                  </p>
                </div>
              ) : null}
              {handler === "http" ? (
                <div>
                  <label
                    className="text-xs font-medium text-muted-foreground"
                    htmlFor={`hook-url-${hook.id}`}
                  >
                    {t("settings:hooks.fields.url")}
                  </label>
                  <Input
                    id={`hook-url-${hook.id}`}
                    className="mt-1"
                    value={hook.url ?? ""}
                    placeholder="https://"
                    onChange={(e) => patchHook(index, { url: e.target.value })}
                  />
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("settings:hooks.fieldHints.url")}
                  </p>
                </div>
              ) : null}
              {handler === "agent" ? (
                <div>
                  <label
                    className="text-xs font-medium text-muted-foreground"
                    htmlFor={`hook-agent-${hook.id}`}
                  >
                    {t("settings:hooks.fields.agentPrompt")}
                  </label>
                  <Textarea
                    id={`hook-agent-${hook.id}`}
                    className="mt-1"
                    value={hook.agent_prompt ?? ""}
                    placeholder={t("settings:hooks.placeholders.agentPrompt")}
                    onChange={(e) => patchHook(index, { agent_prompt: e.target.value })}
                  />
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("settings:hooks.fieldHints.agentPrompt")}
                  </p>
                </div>
              ) : null}
              <div className="flex items-center justify-between gap-3">
                <label
                  className="text-xs text-muted-foreground"
                  htmlFor={`hook-timeout-${hook.id}`}
                >
                  {t("settings:hooks.fields.timeout")}
                  <Input
                    id={`hook-timeout-${hook.id}`}
                    className="mt-1 w-28"
                    type="number"
                    min={1}
                    max={120}
                    value={hook.timeout_secs}
                    onChange={(e) => patchHook(index, { timeout_secs: Number(e.target.value) })}
                  />
                </label>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setHooks(hooks.filter((_, i) => i !== index))}
                >
                  {t("common:delete")}
                </Button>
              </div>
            </div>
          );
        })}
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={() =>
              setHooks([
                ...hooks,
                {
                  id: crypto.randomUUID(),
                  event: "pre_tool_use",
                  matcher: "*",
                  command: "",
                  timeout_secs: 30,
                  enabled: true,
                  handler_type: "command",
                  url: null,
                  agent_prompt: null,
                  source: "global",
                },
              ])
            }
          >
            {t("common:create")}
          </Button>
          <Button
            onClick={() =>
              void updateNativeSettings({
                hooks: hooks.map((hook) => ({
                  ...hook,
                  event: normalizeHookEvent(hook.event),
                  handler_type: normalizeHandlerType(hook.handler_type),
                })),
              }).then(setNative)
            }
          >
            {t("common:save")}
          </Button>
        </div>
      </div>
    </SettingCard>
  );
}
