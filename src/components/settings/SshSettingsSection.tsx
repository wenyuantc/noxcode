import { confirm, open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Loader2, Plus, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  createSshConfig,
  deleteSshConfig,
  importSshConfigFileHost,
  listSshConfigFileHosts,
  listSshConfigs,
  probeSshPasswordAuth,
  testSshConnection,
  updateSshConfig,
} from "@/lib/backend";
import { formatDate } from "@/lib/utils";
import type {
  SshAuthType,
  SshConfig,
  SshConfigFileHost,
  SshKnownHostsMode,
  UpdateSshConfigInput,
} from "@/lib/types";
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

interface SshConfigFormState {
  name: string;
  host: string;
  port: string;
  username: string;
  authType: SshAuthType;
  privateKeyPath: string;
  password: string;
  passphrase: string;
  knownHostsMode: SshKnownHostsMode;
}

const EMPTY_FORM: SshConfigFormState = {
  name: "",
  host: "",
  port: "22",
  username: "",
  authType: "key",
  privateKeyPath: "",
  password: "",
  passphrase: "",
  knownHostsMode: "accept-new",
};

const KNOWN_HOSTS_MODES: SshKnownHostsMode[] = ["accept-new", "strict", "ask", "off"];

function configToForm(config: SshConfig): SshConfigFormState {
  return {
    name: config.name,
    host: config.host,
    port: String(config.port || 22),
    username: config.username,
    authType: config.auth_type,
    privateKeyPath: config.private_key_path ?? "",
    password: "",
    passphrase: "",
    knownHostsMode: config.known_hosts_mode,
  };
}

export function SshSettingsSection() {
  const { t } = useTranslation("settings");
  const [configs, setConfigs] = useState<SshConfig[]>([]);
  const [hosts, setHosts] = useState<SshConfigFileHost[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState<"save" | "delete" | "test" | "probe" | null>(null);
  const [deleteConfirming, setDeleteConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [form, setForm] = useState<SshConfigFormState>(EMPTY_FORM);
  const [dialogOpen, setDialogOpen] = useState(false);

  const selected = useMemo(
    () => configs.find((config) => config.id === selectedId) ?? null,
    [configs, selectedId],
  );
  const isCreate = selectedId === null;
  const knownHostsOptions = KNOWN_HOSTS_MODES.map((value) => ({
    value,
    label:
      value === "accept-new"
        ? t("ssh.knownHosts.acceptNew")
        : value === "strict"
          ? t("ssh.knownHosts.strict")
          : value === "ask"
            ? t("ssh.knownHosts.ask")
            : t("ssh.knownHosts.off"),
  }));

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const [items, fileHosts] = await Promise.all([
        listSshConfigs(),
        listSshConfigFileHosts().catch(() => [] as SshConfigFileHost[]),
      ]);
      setConfigs(items);
      setHosts(fileHosts);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const patchForm = (updates: Partial<SshConfigFormState>) => {
    setForm((current) => ({ ...current, ...updates }));
  };

  const openCreate = () => {
    setSelectedId(null);
    setForm(EMPTY_FORM);
    setMessage(null);
    setError(null);
    setDialogOpen(true);
  };

  const openEdit = (config: SshConfig) => {
    setSelectedId(config.id);
    setForm(configToForm(config));
    setMessage(null);
    setError(null);
    setDialogOpen(true);
  };

  const closeDialog = () => {
    if (saving !== null || deleteConfirming) {
      return;
    }
    setDialogOpen(false);
    setMessage(null);
    setError(null);
  };

  const handleImport = async (alias: string) => {
    setError(null);
    setMessage(null);
    try {
      const imported = await importSshConfigFileHost(alias);
      setSelectedId(null);
      setForm({
        name: imported.alias,
        host: imported.host,
        port: String(imported.port || 22),
        username: imported.username ?? "",
        authType: "key",
        privateKeyPath: imported.private_key_path ?? "",
        password: "",
        passphrase: "",
        knownHostsMode: "accept-new",
      });
      setDialogOpen(true);
      if (imported.proxy_jump_unsupported) {
        setError(t("ssh.import.proxyJumpUnsupported"));
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleSave = async () => {
    setError(null);
    setMessage(null);
    if (!form.name.trim() || !form.host.trim() || !form.username.trim()) {
      setError(t("ssh.messages.requiredFields"));
      return;
    }
    if (form.authType === "key" && !form.privateKeyPath.trim()) {
      setError(t("ssh.messages.privateKeyRequired"));
      return;
    }

    setSaving("save");
    const privateKeyPath = form.authType === "key" ? form.privateKeyPath.trim() || null : null;
    try {
      if (selectedId) {
        const updates: UpdateSshConfigInput = {
          name: form.name.trim(),
          host: form.host.trim(),
          port: Number(form.port) || 22,
          username: form.username.trim(),
          auth_type: form.authType,
          private_key_path: privateKeyPath,
          known_hosts_mode: form.knownHostsMode,
        };
        if (form.authType === "password" && form.password) {
          updates.password = form.password;
        }
        if (form.passphrase) {
          updates.passphrase = form.passphrase;
        }
        const updated = await updateSshConfig(selectedId, updates);
        setConfigs((current) =>
          current.map((config) => (config.id === updated.id ? updated : config)),
        );
        setDialogOpen(false);
        setMessage(t("ssh.messages.updated"));
      } else {
        const created = await createSshConfig({
          name: form.name.trim(),
          host: form.host.trim(),
          port: Number(form.port) || 22,
          username: form.username.trim(),
          auth_type: form.authType,
          private_key_path: privateKeyPath,
          password: form.authType === "password" && form.password ? form.password : null,
          passphrase: form.passphrase || null,
          known_hosts_mode: form.knownHostsMode,
        });
        setConfigs((current) => [created, ...current.filter((config) => config.id !== created.id)]);
        setSelectedId(created.id);
        setDialogOpen(false);
        setMessage(t("ssh.messages.created"));
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleDelete = async () => {
    if (!selected || saving !== null || deleteConfirming) return;
    const targetId = selected.id;
    const targetName = selected.name;
    setDeleteConfirming(true);
    setError(null);
    setMessage(null);
    try {
      const confirmed = await confirm(t("ssh.dialogs.deleteConfirm", { name: targetName }), {
        title: t("ssh.dialogs.deleteTitle"),
        kind: "warning",
      });
      if (!confirmed) return;
      setSaving("delete");
      await deleteSshConfig(targetId);
      setConfigs((current) => current.filter((config) => config.id !== targetId));
      setSelectedId(null);
      setForm(EMPTY_FORM);
      setDialogOpen(false);
      setMessage(t("ssh.messages.deleted"));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeleteConfirming(false);
      setSaving(null);
    }
  };

  const handleTest = async () => {
    if (!selectedId) return;
    setSaving("test");
    setError(null);
    setMessage(null);
    try {
      const result = await testSshConnection(selectedId);
      const items = await listSshConfigs();
      setConfigs(items);
      if (result.ok) {
        setMessage(result.message);
      } else {
        setError(result.message);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleProbe = async () => {
    if (!selectedId) return;
    setSaving("probe");
    setError(null);
    setMessage(null);
    try {
      const result = await probeSshPasswordAuth(selectedId);
      const items = await listSshConfigs();
      setConfigs(items);
      if (result.supported && result.status !== "failed") {
        setMessage(result.message);
      } else {
        setError(result.message);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleChoosePrivateKey = async () => {
    const path = await open({
      multiple: false,
      title: t("ssh.desktop.selectPrivateKeyTitle"),
    });
    if (typeof path === "string") {
      patchForm({ privateKeyPath: path });
    }
  };

  const busy = saving !== null;
  const formLocked = busy || deleteConfirming;
  const selectedSummary = selected ? `${selected.username}@${selected.host}:${selected.port}` : "";
  const testStatus = selected
    ? selected.auth_type === "password"
      ? selected.password_probe_status
      : selected.last_check_status
    : null;
  const testDetail = selected
    ? selected.auth_type === "password"
      ? selected.password_probe_message
      : selected.last_check_message
    : null;

  return (
    <div className="space-y-6">
      <div className="space-y-4 rounded-lg border border-border bg-card p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h3 className="text-sm font-medium">{t("ssh.title")}</h3>
            <p className="text-xs text-muted-foreground">{t("ssh.description")}</p>
          </div>
          <Button variant="outline" onClick={openCreate}>
            <Plus className="mr-1 h-4 w-4" />
            {t("ssh.actions.new")}
          </Button>
        </div>

        {!dialogOpen && message ? <p className="text-sm text-muted-foreground">{message}</p> : null}
        {!dialogOpen && error ? <p className="text-sm text-destructive">{error}</p> : null}

        <div className="rounded-md border border-border">
          {loading ? (
            <div className="flex h-28 items-center justify-center text-sm text-muted-foreground">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              {t("ssh.list.loading")}
            </div>
          ) : configs.length === 0 ? (
            <div className="px-3 py-6 text-sm text-muted-foreground">{t("ssh.list.empty")}</div>
          ) : (
            configs.map((config) => (
              <button
                key={config.id}
                type="button"
                onClick={() => openEdit(config)}
                className={`w-full border-b border-border px-3 py-3 text-left last:border-b-0 ${
                  selectedId === config.id ? "bg-primary/5" : "hover:bg-muted/40"
                }`}
              >
                <div className="text-sm font-medium">{config.name}</div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {config.username}@{config.host}:{config.port}
                </div>
                <div className="mt-2 flex flex-wrap gap-2 text-[11px]">
                  <span className="rounded bg-secondary px-1.5 py-0.5 text-secondary-foreground">
                    {t(
                      config.auth_type === "password"
                        ? "ssh.badges.passwordLogin"
                        : "ssh.badges.keyLogin",
                    )}
                  </span>
                  {config.last_checked_at ? (
                    <span className="rounded border border-border px-1.5 py-0.5 text-muted-foreground">
                      {t("ssh.list.checkedAt", { date: formatDate(config.last_checked_at) })}
                    </span>
                  ) : null}
                </div>
              </button>
            ))
          )}
        </div>
      </div>

      <div className="space-y-4 rounded-lg border border-border bg-card p-4">
        <div>
          <h3 className="text-sm font-medium">{t("ssh.import.title")}</h3>
          <p className="text-xs text-muted-foreground">{t("ssh.import.hint")}</p>
        </div>
        {hosts.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("ssh.import.empty")}</p>
        ) : (
          <div className="rounded-md border border-border">
            {hosts.map((host) => (
              <button
                key={host.alias}
                type="button"
                className="w-full border-b border-border px-3 py-3 text-left text-sm last:border-b-0 hover:bg-muted/40"
                onClick={() => void handleImport(host.alias)}
              >
                {host.alias} · {host.host}:{host.port}
                {host.has_proxy_jump ? ` · ${t("ssh.import.proxyJump")}` : ""}
              </button>
            ))}
          </div>
        )}
      </div>

      <Dialog
        open={dialogOpen}
        onOpenChange={(openDialog) => {
          if (!openDialog) {
            closeDialog();
          }
        }}
      >
        <DialogContent
          className="flex max-h-[90vh] w-full flex-col gap-0 overflow-hidden sm:max-w-2xl"
          showCloseButton={!formLocked}
        >
          <DialogHeader className="shrink-0 pb-3">
            <DialogTitle>
              {isCreate ? t("ssh.dialogs.createTitle") : t("ssh.dialogs.editTitle")}
            </DialogTitle>
            <DialogDescription>
              {isCreate ? t("ssh.form.createDescription") : t("ssh.form.editDescription")}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto pr-1">
            <div className="space-y-3">
              <div className="grid gap-3 md:grid-cols-2">
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("ssh.form.configName")}
                  </label>
                  <Input
                    value={form.name}
                    onChange={(event) => patchForm({ name: event.target.value })}
                    placeholder={t("ssh.form.placeholders.configName")}
                    className="mt-1"
                    disabled={formLocked}
                  />
                </div>
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("ssh.form.host")}
                  </label>
                  <Input
                    value={form.host}
                    onChange={(event) => patchForm({ host: event.target.value })}
                    placeholder={t("ssh.form.placeholders.host")}
                    className="mt-1"
                    disabled={formLocked}
                  />
                </div>
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("ssh.form.port")}
                  </label>
                  <Input
                    value={form.port}
                    onChange={(event) => patchForm({ port: event.target.value })}
                    placeholder={t("ssh.form.placeholders.port")}
                    className="mt-1"
                    disabled={formLocked}
                  />
                </div>
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("ssh.form.username")}
                  </label>
                  <Input
                    value={form.username}
                    onChange={(event) => patchForm({ username: event.target.value })}
                    placeholder={t("ssh.form.placeholders.username")}
                    className="mt-1"
                    disabled={formLocked}
                  />
                </div>
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("ssh.form.authType")}
                  </label>
                  <Select
                    value={form.authType}
                    disabled={formLocked}
                    onValueChange={(value) => {
                      if (value === "key" || value === "password") {
                        patchForm({ authType: value });
                      }
                    }}
                  >
                    <SelectTrigger className="mt-1 bg-background">
                      <SelectValue>
                        {(value) =>
                          value === "password"
                            ? t("ssh.badges.passwordLogin")
                            : value === "key"
                              ? t("ssh.badges.keyLogin")
                              : t("ssh.form.authType")
                        }
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="key">{t("ssh.badges.keyLogin")}</SelectItem>
                      <SelectItem value="password">{t("ssh.badges.passwordLogin")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("ssh.form.knownHostsPolicy")}
                  </label>
                  <Select
                    value={form.knownHostsMode}
                    disabled={formLocked}
                    onValueChange={(value) => {
                      if (
                        value === "accept-new" ||
                        value === "strict" ||
                        value === "ask" ||
                        value === "off"
                      ) {
                        patchForm({ knownHostsMode: value });
                      }
                    }}
                  >
                    <SelectTrigger className="mt-1 bg-background">
                      <SelectValue>
                        {(value) =>
                          typeof value === "string"
                            ? (knownHostsOptions.find((option) => option.value === value)?.label ??
                              value)
                            : t("ssh.form.knownHostsPolicy")
                        }
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      {knownHostsOptions.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {form.authType === "key" ? (
                <div className="grid gap-3 md:grid-cols-2">
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("ssh.form.privateKeyPath")}
                    </label>
                    <div className="mt-1 flex gap-2">
                      <Input
                        value={form.privateKeyPath}
                        onChange={(event) => patchForm({ privateKeyPath: event.target.value })}
                        placeholder={t("ssh.form.placeholders.privateKeyPath")}
                        className="flex-1"
                        disabled={formLocked}
                      />
                      <Button
                        type="button"
                        variant="outline"
                        onClick={() => void handleChoosePrivateKey()}
                        disabled={formLocked}
                        title={t("ssh.desktop.selectPrivateKeyTitle")}
                      >
                        <FolderOpen className="h-4 w-4" />
                        {t("ssh.actions.choose")}
                      </Button>
                    </div>
                  </div>
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("ssh.form.passphrase")}
                    </label>
                    <Input
                      type="password"
                      value={form.passphrase}
                      onChange={(event) => patchForm({ passphrase: event.target.value })}
                      placeholder={
                        selected?.passphrase_configured
                          ? t("ssh.form.placeholders.passphraseKeepExisting")
                          : t("ssh.form.placeholders.passphraseOptional")
                      }
                      className="mt-1"
                      autoComplete="off"
                      disabled={formLocked}
                    />
                  </div>
                </div>
              ) : (
                <div className="grid gap-3 md:grid-cols-2">
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("ssh.form.password")}
                    </label>
                    <Input
                      type="password"
                      value={form.password}
                      onChange={(event) => patchForm({ password: event.target.value })}
                      placeholder={
                        selected?.password_configured
                          ? t("ssh.form.placeholders.passwordKeepExisting")
                          : t("ssh.form.placeholders.passwordEnter")
                      }
                      className="mt-1"
                      autoComplete="off"
                      disabled={formLocked}
                    />
                  </div>
                  <div>
                    <label className="text-xs font-medium text-muted-foreground">
                      {t("ssh.form.passphrase")}
                    </label>
                    <Input
                      type="password"
                      value={form.passphrase}
                      onChange={(event) => patchForm({ passphrase: event.target.value })}
                      placeholder={
                        selected?.passphrase_configured
                          ? t("ssh.form.placeholders.passphraseKeepExisting")
                          : t("ssh.form.placeholders.passphraseOptional")
                      }
                      className="mt-1"
                      autoComplete="off"
                      disabled={formLocked}
                    />
                  </div>
                </div>
              )}

              {selected ? (
                <div className="rounded-md border border-border bg-muted/30 px-3 py-3 text-xs text-muted-foreground">
                  <div className="flex items-center justify-between gap-3">
                    <div className="font-medium text-foreground">{t("ssh.status.title")}</div>
                    <span className="rounded bg-secondary px-2 py-1 text-xs text-secondary-foreground">
                      {t(
                        selected.auth_type === "password"
                          ? "ssh.badges.passwordAuth"
                          : "ssh.badges.keyAuth",
                      )}
                    </span>
                  </div>
                  <div className="mt-1">
                    {t("ssh.status.host")}: {selectedSummary}
                  </div>
                  <div className="mt-1">
                    {t("ssh.status.connectionTest")}:{" "}
                    {testStatus ? testStatus : t("ssh.status.notTested")}
                  </div>
                  {testDetail ? <div className="mt-1">{testDetail}</div> : null}
                </div>
              ) : null}

              {!isCreate ? (
                <div className="flex flex-wrap gap-2">
                  <Button variant="outline" onClick={() => void handleTest()} disabled={formLocked}>
                    {saving === "test" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                    {t("ssh.actions.testConnection")}
                  </Button>
                  {selected?.auth_type === "password" ? (
                    <Button
                      variant="outline"
                      onClick={() => void handleProbe()}
                      disabled={formLocked}
                    >
                      {saving === "probe" ? (
                        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      ) : null}
                      {t("ssh.actions.probePassword")}
                    </Button>
                  ) : null}
                </div>
              ) : null}

              {error ? <p className="text-sm text-destructive">{error}</p> : null}
              {message ? <p className="text-sm text-muted-foreground">{message}</p> : null}
            </div>
          </div>
          <DialogFooter className="mt-4 shrink-0">
            {!isCreate ? (
              <Button
                variant="destructive"
                className="sm:mr-auto"
                onClick={() => void handleDelete()}
                disabled={formLocked}
              >
                {saving === "delete" ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : null}
                <Trash2 className="mr-1 h-4 w-4" />
                {t("ssh.actions.delete")}
              </Button>
            ) : null}
            <Button variant="outline" onClick={closeDialog} disabled={formLocked}>
              {t("ssh.actions.cancel")}
            </Button>
            <Button onClick={() => void handleSave()} disabled={formLocked}>
              {saving === "save" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("ssh.actions.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
