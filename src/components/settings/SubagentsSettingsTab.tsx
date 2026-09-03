import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  createNativeSubagent,
  deleteNativeSubagent,
  listNativeSubagents,
  listWorkspaces,
  updateNativeSubagent,
} from "@/lib/backend";
import type { NativeSubagent, Workspace } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { SettingCard } from "./SettingCard";

export function SubagentsSettingsTab() {
  const { t } = useTranslation(["settings", "common"]);
  const [items, setItems] = useState<NativeSubagent[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [editing, setEditing] = useState<NativeSubagent | null>(null);

  const reload = () => void listNativeSubagents().then(setItems);
  useEffect(() => {
    void reload();
    void listWorkspaces().then(setWorkspaces);
  }, []);

  return (
    <SettingCard title={t("settings:subagents.title")} description={t("settings:subagents.hint")}>
      <div className="space-y-2">
        {items.map((item) => (
          <div
            key={item.id}
            className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm"
          >
            <div className="flex-1">
              <p className="font-medium">{item.name}</p>
              <p className="text-xs text-muted-foreground">{item.description}</p>
            </div>
            <Button size="sm" variant="outline" onClick={() => setEditing(item)}>
              {t("common:edit")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void deleteNativeSubagent(item.id).then(reload)}
            >
              {t("common:delete")}
            </Button>
          </div>
        ))}
        <Button
          size="sm"
          onClick={() =>
            setEditing({
              id: "",
              name: "",
              description: "",
              model_mode: "inherit",
              channel_id: null,
              model: null,
              tool_mode: "all",
              tools: [],
              system_prompt: "",
              inject_agents_md: true,
              scope: "all",
              workspace_ids: [],
            })
          }
        >
          {t("common:create")}
        </Button>
      </div>
      {editing ? (
        <div className="mt-4 space-y-2">
          <Input
            value={editing.name}
            onChange={(e) => setEditing({ ...editing, name: e.target.value })}
          />
          <Input
            value={editing.description}
            onChange={(e) => setEditing({ ...editing, description: e.target.value })}
          />
          <Textarea
            value={editing.system_prompt}
            onChange={(e) => setEditing({ ...editing, system_prompt: e.target.value })}
          />
          <select
            className="h-8 w-full rounded-md border px-2 text-sm"
            value={editing.scope}
            onChange={(e) => setEditing({ ...editing, scope: e.target.value })}
          >
            <option value="all">all</option>
            <option value="workspaces">workspaces</option>
          </select>
          {editing.scope === "workspaces" ? (
            <select
              multiple
              className="min-h-24 w-full rounded-md border px-2 text-sm"
              value={editing.workspace_ids}
              onChange={(e) =>
                setEditing({
                  ...editing,
                  workspace_ids: Array.from(e.target.selectedOptions).map((item) => item.value),
                })
              }
            >
              {workspaces.map((workspace) => (
                <option key={workspace.id} value={workspace.id}>
                  {workspace.name}
                </option>
              ))}
            </select>
          ) : null}
          <Button
            onClick={() => {
              const payload = {
                name: editing.name,
                description: editing.description,
                system_prompt: editing.system_prompt,
                scope: editing.scope,
                workspace_ids: editing.workspace_ids,
              };
              const task = editing.id
                ? updateNativeSubagent(editing.id, payload)
                : createNativeSubagent(payload);
              void task.then(() => {
                setEditing(null);
                void reload();
              });
            }}
          >
            {t("common:save")}
          </Button>
        </div>
      ) : null}
    </SettingCard>
  );
}
