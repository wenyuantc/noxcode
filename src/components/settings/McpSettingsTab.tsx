import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { getMcpServers, resetMcpServers, updateMcpServers } from "@/lib/backend";
import type { McpServerConfig } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SettingCard } from "./SettingCard";

export function McpSettingsTab() {
  const { t } = useTranslation(["settings", "common"]);
  const [servers, setServers] = useState<McpServerConfig[]>([]);

  useEffect(() => {
    void getMcpServers().then((doc) => setServers(doc.servers));
  }, []);

  return (
    <SettingCard title={t("settings:mcp.title")} description={t("settings:mcp.hint")}>
      <div className="space-y-3">
        {servers.map((server, index) => (
          <div key={server.id} className="space-y-2 rounded-md border p-3">
            <Input
              value={server.name}
              onChange={(event) => {
                const next = [...servers];
                next[index] = { ...server, name: event.target.value };
                setServers(next);
              }}
            />
            <Input
              value={server.command}
              onChange={(event) => {
                const next = [...servers];
                next[index] = { ...server, command: event.target.value };
                setServers(next);
              }}
            />
            <Input
              value={server.args.join(" ")}
              onChange={(event) => {
                const next = [...servers];
                next[index] = { ...server, args: event.target.value.split(" ").filter(Boolean) };
                setServers(next);
              }}
            />
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={server.enabled}
                onChange={(event) => {
                  const next = [...servers];
                  next[index] = { ...server, enabled: event.target.checked };
                  setServers(next);
                }}
              />
              {t("settings:channels.status.enabled")}
            </label>
          </div>
        ))}
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={() =>
              setServers([
                ...servers,
                {
                  id: crypto.randomUUID(),
                  name: "mcp",
                  command: "",
                  args: [],
                  env: [],
                  enabled: true,
                  notes: null,
                },
              ])
            }
          >
            {t("settings:mcp.add")}
          </Button>
          <Button
            onClick={() =>
              void updateMcpServers({ servers }).then((doc) => setServers(doc.servers))
            }
          >
            {t("common:save")}
          </Button>
          <Button
            variant="ghost"
            onClick={() => void resetMcpServers().then((doc) => setServers(doc.servers))}
          >
            {t("settings:mcp.reset")}
          </Button>
        </div>
      </div>
    </SettingCard>
  );
}
