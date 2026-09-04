import { confirm, open } from "@tauri-apps/plugin-dialog";
import { Download, FolderOpen, Loader2, Pencil, Plus, Terminal, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  createSshConfig,
  deleteSshConfig,
  importSshConfigFileHost,
  listSshConfigFileHosts,
  listSshConfigs,
  listSshSupportedAlgorithms,
  probeSshPasswordAuth,
  testSshConnection,
  updateSshConfig,
} from "@/lib/backend";
import { formatDate } from "@/lib/utils";
import type {
  SshAlgorithms,
  SshAuthType,
  SshConfig,
  SshConfigFileHost,
  SshKnownHostsMode,
  SshSupportedAlgorithms,
  UpdateSshConfigInput,
} from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
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
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

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
  algorithms: SshAlgorithms;
}

function emptyAlgorithms(): SshAlgorithms {
  return { kex: [], host_key: [], cipher: [], mac: [] };
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
  algorithms: emptyAlgorithms(),
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
    algorithms: config.algorithms
      ? {
          kex: [...config.algorithms.kex],
          host_key: [...config.algorithms.host_key],
          cipher: [...config.algorithms.cipher],
          mac: [...config.algorithms.mac],
        }
      : emptyAlgorithms(),
  };
}

export function SshSettingsSection() {
  const { t } = useTranslation("settings");
  const [configs, setConfigs] = useState<SshConfig[]>([]);
  const [hosts, setHosts] = useState<SshConfigFileHost[]>([]);
  const [algorithmCatalog, setAlgorithmCatalog] = useState<SshSupportedAlgorithms | null>(null);
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
      const [items, fileHosts, algorithms] = await Promise.all([
        listSshConfigs(),
        listSshConfigFileHosts().catch(() => [] as SshConfigFileHost[]),
        listSshSupportedAlgorithms(),
      ]);
      setConfigs(items);
      setHosts(fileHosts);
      setAlgorithmCatalog(algorithms);
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

  const toggleAlgorithm = (kind: keyof SshAlgorithms, name: string, checked: boolean) => {
    setForm((current) => ({
      ...current,
      algorithms: {
        ...current.algorithms,
        [kind]: checked
          ? [...current.algorithms[kind].filter((item) => item !== name), name]
          : current.algorithms[kind].filter((item) => item !== name),
      },
    }));
  };

  const openCreate = () => {
    setSelectedId(null);
    setForm({ ...EMPTY_FORM, algorithms: emptyAlgorithms() });
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
        algorithms: emptyAlgorithms(),
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
    const algorithms = Object.values(form.algorithms).some((names) => names.length > 0)
      ? form.algorithms
      : null;
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
          algorithms,
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
          algorithms,
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
      setForm({ ...EMPTY_FORM, algorithms: emptyAlgorithms() });
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
      {!dialogOpen && message ? (
        <SettingFeedbackCallout
          variant="success"
          message={message}
          onClose={() => setMessage(null)}
        />
      ) : null}
      {!dialogOpen && error ? (
        <SettingFeedbackCallout variant="error" message={error} onClose={() => setError(null)} />
      ) : null}

      {/* SSH 连接列表卡片 */}
      <SettingCard
        icon={Terminal}
        title={t("ssh.title")}
        description={t("ssh.description")}
        badge={`${configs.length} 个配置`}
        headerAction={
          <Button size="sm" onClick={openCreate} className="h-7 gap-1 text-xs">
            <Plus className="size-3.5" />
            {t("ssh.actions.new")}
          </Button>
        }
      >
        {loading ? (
          <div className="flex h-32 items-center justify-center text-xs text-muted-foreground">
            <Loader2 className="mr-2 size-4 animate-spin text-primary" />
            {t("ssh.list.loading")}
          </div>
        ) : configs.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 text-center">
            <div className="flex size-10 items-center justify-center rounded-xl border border-border/70 bg-muted/30 text-muted-foreground">
              <Terminal className="size-5" />
            </div>
            <p className="mt-3 text-xs font-semibold text-foreground">{t("ssh.list.empty")}</p>
            <p className="mt-1 text-[11px] text-muted-foreground">
              添加远程 SSH 主机配置，即可支持远端执行与代码同步。
            </p>
          </div>
        ) : (
          <div className="grid gap-2.5">
            {configs.map((config) => (
              <div
                key={config.id}
                className="group flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all hover:border-border hover:shadow-xs"
              >
                <div className="flex items-start gap-3 min-w-0">
                  <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                    <Terminal className="size-4" />
                  </div>
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-xs font-semibold tracking-tight text-foreground truncate">
                        {config.name}
                      </span>
                      <span className="rounded-md border border-border/60 bg-muted/40 px-1.5 py-0.2 text-[10px] font-mono text-muted-foreground">
                        {t(
                          config.auth_type === "password"
                            ? "ssh.badges.passwordLogin"
                            : "ssh.badges.keyLogin",
                        )}
                      </span>
                      {config.last_checked_at ? (
                        <span className="rounded-md border border-border/40 bg-background px-1.5 py-0.2 text-[10px] font-mono text-muted-foreground">
                          {t("ssh.list.checkedAt", { date: formatDate(config.last_checked_at) })}
                        </span>
                      ) : null}
                    </div>
                    <p className="text-[11px] font-mono text-muted-foreground/80 truncate">
                      {config.username}@{config.host}:{config.port}
                    </p>
                  </div>
                </div>

                <div className="flex items-center gap-1.5 shrink-0 self-end sm:self-center">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => openEdit(config)}
                    className="h-7 text-xs gap-1"
                  >
                    <Pencil className="size-3" />
                    {t("common:edit", { defaultValue: "编辑" })}
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </SettingCard>

      {/* 导入 ~/.ssh/config 主机卡片 */}
      <SettingCard
        icon={FolderOpen}
        title={t("ssh.import.title")}
        description={t("ssh.import.hint")}
        badge={hosts.length > 0 ? `${hosts.length} 可导入` : undefined}
      >
        {hosts.length === 0 ? (
          <p className="py-2 text-center text-xs text-muted-foreground">{t("ssh.import.empty")}</p>
        ) : (
          <div className="grid gap-2 sm:grid-cols-2">
            {hosts.map((host) => (
              <div
                key={host.alias}
                className="flex items-center justify-between gap-2 rounded-xl border border-border/70 bg-card p-2.5 text-xs shadow-2xs transition-all hover:border-border"
              >
                <div className="min-w-0 flex-1 space-y-0.5">
                  <p className="font-semibold text-foreground truncate">{host.alias}</p>
                  <p className="text-[11px] font-mono text-muted-foreground truncate">
                    {host.host}:{host.port}
                    {host.has_proxy_jump ? ` · ${t("ssh.import.proxyJump")}` : ""}
                  </p>
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-6 text-xs gap-1 px-2 shrink-0"
                  onClick={() => void handleImport(host.alias)}
                >
                  <Download className="size-3" />
                  {t("ssh.import.action", { defaultValue: "导入" })}
                </Button>
              </div>
            ))}
          </div>
        )}
      </SettingCard>

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

              <Collapsible className="rounded-md border border-border">
                <CollapsibleTrigger className="flex w-full items-center justify-between px-3 py-2 text-left text-sm font-medium">
                  {t("ssh.form.algorithmsTitle")}
                </CollapsibleTrigger>
                <CollapsibleContent className="space-y-3 border-t border-border p-3">
                  <p className="text-xs text-muted-foreground">{t("ssh.form.algorithmsHint")}</p>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={formLocked || !algorithmCatalog}
                      onClick={() => {
                        if (!algorithmCatalog) return;
                        patchForm({
                          algorithms: {
                            kex: [...algorithmCatalog.legacy_preset.kex],
                            host_key: [...algorithmCatalog.legacy_preset.host_key],
                            cipher: [...algorithmCatalog.legacy_preset.cipher],
                            mac: [...algorithmCatalog.legacy_preset.mac],
                          },
                        });
                      }}
                    >
                      {t("ssh.form.algorithmsLegacyPreset")}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      disabled={formLocked}
                      onClick={() => patchForm({ algorithms: emptyAlgorithms() })}
                    >
                      {t("ssh.form.algorithmsRestoreDefault")}
                    </Button>
                  </div>
                  {algorithmCatalog ? (
                    <div className="grid gap-3 md:grid-cols-2">
                      {(
                        [
                          ["kex", t("ssh.form.algorithmsKex")],
                          ["host_key", t("ssh.form.algorithmsHostKey")],
                          ["cipher", t("ssh.form.algorithmsCipher")],
                          ["mac", t("ssh.form.algorithmsMac")],
                        ] as const
                      ).map(([kind, label]) => (
                        <fieldset key={kind} className="rounded-md border border-border p-2">
                          <legend className="px-1 text-xs font-medium">{label}</legend>
                          <div className="max-h-40 space-y-1 overflow-y-auto">
                            {algorithmCatalog.supported[kind].map((name) => (
                              <label
                                key={name}
                                className="flex items-start gap-2 font-mono text-[11px]"
                              >
                                <input
                                  type="checkbox"
                                  className="mt-0.5 h-3.5 w-3.5"
                                  checked={form.algorithms[kind].includes(name)}
                                  disabled={formLocked}
                                  onChange={(event) =>
                                    toggleAlgorithm(kind, name, event.target.checked)
                                  }
                                />
                                <span className="break-all">{name}</span>
                              </label>
                            ))}
                          </div>
                        </fieldset>
                      ))}
                    </div>
                  ) : (
                    <p className="text-xs text-muted-foreground">
                      {t("ssh.form.algorithmsUnavailable")}
                    </p>
                  )}
                </CollapsibleContent>
              </Collapsible>

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
