import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { updateNativeSettings } from "@/lib/backend";
import type { NativeHook } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard } from "./SettingCard";

const HOOK_EVENT_PRE = "pre_tool_use";
const HOOK_EVENT_POST = "post_tool_use";
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
  "AskQuestion",
] as const;

function normalizeHookEvent(event: string): typeof HOOK_EVENT_PRE | typeof HOOK_EVENT_POST {
  const value = event.trim();
  if (value === HOOK_EVENT_POST || value === "PostToolUse") {
    return HOOK_EVENT_POST;
  }
  return HOOK_EVENT_PRE;
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

  return (
    <SettingCard title={t("settings:hooks.title")} description={t("settings:hooks.hint")}>
      <div className="space-y-3">
        {hooks.map((hook, index) => {
          const event = normalizeHookEvent(hook.event);
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
                      if (value === HOOK_EVENT_PRE || value === HOOK_EVENT_POST) {
                        patchHook(index, { event: value });
                      }
                    }}
                  >
                    <SelectTrigger id={`hook-event-${hook.id}`} className="mt-1 bg-background">
                      <SelectValue>
                        {(value) =>
                          value === HOOK_EVENT_POST
                            ? t("settings:hooks.events.postToolUse")
                            : t("settings:hooks.events.preToolUse")
                        }
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={HOOK_EVENT_PRE}>
                        {t("settings:hooks.events.preToolUse")}
                      </SelectItem>
                      <SelectItem value={HOOK_EVENT_POST}>
                        {t("settings:hooks.events.postToolUse")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="min-w-0">
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
              </div>
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
                  event: HOOK_EVENT_PRE,
                  matcher: "*",
                  command: "",
                  timeout_secs: 30,
                  enabled: true,
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
