import { useEffect, useState } from "react";
import { Loader2, Plus, RefreshCw, Save, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  exportMcpServersSnippet,
  getMcpServers,
  resetMcpServers,
  updateMcpServers,
} from "@/lib/backend";
import type { McpServerConfig } from "@/lib/types";
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
import { useWorkspaceStore } from "@/stores/workspaceStore";

const EXAMPLE_FILESYSTEM_ID = "example-filesystem";

function createEmptyServer(): McpServerConfig {
  return {
    id: globalThis.crypto?.randomUUID?.() ?? `mcp-${Date.now()}`,
    name: "",
    command: "",
    args: [],
    env: [],
    enabled: false,
    notes: null,
    scope: "all",
    workspace_ids: [],
  };
}

function localizeExampleServers(
  servers: McpServerConfig[],
  t: (key: string) => string,
): McpServerConfig[] {
  return servers.map((server) => {
    if (server.id !== EXAMPLE_FILESYSTEM_ID) {
      return server;
    }
    return {
      ...server,
      name: t("mcp.exampleFilesystem.name"),
      notes: t("mcp.exampleFilesystem.notes"),
    };
  });
}

export function McpSettingsTab() {
  const { t } = useTranslation("settings");
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const [servers, setServers] = useState<McpServerConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [snippet, setSnippet] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setLoading(true);
      setError(null);
      try {
        const doc = await getMcpServers();
        if (!cancelled) {
          setServers(localizeExampleServers(doc.servers, t));
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [t]);

  const updateServer = (id: string, patch: Partial<McpServerConfig>) => {
    setServers((current) =>
      current.map((server) => (server.id === id ? { ...server, ...patch } : server)),
    );
  };

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    setMessage(null);
    try {
      const doc = await updateMcpServers({ servers });
      setServers(localizeExampleServers(doc.servers, t));
      setMessage(t("mcp.messages.saved"));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleReset = async () => {
    setSaving(true);
    setError(null);
    setMessage(null);
    try {
      const doc = await resetMcpServers();
      setServers(localizeExampleServers(doc.servers, t));
      setMessage(t("mcp.messages.reset"));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleExport = async () => {
    setError(null);
    try {
      const text = await exportMcpServersSnippet();
      setSnippet(text);
      await navigator.clipboard?.writeText(text);
      setMessage(t("mcp.messages.exported"));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const addPlaywrightPreset = () => {
    setError(null);
    setMessage(null);
    const exists = servers.some(
      (server) =>
        server.name.trim().toLowerCase() === "playwright" ||
        server.args.some((arg) => arg.includes("@playwright/mcp")),
    );
    if (exists) {
      setMessage(t("mcp.messages.playwrightExists"));
      return;
    }
    setServers((current) => [
      ...current,
      {
        id: globalThis.crypto?.randomUUID?.() ?? `mcp-playwright-${Date.now()}`,
        name: "playwright",
        command: "npx",
        args: ["@playwright/mcp@latest"],
        env: [],
        enabled: false,
        notes: t("mcp.playwright.notes"),
        scope: "all",
        workspace_ids: [],
      },
    ]);
    setMessage(t("mcp.messages.playwrightAdded"));
  };

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("mcp.states.loading")}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="space-y-2 rounded-lg border border-border bg-card p-4">
        <h3 className="text-sm font-medium">{t("mcp.title")}</h3>
        <p className="text-xs text-muted-foreground">{t("mcp.description")}</p>
        <p className="text-xs text-muted-foreground">{t("mcp.browserCapability")}</p>
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            onClick={() => setServers((current) => [...current, createEmptyServer()])}
          >
            <Plus className="h-4 w-4" />
            {t("mcp.actions.addServer")}
          </Button>
          <Button size="sm" variant="outline" onClick={() => addPlaywrightPreset()}>
            {t("mcp.actions.addPlaywright")}
          </Button>
          <Button size="sm" variant="outline" onClick={() => void handleSave()} disabled={saving}>
            {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save className="h-4 w-4" />}
            {t("mcp.actions.save")}
          </Button>
          <Button size="sm" variant="outline" onClick={() => void handleExport()}>
            {t("mcp.actions.exportSnippet")}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => void handleReset()} disabled={saving}>
            <RefreshCw className="h-4 w-4" />
            {t("mcp.actions.resetExample")}
          </Button>
        </div>
        {message ? <p className="text-xs text-green-700 dark:text-green-300">{message}</p> : null}
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </div>

      <div className="space-y-3">
        {servers.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("mcp.states.empty")}</p>
        ) : null}
        {servers.map((server) => (
          <div key={server.id} className="space-y-3 rounded-lg border border-border bg-card p-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={server.enabled}
                  onChange={(event) => updateServer(server.id, { enabled: event.target.checked })}
                />
                {t("mcp.fields.enabled")}
              </label>
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  setServers((current) => current.filter((item) => item.id !== server.id))
                }
              >
                <Trash2 className="h-4 w-4" />
                {t("mcp.actions.delete")}
              </Button>
            </div>
            <div className="grid gap-2 sm:grid-cols-2">
              <Input
                placeholder={t("mcp.fields.namePlaceholder")}
                value={server.name}
                onChange={(event) => updateServer(server.id, { name: event.target.value })}
              />
              <Input
                placeholder={t("mcp.fields.commandPlaceholder")}
                value={server.command}
                onChange={(event) => updateServer(server.id, { command: event.target.value })}
              />
            </div>
            <Input
              placeholder={t("mcp.fields.argsPlaceholder")}
              value={server.args.join(" ")}
              onChange={(event) =>
                updateServer(server.id, {
                  args: event.target.value
                    .split(/\s+/)
                    .map((part) => part.trim())
                    .filter(Boolean),
                })
              }
            />
            <div>
              <label className="text-xs font-medium text-muted-foreground">
                {t("mcp.fields.scope")}
              </label>
              <Select
                value={server.scope}
                disabled={saving}
                onValueChange={(value) => {
                  if (value === "all" || value === "workspaces") {
                    updateServer(server.id, {
                      scope: value,
                      workspace_ids: value === "all" ? [] : server.workspace_ids,
                    });
                  }
                }}
              >
                <SelectTrigger className="mt-1 bg-background">
                  <SelectValue>
                    {(value) =>
                      value === "workspaces"
                        ? t("mcp.fields.scopeWorkspaces")
                        : t("mcp.fields.scopeAll")
                    }
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">{t("mcp.fields.scopeAll")}</SelectItem>
                  <SelectItem value="workspaces">{t("mcp.fields.scopeWorkspaces")}</SelectItem>
                </SelectContent>
              </Select>
              <p className="mt-1 text-xs text-muted-foreground">{t("mcp.fields.scopeHint")}</p>
            </div>
            {server.scope === "workspaces" ? (
              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("mcp.fields.scopePickWorkspaces")}
                </label>
                <div className="mt-1 max-h-48 space-y-2 overflow-y-auto rounded-md border border-border p-3">
                  {workspaces.length === 0 ? (
                    <p className="text-xs text-muted-foreground">
                      {t("mcp.fields.scopeWorkspacesEmpty")}
                    </p>
                  ) : (
                    workspaces.map((workspace) => (
                      <label key={workspace.id} className="flex items-center gap-2 text-sm">
                        <input
                          type="checkbox"
                          className="h-4 w-4 rounded border-input"
                          checked={server.workspace_ids.includes(workspace.id)}
                          disabled={saving}
                          onChange={(event) => {
                            const workspace_ids = event.target.checked
                              ? [
                                  ...server.workspace_ids.filter((id) => id !== workspace.id),
                                  workspace.id,
                                ]
                              : server.workspace_ids.filter((id) => id !== workspace.id);
                            updateServer(server.id, { workspace_ids });
                          }}
                        />
                        {workspace.name}
                      </label>
                    ))
                  )}
                </div>
              </div>
            ) : null}
            <Textarea
              placeholder={t("mcp.fields.notesPlaceholder")}
              value={server.notes ?? ""}
              onChange={(event) => updateServer(server.id, { notes: event.target.value })}
              rows={2}
            />
          </div>
        ))}
      </div>

      {snippet ? (
        <div className="rounded-lg border border-border bg-muted/30 p-3">
          <p className="mb-2 text-xs font-medium">{t("mcp.preview.title")}</p>
          <pre className="max-h-64 overflow-auto whitespace-pre-wrap font-mono text-[11px]">
            {snippet}
          </pre>
        </div>
      ) : null}
    </div>
  );
}
