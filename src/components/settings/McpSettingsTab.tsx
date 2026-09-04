import { useEffect, useMemo, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  Blocks,
  Download,
  Globe,
  KeyRound,
  Loader2,
  Pencil,
  Plus,
  Radio,
  RefreshCw,
  Terminal,
  Trash2,
} from "lucide-react";
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
    enabled: true,
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

function getTransportIcon(transport: McpTransport) {
  switch (transport) {
    case "http":
      return Globe;
    case "sse":
      return Radio;
    case "stdio":
    default:
      return Terminal;
  }
}

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

interface McpFormState {
  name: string;
  transport: McpTransport;
  command: string;
  argsText: string;
  envText: string;
  url: string;
  headersText: string;
  oauth: McpOAuthConfig | null;
  scope: "all" | "workspaces";
  workspace_ids: string[];
  notes: string;
  enabled: boolean;
}

const EMPTY_FORM: McpFormState = {
  name: "",
  transport: "stdio",
  command: "",
  argsText: "",
  envText: "",
  url: "",
  headersText: "",
  oauth: null,
  scope: "all",
  workspace_ids: [],
  notes: "",
  enabled: true,
};

function serverToForm(server: McpServerConfig): McpFormState {
  return {
    name: server.name,
    transport: server.transport ?? "stdio",
    command: server.command ?? "",
    argsText: (server.args ?? []).join(" "),
    envText: formatKeyValueLines(server.env ?? []),
    url: server.url ?? "",
    headersText: formatKeyValueLines(server.headers ?? []),
    oauth: server.oauth ?? null,
    scope: server.scope ?? "all",
    workspace_ids: server.workspace_ids ?? [],
    notes: server.notes ?? "",
    enabled: server.enabled ?? true,
  };
}

function McpOAuthPanel({
  serverId,
  oauth,
  saving,
  canAuthorize,
  onChange,
}: {
  serverId: string;
  oauth: McpOAuthConfig | null;
  saving: boolean;
  canAuthorize: boolean;
  onChange: (oauth: McpOAuthConfig | null) => void;
}) {
  const { t } = useTranslation("settings");
  const [authorized, setAuthorized] = useState<boolean | null>(null);
  const [expiresAt, setExpiresAt] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = async () => {
    if (!canAuthorize || !serverId) return;
    try {
      const status = await getMcpOAuthStatus(serverId);
      setAuthorized(status.authorized);
      setExpiresAt(status.expiresAt);
    } catch {
      setAuthorized(null);
    }
  };

  useEffect(() => {
    if (!oauth || !canAuthorize || !serverId) return;
    void refresh();
    let unlisten: (() => void) | undefined;
    void onNativeMcpOAuth((event) => {
      if (event.serverId !== serverId) return;
      setNotice(event.message);
      setBusy(false);
      void refresh();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverId, Boolean(oauth), canAuthorize]);

  if (!oauth) {
    return (
      <Button
        size="sm"
        variant="outline"
        disabled={saving}
        onClick={() => onChange(emptyOAuth())}
        className="h-8 text-xs gap-1.5"
      >
        <KeyRound className="size-3.5" />
        {t("mcp.oauth.enable")}
      </Button>
    );
  }

  const patch = (next: Partial<McpOAuthConfig>) => onChange({ ...oauth, ...next });

  return (
    <div className="space-y-3 rounded-xl border border-border/70 bg-muted/20 p-3.5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-xs font-semibold text-foreground tracking-tight">
          {t("mcp.oauth.title")}
        </p>
        <Button
          size="sm"
          variant="ghost"
          disabled={saving}
          onClick={() => onChange(null)}
          className="h-7 text-xs text-muted-foreground hover:text-destructive"
        >
          {t("mcp.oauth.disable")}
        </Button>
      </div>
      <div className="grid gap-2.5 sm:grid-cols-2">
        <div>
          <label className="text-[11px] font-medium text-muted-foreground">
            {t("mcp.oauth.clientId")}
          </label>
          <Input
            className="mt-1 h-8 text-xs font-mono"
            placeholder={t("mcp.oauth.clientId")}
            value={oauth.client_id}
            onChange={(event) => patch({ client_id: event.target.value })}
            disabled={saving}
          />
        </div>
        <div>
          <label className="text-[11px] font-medium text-muted-foreground">
            {t("mcp.oauth.clientSecret")}
          </label>
          <Input
            className="mt-1 h-8 text-xs font-mono"
            placeholder={t("mcp.oauth.clientSecret")}
            type="password"
            value={oauth.client_secret ?? ""}
            onChange={(event) => patch({ client_secret: event.target.value || null })}
            disabled={saving}
          />
        </div>
        <div>
          <label className="text-[11px] font-medium text-muted-foreground">
            {t("mcp.oauth.authorizeUrl")}
          </label>
          <Input
            className="mt-1 h-8 text-xs font-mono"
            placeholder={t("mcp.oauth.authorizeUrl")}
            value={oauth.authorize_url}
            onChange={(event) => patch({ authorize_url: event.target.value })}
            disabled={saving}
          />
        </div>
        <div>
          <label className="text-[11px] font-medium text-muted-foreground">
            {t("mcp.oauth.tokenUrl")}
          </label>
          <Input
            className="mt-1 h-8 text-xs font-mono"
            placeholder={t("mcp.oauth.tokenUrl")}
            value={oauth.token_url}
            onChange={(event) => patch({ token_url: event.target.value })}
            disabled={saving}
          />
        </div>
      </div>
      <div>
        <label className="text-[11px] font-medium text-muted-foreground">
          {t("mcp.oauth.scopes")}
        </label>
        <Input
          className="mt-1 h-8 text-xs font-mono"
          placeholder={t("mcp.oauth.scopes")}
          value={oauth.scopes.join(" ")}
          onChange={(event) =>
            patch({ scopes: event.target.value.split(/[\s,]+/).filter(Boolean) })
          }
          disabled={saving}
        />
      </div>
      <div className="flex flex-wrap items-center gap-2 pt-1 text-xs">
        <Button
          size="sm"
          variant="outline"
          disabled={busy || saving || !canAuthorize}
          className="h-7 text-xs gap-1"
          onClick={async () => {
            setBusy(true);
            setNotice(null);
            try {
              const started = await startMcpOAuth(serverId);
              setNotice(t("mcp.oauth.browserOpened", { url: started.authorizeUrl }));
            } catch (err) {
              setBusy(false);
              setNotice(err instanceof Error ? err.message : String(err));
            }
          }}
        >
          {busy ? <Loader2 className="size-3 animate-spin" /> : <KeyRound className="size-3" />}
          {t("mcp.oauth.authorize")}
        </Button>
        {authorized && canAuthorize ? (
          <Button
            size="sm"
            variant="ghost"
            disabled={saving}
            className="h-7 text-xs"
            onClick={async () => {
              try {
                await clearMcpOAuth(serverId);
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
        <span className="text-[11px] text-muted-foreground">
          {!canAuthorize
            ? "保存服务器后可进行 OAuth 授权"
            : authorized === null
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
      <p className="text-[11px] text-muted-foreground">{t("mcp.oauth.hint")}</p>
      {notice ? <p className="break-all text-[11px] text-muted-foreground">{notice}</p> : null}
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
  const [saving, setSaving] = useState<"save" | "delete" | "reset" | null>(null);
  const [deleteConfirming, setDeleteConfirming] = useState(false);
  const [togglingId, setTogglingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [modalError, setModalError] = useState<string | null>(null);
  const [snippet, setSnippet] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [form, setForm] = useState<McpFormState>(EMPTY_FORM);
  const [dialogOpen, setDialogOpen] = useState(false);

  const selected = useMemo(
    () => servers.find((server) => server.id === selectedId) ?? null,
    [servers, selectedId],
  );
  const isCreate = selectedId === null;
  const formLocked = saving !== null || deleteConfirming;

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

  const patchForm = (patch: Partial<McpFormState>) => {
    setForm((current) => ({ ...current, ...patch }));
  };

  const openCreate = (preset?: Partial<McpServerConfig>) => {
    setSelectedId(null);
    if (preset) {
      setForm({
        ...EMPTY_FORM,
        ...serverToForm({
          ...createEmptyServer(),
          ...preset,
        }),
      });
    } else {
      setForm(EMPTY_FORM);
    }
    setModalError(null);
    setDialogOpen(true);
  };

  const openEdit = (server: McpServerConfig) => {
    setSelectedId(server.id);
    setForm(serverToForm(server));
    setModalError(null);
    setDialogOpen(true);
  };

  const closeDialog = () => {
    if (formLocked) return;
    setDialogOpen(false);
    setSelectedId(null);
    setForm(EMPTY_FORM);
    setModalError(null);
  };

  const handleSave = async () => {
    if (!form.name.trim()) {
      setModalError(t("mcp.validation.nameRequired"));
      return;
    }
    if (form.transport === "stdio") {
      if (!form.command.trim()) {
        setModalError(t("mcp.validation.commandRequired"));
        return;
      }
    } else {
      if (!form.url.trim()) {
        setModalError(t("mcp.validation.urlRequired"));
        return;
      }
      if (!form.url.trim().startsWith("http://") && !form.url.trim().startsWith("https://")) {
        setModalError(t("mcp.validation.urlInvalid"));
        return;
      }
      if (form.oauth) {
        if (
          !form.oauth.client_id.trim() ||
          !form.oauth.authorize_url.trim() ||
          !form.oauth.token_url.trim()
        ) {
          setModalError(t("mcp.validation.oauthIncomplete"));
          return;
        }
      }
    }

    setSaving("save");
    setModalError(null);

    const targetId = isCreate
      ? (globalThis.crypto?.randomUUID?.() ?? `mcp-${Date.now()}`)
      : selected!.id;

    const newServer: McpServerConfig = {
      id: targetId,
      name: form.name.trim(),
      transport: form.transport,
      command: form.command.trim(),
      args: form.argsText
        .split(/\s+/)
        .map((part) => part.trim())
        .filter(Boolean),
      env: parseKeyValueLines(form.envText),
      url: form.url.trim() || null,
      headers: parseKeyValueLines(form.headersText),
      oauth: form.oauth,
      scope: form.scope,
      workspace_ids: form.scope === "workspaces" ? form.workspace_ids : [],
      notes: form.notes.trim() || null,
      enabled: form.enabled,
    };

    const nextServers = isCreate
      ? [...servers, newServer]
      : servers.map((item) => (item.id === targetId ? newServer : item));

    try {
      const doc = await updateMcpServers({ servers: nextServers });
      setServers(localizeExampleServers(doc.servers, t));
      setDialogOpen(false);
      setSelectedId(null);
      setForm(EMPTY_FORM);
      setMessage(isCreate ? t("mcp.messages.created") : t("mcp.messages.updated"));
      setError(null);
    } catch (err) {
      setModalError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleDelete = async () => {
    if (!selected || formLocked) return;
    const targetId = selected.id;
    const targetName = selected.name;
    setDeleteConfirming(true);
    setModalError(null);
    try {
      const confirmed = await confirm(t("mcp.dialogs.deleteConfirm", { name: targetName }), {
        title: t("mcp.dialogs.deleteTitle"),
        kind: "warning",
      });
      if (!confirmed) return;
      setSaving("delete");
      const nextServers = servers.filter((item) => item.id !== targetId);
      const doc = await updateMcpServers({ servers: nextServers });
      setServers(localizeExampleServers(doc.servers, t));
      setSelectedId(null);
      setForm(EMPTY_FORM);
      setDialogOpen(false);
      setMessage(t("mcp.messages.deleted"));
      setError(null);
    } catch (err) {
      setModalError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeleteConfirming(false);
      setSaving(null);
    }
  };

  const handleToggleEnabled = async (server: McpServerConfig, enabled: boolean) => {
    if (togglingId !== null) return;
    setTogglingId(server.id);
    setError(null);
    const nextServers = servers.map((item) =>
      item.id === server.id ? { ...item, enabled } : item,
    );
    try {
      const doc = await updateMcpServers({ servers: nextServers });
      setServers(localizeExampleServers(doc.servers, t));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setTogglingId(null);
    }
  };

  const handleReset = async () => {
    if (formLocked) return;
    setError(null);
    setMessage(null);
    try {
      const confirmed = await confirm(t("mcp.dialogs.resetConfirm"), {
        title: t("mcp.dialogs.resetTitle"),
        kind: "warning",
      });
      if (!confirmed) return;
      setSaving("reset");
      const doc = await resetMcpServers();
      setServers(localizeExampleServers(doc.servers, t));
      setMessage(t("mcp.messages.reset"));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
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
    openCreate({
      name: "playwright",
      command: "npx",
      args: ["@playwright/mcp@latest"],
      notes: t("mcp.playwright.notes"),
      enabled: true,
    });
  };

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
              variant="outline"
              onClick={() => void handleReset()}
              disabled={formLocked}
              className="h-7 text-xs gap-1"
            >
              {saving === "reset" ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <RefreshCw className="size-3" />
              )}
              {t("mcp.actions.resetExample")}
            </Button>
            <Button size="sm" onClick={() => openCreate()} className="h-7 text-xs gap-1">
              <Plus className="size-3" />
              {t("mcp.actions.addServer")}
            </Button>
          </div>
        }
      >
        {loading ? (
          <div className="flex h-36 items-center justify-center text-xs text-muted-foreground">
            <Loader2 className="mr-2 size-4 animate-spin text-primary" />
            {t("mcp.states.loading")}
          </div>
        ) : servers.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12 text-center">
            <div className="flex size-12 items-center justify-center rounded-2xl border border-border/70 bg-muted/30 text-muted-foreground shadow-2xs">
              <Blocks className="size-6" />
            </div>
            <p className="mt-3 text-xs font-semibold text-foreground">{t("mcp.states.empty")}</p>
            <p className="mt-1 text-[11px] text-muted-foreground max-w-sm">
              {t("mcp.description")}
            </p>
            <Button size="sm" onClick={() => openCreate()} className="mt-4 h-7 gap-1 text-xs">
              <Plus className="size-3.5" />
              {t("mcp.actions.addServer")}
            </Button>
          </div>
        ) : (
          <div className="grid gap-3 sm:grid-cols-1">
            {servers.map((server) => {
              const Icon = getTransportIcon(server.transport);
              const isToggling = togglingId === server.id;

              return (
                <div
                  key={server.id}
                  className="group relative flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all duration-150 hover:border-border hover:shadow-xs"
                >
                  <div className="flex items-start gap-3 min-w-0">
                    <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                      <Icon className="size-4" />
                    </div>
                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-xs font-semibold tracking-tight text-foreground truncate">
                          {server.name || t("mcp.fields.namePlaceholder")}
                        </span>
                        {/* 状态指示灯 */}
                        <div className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-muted/40 px-2 py-0.5 text-[10px] font-mono">
                          <span
                            className={`size-1.5 rounded-full ${
                              server.enabled
                                ? "bg-emerald-500 shadow-2xs shadow-emerald-500/50"
                                : "bg-muted-foreground/40"
                            }`}
                          />
                          <span className="text-muted-foreground">
                            {server.enabled ? t("mcp.status.enabled") : t("mcp.status.disabled")}
                          </span>
                        </div>
                        {/* 传输协议 Badge */}
                        <span className="rounded-md border border-border/50 bg-background px-1.5 py-0.2 text-[10px] font-mono text-muted-foreground uppercase">
                          {server.transport ?? "stdio"}
                        </span>
                        {/* 作用域 Badge */}
                        <span className="rounded-md bg-muted px-1.5 py-0.2 text-[10px] font-mono text-muted-foreground">
                          {server.scope === "workspaces"
                            ? `${server.workspace_ids.length} 个工作区`
                            : t("mcp.fields.scopeAll")}
                        </span>
                      </div>
                      <p className="text-[11px] font-mono text-muted-foreground/80 truncate max-w-md">
                        {server.transport === "http" || server.transport === "sse"
                          ? server.url || "-"
                          : `${server.command} ${server.args.join(" ")}`.trim() || "-"}
                      </p>
                      {server.notes ? (
                        <p className="text-[11px] text-muted-foreground/60 truncate max-w-md">
                          {server.notes}
                        </p>
                      ) : null}
                    </div>
                  </div>

                  {/* 快捷操作 */}
                  <div className="flex items-center gap-3 shrink-0 self-end sm:self-center">
                    <div className="flex items-center gap-1.5">
                      <Switch
                        id={`mcp-toggle-${server.id}`}
                        checked={server.enabled}
                        disabled={isToggling}
                        onCheckedChange={(checked) => void handleToggleEnabled(server, checked)}
                        title={server.enabled ? t("mcp.status.enabled") : t("mcp.status.disabled")}
                      />
                    </div>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => openEdit(server)}
                      className="h-7 text-xs gap-1"
                    >
                      <Pencil className="size-3" />
                      {t("mcp.actions.edit")}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </SettingCard>

      {snippet ? (
        <div className="rounded-xl border border-border/70 bg-muted/20 p-4 shadow-2xs">
          <p className="mb-2 text-xs font-semibold text-foreground">{t("mcp.preview.title")}</p>
          <pre className="max-h-64 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-muted-foreground">
            {snippet}
          </pre>
        </div>
      ) : null}

      {/* 新建/编辑 MCP 服务器 Modal */}
      <Dialog
        open={dialogOpen}
        onOpenChange={(open) => {
          if (!open) closeDialog();
        }}
      >
        <DialogContent
          className="flex max-h-[90vh] w-full flex-col gap-0 overflow-hidden sm:max-w-2xl rounded-2xl p-0"
          showCloseButton={!formLocked}
        >
          <DialogHeader className="shrink-0 border-b border-border/50 px-6 py-4">
            <DialogTitle className="text-base font-semibold tracking-tight">
              {isCreate ? t("mcp.dialogs.createTitle") : t("mcp.dialogs.editTitle")}
            </DialogTitle>
            <DialogDescription className="text-xs text-muted-foreground">
              {t("mcp.description")}
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
            <div className="space-y-4">
              {modalError ? <SettingFeedbackCallout variant="error" message={modalError} /> : null}

              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("mcp.fields.name")}
                  </label>
                  <Input
                    className="mt-1 h-8 text-xs"
                    value={form.name}
                    onChange={(event) => patchForm({ name: event.target.value })}
                    placeholder={t("mcp.fields.namePlaceholder")}
                    disabled={formLocked}
                  />
                </div>
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("mcp.fields.transport")}
                  </label>
                  <Select
                    value={form.transport}
                    disabled={formLocked}
                    onValueChange={(value) => {
                      if (TRANSPORTS.includes(value as McpTransport)) {
                        patchForm({ transport: value as McpTransport });
                      }
                    }}
                  >
                    <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                      <SelectValue>
                        {(value) => t(`mcp.transport.${String(value || "stdio")}`)}
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      {TRANSPORTS.map((transport) => (
                        <SelectItem key={transport} value={transport} className="text-xs">
                          {t(`mcp.transport.${transport}`)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {form.transport === "http" || form.transport === "sse" ? (
                <>
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("mcp.fields.url")}
                    </label>
                    <Input
                      className="mt-1 h-8 text-xs font-mono"
                      placeholder={t("mcp.fields.urlPlaceholder")}
                      value={form.url}
                      onChange={(event) => patchForm({ url: event.target.value })}
                      disabled={formLocked}
                    />
                  </div>
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("mcp.fields.headers")}
                    </label>
                    <Textarea
                      className="mt-1 resize-none font-mono text-xs leading-relaxed"
                      placeholder={t("mcp.fields.headersPlaceholder")}
                      value={form.headersText}
                      onChange={(event) => patchForm({ headersText: event.target.value })}
                      rows={2}
                      disabled={formLocked}
                    />
                  </div>
                  <McpOAuthPanel
                    serverId={selected?.id ?? ""}
                    oauth={form.oauth}
                    saving={formLocked}
                    canAuthorize={!isCreate}
                    onChange={(oauth) => patchForm({ oauth })}
                  />
                </>
              ) : (
                <>
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("mcp.fields.command")}
                    </label>
                    <Input
                      className="mt-1 h-8 text-xs font-mono"
                      placeholder={t("mcp.fields.commandPlaceholder")}
                      value={form.command}
                      onChange={(event) => patchForm({ command: event.target.value })}
                      disabled={formLocked}
                    />
                  </div>
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("mcp.fields.args")}
                    </label>
                    <Input
                      className="mt-1 h-8 text-xs font-mono"
                      placeholder={t("mcp.fields.argsPlaceholder")}
                      value={form.argsText}
                      onChange={(event) => patchForm({ argsText: event.target.value })}
                      disabled={formLocked}
                    />
                  </div>
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("mcp.fields.env")}
                    </label>
                    <Textarea
                      className="mt-1 resize-none font-mono text-xs leading-relaxed"
                      placeholder={t("mcp.fields.envPlaceholder")}
                      value={form.envText}
                      onChange={(event) => patchForm({ envText: event.target.value })}
                      rows={3}
                      disabled={formLocked}
                    />
                  </div>
                </>
              )}

              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("mcp.fields.scope")}
                  </label>
                  <Select
                    value={form.scope}
                    disabled={formLocked}
                    onValueChange={(value) => {
                      if (value === "all" || value === "workspaces") {
                        patchForm({
                          scope: value,
                          workspace_ids: value === "all" ? [] : form.workspace_ids,
                        });
                      }
                    }}
                  >
                    <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                      <SelectValue>
                        {(value) =>
                          value === "workspaces"
                            ? t("mcp.fields.scopeWorkspaces")
                            : t("mcp.fields.scopeAll")
                        }
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="all" className="text-xs">
                        {t("mcp.fields.scopeAll")}
                      </SelectItem>
                      <SelectItem value="workspaces" className="text-xs">
                        {t("mcp.fields.scopeWorkspaces")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    {t("mcp.fields.scopeHint")}
                  </p>
                </div>

                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("mcp.fields.enabled")}
                  </label>
                  <Select
                    value={form.enabled ? "enabled" : "disabled"}
                    disabled={formLocked}
                    onValueChange={(value) => patchForm({ enabled: value === "enabled" })}
                  >
                    <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="enabled" className="text-xs">
                        {t("mcp.status.enabled")}
                      </SelectItem>
                      <SelectItem value="disabled" className="text-xs">
                        {t("mcp.status.disabled")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {form.scope === "workspaces" ? (
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("mcp.fields.scopePickWorkspaces")}
                  </label>
                  <div className="mt-1 max-h-36 space-y-2 overflow-y-auto rounded-xl border border-border/70 bg-muted/20 p-3">
                    {workspaces.length === 0 ? (
                      <p className="text-xs text-muted-foreground">
                        {t("mcp.fields.scopeWorkspacesEmpty")}
                      </p>
                    ) : (
                      workspaces.map((workspace) => (
                        <label
                          key={workspace.id}
                          className="flex items-center gap-2 text-xs text-foreground cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            className="size-3.5 rounded border-input text-primary focus:ring-1"
                            checked={form.workspace_ids.includes(workspace.id)}
                            disabled={formLocked}
                            onChange={(event) => {
                              const workspace_ids = event.target.checked
                                ? [
                                    ...form.workspace_ids.filter((id) => id !== workspace.id),
                                    workspace.id,
                                  ]
                                : form.workspace_ids.filter((id) => id !== workspace.id);
                              patchForm({ workspace_ids });
                            }}
                          />
                          <span className="truncate">{workspace.name}</span>
                        </label>
                      ))
                    )}
                  </div>
                </div>
              ) : null}

              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("mcp.fields.notes")}
                </label>
                <Textarea
                  className="mt-1 resize-none text-xs leading-relaxed"
                  placeholder={t("mcp.fields.notesPlaceholder")}
                  value={form.notes}
                  onChange={(event) => patchForm({ notes: event.target.value })}
                  rows={2}
                  disabled={formLocked}
                />
              </div>
            </div>
          </div>

          <DialogFooter className="m-0 shrink-0 border-t border-border/50 bg-muted/10 px-6 py-4">
            {!isCreate ? (
              <Button
                variant="destructive"
                size="sm"
                className="sm:mr-auto h-8 text-xs"
                onClick={() => void handleDelete()}
                disabled={formLocked}
              >
                {saving === "delete" ? (
                  <Loader2 className="mr-1 size-3.5 animate-spin" />
                ) : (
                  <Trash2 className="mr-1 size-3.5" />
                )}
                {t("mcp.actions.delete")}
              </Button>
            ) : null}
            <Button
              variant="outline"
              size="sm"
              className="h-8 text-xs"
              onClick={closeDialog}
              disabled={formLocked}
            >
              {t("mcp.actions.cancel")}
            </Button>
            <Button
              size="sm"
              className="h-8 text-xs"
              onClick={() => void handleSave()}
              disabled={formLocked}
            >
              {saving === "save" ? <Loader2 className="mr-1 size-3.5 animate-spin" /> : null}
              {isCreate ? t("mcp.actions.create", { defaultValue: "创建" }) : t("mcp.actions.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
