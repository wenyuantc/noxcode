import { useEffect, useState } from "react";
import { Blocks, Download, KeyRound, Loader2, Plus, RefreshCw, Save, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  clearMcpOAuth,
  exportMcpServersSnippet,
  getMcpOAuthStatus,
  getMcpServers,
  onNativeMcpOAuth,
  resetMcpServers,
  startMcpOAuth,
  updateMcpServers,
} from "@/lib/backend";
import type { McpEnvVar, McpOAuthConfig, McpServerConfig, McpTransport } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

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
    transport: "stdio",
    url: null,
    headers: [],
    oauth: null,
  };
}

const TRANSPORTS: McpTransport[] = ["stdio", "http", "sse"];

/** `KEY=value` 每行一条 ↔ 键值数组。 */
export function parseKeyValueLines(text: string): McpEnvVar[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const index = line.indexOf("=");
      if (index <= 0) return { key: line, value: "" };
      return { key: line.slice(0, index).trim(), value: line.slice(index + 1).trim() };
    })
    .filter((pair) => pair.key.length > 0);
}

export function formatKeyValueLines(pairs: McpEnvVar[]): string {
  return pairs.map((pair) => `${pair.key}=${pair.value}`).join("\n");
}

function emptyOAuth(): McpOAuthConfig {
  return { client_id: "", client_secret: null, authorize_url: "", token_url: "", scopes: [] };
}

function McpOAuthPanel({
  server,
  saving,
  onChange,
}: {
  server: McpServerConfig;
  saving: boolean;
  onChange: (oauth: McpOAuthConfig | null) => void;
}) {
  const { t } = useTranslation("settings");
  const [authorized, setAuthorized] = useState<boolean | null>(null);
  const [expiresAt, setExpiresAt] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const oauth = server.oauth;

  const refresh = async () => {
    try {
      const status = await getMcpOAuthStatus(server.id);
      setAuthorized(status.authorized);
      setExpiresAt(status.expiresAt);
    } catch {
      setAuthorized(null);
    }
  };

  useEffect(() => {
    if (!oauth) return;
    void refresh();
    let unlisten: (() => void) | undefined;
    void onNativeMcpOAuth((event) => {
      if (event.serverId !== server.id) return;
      setNotice(event.message);
      setBusy(false);
      void refresh();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server.id, Boolean(oauth)]);

  if (!oauth) {
    return (
      <Button size="sm" variant="outline" disabled={saving} onClick={() => onChange(emptyOAuth())}>
        <KeyRound className="h-4 w-4" />
        {t("mcp.oauth.enable")}
      </Button>
    );
  }

  const patch = (next: Partial<McpOAuthConfig>) => onChange({ ...oauth, ...next });

  return (
    <div className="space-y-2 rounded-md border border-border p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-xs font-medium">{t("mcp.oauth.title")}</p>
        <Button size="sm" variant="ghost" disabled={saving} onClick={() => onChange(null)}>
          {t("mcp.oauth.disable")}
        </Button>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <Input
          placeholder={t("mcp.oauth.clientId")}
          value={oauth.client_id}
          onChange={(event) => patch({ client_id: event.target.value })}
        />
        <Input
          placeholder={t("mcp.oauth.clientSecret")}
          type="password"
          value={oauth.client_secret ?? ""}
          onChange={(event) => patch({ client_secret: event.target.value || null })}
        />
        <Input
          placeholder={t("mcp.oauth.authorizeUrl")}
          value={oauth.authorize_url}
          onChange={(event) => patch({ authorize_url: event.target.value })}
        />
        <Input
          placeholder={t("mcp.oauth.tokenUrl")}
          value={oauth.token_url}
          onChange={(event) => patch({ token_url: event.target.value })}
        />
      </div>
      <Input
        placeholder={t("mcp.oauth.scopes")}
        value={oauth.scopes.join(" ")}
        onChange={(event) => patch({ scopes: event.target.value.split(/[\s,]+/).filter(Boolean) })}
      />
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <Button
          size="sm"
          variant="outline"
          disabled={busy || saving}
          onClick={async () => {
            setBusy(true);
            setNotice(null);
            try {
              const started = await startMcpOAuth(server.id);
              setNotice(t("mcp.oauth.browserOpened", { url: started.authorizeUrl }));
            } catch (err) {
              setBusy(false);
              setNotice(err instanceof Error ? err.message : String(err));
            }
          }}
        >
          {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <KeyRound className="h-4 w-4" />}
          {t("mcp.oauth.authorize")}
        </Button>
        {authorized ? (
          <Button
            size="sm"
            variant="ghost"
            disabled={saving}
            onClick={async () => {
              try {
                await clearMcpOAuth(server.id);
                setNotice(t("mcp.oauth.cleared"));
                await refresh();
              } catch (err) {
                setNotice(err instanceof Error ? err.message : String(err));
              }
            }}
          >
            {t("mcp.oauth.clear")}
          </Button>
        ) : null}
        <span className="text-muted-foreground">
          {authorized === null
            ? t("mcp.oauth.statusUnknown")
            : authorized
              ? expiresAt
                ? t("mcp.oauth.statusAuthorizedUntil", {
                    time: new Date(expiresAt * 1000).toLocaleString(),
                  })
                : t("mcp.oauth.statusAuthorized")
              : t("mcp.oauth.statusMissing")}
        </span>
      </div>
      <p className="text-xs text-muted-foreground">{t("mcp.oauth.hint")}</p>
      {notice ? <p className="break-all text-xs text-muted-foreground">{notice}</p> : null}
    </div>
  );
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
        transport: "stdio",
        url: null,
        headers: [],
        oauth: null,
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
    <div className="space-y-6">
      {message ? (
        <SettingFeedbackCallout
          variant="success"
          message={message}
          onClose={() => setMessage(null)}
        />
      ) : null}
      {error ? (
        <SettingFeedbackCallout variant="error" message={error} onClose={() => setError(null)} />
      ) : null}

      <SettingCard
        icon={Blocks}
        title={t("mcp.title")}
        description={t("mcp.description")}
        badge={`${servers.length} 个服务`}
        headerAction={
          <div className="flex flex-wrap items-center gap-1.5">
            <Button
              size="sm"
              variant="outline"
              onClick={() => addPlaywrightPreset()}
              className="h-7 text-xs gap-1"
            >
              + Playwright
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void handleExport()}
              className="h-7 text-xs gap-1"
            >
              <Download className="size-3" />
              {t("mcp.actions.exportSnippet")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void handleReset()}
              disabled={saving}
              className="h-7 text-xs gap-1"
            >
              <RefreshCw className="size-3" />
              {t("mcp.actions.resetExample")}
            </Button>
            <Button
              size="sm"
              onClick={() => void handleSave()}
              disabled={saving}
              className="h-7 text-xs gap-1"
            >
              {saving ? <Loader2 className="size-3 animate-spin" /> : <Save className="size-3" />}
              {t("mcp.actions.save")}
            </Button>
            <Button
              size="sm"
              onClick={() => setServers((current) => [...current, createEmptyServer()])}
              className="h-7 text-xs gap-1"
            >
              <Plus className="size-3" />
              {t("mcp.actions.addServer")}
            </Button>
          </div>
        }
      >
        <div className="space-y-3">
          {servers.length === 0 ? (
            <div className="rounded-xl border border-dashed border-border/80 py-8 text-center text-xs text-muted-foreground">
              {t("mcp.states.empty")}
            </div>
          ) : null}
          {servers.map((server) => (
            <div
              key={server.id}
              className="space-y-3 rounded-xl border border-border/70 bg-card p-4 shadow-2xs transition-all hover:border-border"
            >
              <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/40 pb-3">
                <div className="flex items-center gap-2.5">
                  <Switch
                    id={`mcp-switch-${server.id}`}
                    checked={server.enabled}
                    onCheckedChange={(checked) => updateServer(server.id, { enabled: checked })}
                  />
                  <span className="text-xs font-semibold text-foreground tracking-tight">
                    {server.name || t("mcp.fields.namePlaceholder")}
                  </span>
                  <span className="rounded-md border border-border/60 bg-muted/40 px-1.5 py-0.2 text-[10px] font-mono text-muted-foreground uppercase">
                    {server.transport ?? "stdio"}
                  </span>
                </div>
                <Button
                  size="icon-xs"
                  variant="ghost"
                  className="text-muted-foreground opacity-60 hover:text-destructive hover:opacity-100"
                  onClick={() =>
                    setServers((current) => current.filter((item) => item.id !== server.id))
                  }
                  title={t("mcp.actions.delete")}
                >
                  <Trash2 className="size-3.5" />
                </Button>
              </div>
              <div className="grid gap-2 sm:grid-cols-2">
                <Input
                  placeholder={t("mcp.fields.namePlaceholder")}
                  value={server.name}
                  onChange={(event) => updateServer(server.id, { name: event.target.value })}
                />
                <Select
                  value={server.transport ?? "stdio"}
                  disabled={saving}
                  onValueChange={(value) => {
                    if (TRANSPORTS.includes(value as McpTransport)) {
                      updateServer(server.id, { transport: value as McpTransport });
                    }
                  }}
                >
                  <SelectTrigger className="bg-background">
                    <SelectValue>
                      {(value) => t(`mcp.transport.${String(value || "stdio")}`)}
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {TRANSPORTS.map((transport) => (
                      <SelectItem key={transport} value={transport}>
                        {t(`mcp.transport.${transport}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              {server.transport === "http" || server.transport === "sse" ? (
                <>
                  <Input
                    placeholder={t("mcp.fields.urlPlaceholder")}
                    value={server.url ?? ""}
                    onChange={(event) =>
                      updateServer(server.id, { url: event.target.value || null })
                    }
                  />
                  <Textarea
                    placeholder={t("mcp.fields.headersPlaceholder")}
                    value={formatKeyValueLines(server.headers ?? [])}
                    onChange={(event) =>
                      updateServer(server.id, { headers: parseKeyValueLines(event.target.value) })
                    }
                    rows={2}
                  />
                  <McpOAuthPanel
                    server={server}
                    saving={saving}
                    onChange={(oauth) => updateServer(server.id, { oauth })}
                  />
                </>
              ) : (
                <>
                  <Input
                    placeholder={t("mcp.fields.commandPlaceholder")}
                    value={server.command}
                    onChange={(event) => updateServer(server.id, { command: event.target.value })}
                  />
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
                </>
              )}
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
      </SettingCard>

      {snippet ? (
        <div className="rounded-xl border border-border/70 bg-muted/20 p-4 shadow-2xs">
          <p className="mb-2 text-xs font-semibold text-foreground">{t("mcp.preview.title")}</p>
          <pre className="max-h-64 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-muted-foreground">
            {snippet}
          </pre>
        </div>
      ) : null}
    </div>
  );
}
