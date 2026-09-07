import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { createSshConfig, listSshConfigs, testSshConnection } from "@/lib/backend";
import { RemoteConnectionValidationError, submitRemoteConnection } from "@/lib/remoteConnection";
import type { CreateSshConfigInput, SshConfig } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function RemoteConnectDialog({
  open: isOpen,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation(["ssh", "git", "common"]);
  const create = useWorkspaceStore((state) => state.create);
  const [configs, setConfigs] = useState<SshConfig[]>([]);
  const [sshConfigId, setSshConfigId] = useState("");
  const [name, setName] = useState("");
  const [remotePath, setRemotePath] = useState("");
  const [creating, setCreating] = useState(false);
  const [form, setForm] = useState<CreateSshConfigInput>({
    name: "",
    host: "",
    port: 22,
    username: "",
    auth_type: "key",
    private_key_path: "",
    password: "",
  });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const submitting = useRef(false);
  const generation = useRef(0);
  const configRequest = useRef(0);

  useEffect(() => {
    const current = ++generation.current;
    const request = ++configRequest.current;
    if (!isOpen) return;
    void listSshConfigs()
      .then((items) => {
        if (generation.current !== current || configRequest.current !== request) return;
        setConfigs(items);
        setSshConfigId((id) =>
          items.some((config) => config.id === id) ? id : (items[0]?.id ?? ""),
        );
      })
      .catch((err) => {
        if (generation.current === current && configRequest.current === request) {
          setError(String(err));
        }
      });
    return () => {
      generation.current += 1;
    };
  }, [isOpen]);

  const submit = async () => {
    if (submitting.current) return;
    submitting.current = true;
    setBusy(true);
    setError(null);
    const current = generation.current;
    const isCurrent = () => generation.current === current;
    try {
      const completed = await submitRemoteConnection(
        {
          name,
          remotePath,
          sshConfigId: creating ? undefined : sshConfigId,
          newConfig: creating ? form : undefined,
        },
        {
          createSshConfig,
          testSshConnection,
          createWorkspace: create,
          isCurrent,
          onConfigCreated: (config) => {
            if (!isCurrent()) return;
            // 初次加载的旧列表即使在测试失败后才返回，也不能抹掉刚保存的配置 ID。
            configRequest.current += 1;
            setConfigs((items) => [config, ...items.filter((item) => item.id !== config.id)]);
            setSshConfigId(config.id);
            setCreating(false);
            setName((value) => (value.trim() ? value : config.name));
            setForm((value) => ({ ...value, password: "" }));
          },
        },
      );
      if (completed) onOpenChange(false);
    } catch (err) {
      if (isCurrent()) {
        setError(
          err instanceof RemoteConnectionValidationError
            ? t(`ssh:validation.${err.field}`)
            : err instanceof Error
              ? err.message
              : String(err),
        );
      }
    } finally {
      submitting.current = false;
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(next) => {
        if (!submitting.current) onOpenChange(next);
      }}
    >
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("git:remoteConnect")}</DialogTitle>
        </DialogHeader>
        <fieldset disabled={busy} className="space-y-3" aria-busy={busy}>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={creating}
              onChange={(event) => setCreating(event.target.checked)}
            />
            {t("common:create")}
          </label>
          {creating ? (
            <div className="grid grid-cols-2 gap-2">
              <Input
                placeholder={t("ssh:name")}
                value={form.name}
                onChange={(event) => setForm({ ...form, name: event.target.value })}
              />
              <Input
                placeholder={t("ssh:host")}
                value={form.host}
                onChange={(event) => setForm({ ...form, host: event.target.value })}
              />
              <Input
                placeholder={t("ssh:username")}
                value={form.username}
                onChange={(event) => setForm({ ...form, username: event.target.value })}
              />
              <Input
                placeholder={t("ssh:port")}
                value={String(form.port ?? 22)}
                onChange={(event) => setForm({ ...form, port: Number(event.target.value) })}
              />
              <select
                className="h-8 rounded-md border px-2 text-sm"
                value={form.auth_type}
                onChange={(event) =>
                  setForm({ ...form, auth_type: event.target.value as "key" | "password" })
                }
              >
                <option value="key">{t("ssh:key")}</option>
                <option value="password">{t("ssh:password")}</option>
              </select>
              {form.auth_type === "key" ? (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    void open({ multiple: false }).then((path) => {
                      if (typeof path === "string") {
                        setForm((current) => ({ ...current, private_key_path: path }));
                      }
                    });
                  }}
                >
                  {form.private_key_path || t("ssh:privateKey")}
                </Button>
              ) : (
                <Input
                  type="password"
                  placeholder={t("ssh:password")}
                  value={form.password ?? ""}
                  onChange={(event) => setForm({ ...form, password: event.target.value })}
                />
              )}
            </div>
          ) : (
            <select
              className="h-8 w-full rounded-md border px-2 text-sm"
              value={sshConfigId}
              onChange={(event) => setSshConfigId(event.target.value)}
            >
              {configs.map((config) => (
                <option key={config.id} value={config.id}>
                  {config.name} ({config.username}@{config.host})
                </option>
              ))}
            </select>
          )}
          <Input
            placeholder={t("common:create")}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
          <Input
            placeholder={t("ssh:remotePath")}
            value={remotePath}
            onChange={(event) => setRemotePath(event.target.value)}
          />
          {error ? <p className="text-sm text-destructive">{error}</p> : null}
        </fieldset>
        <DialogFooter>
          <Button variant="ghost" disabled={busy} onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button disabled={busy} onClick={() => void submit()}>
            {busy ? t("ssh:connecting") : t("common:confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
