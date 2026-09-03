import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { createSshConfig, listSshConfigs, testSshConnection } from "@/lib/backend";
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

  useEffect(() => {
    if (!isOpen) return;
    void listSshConfigs().then((items) => {
      setConfigs(items);
      if (items[0]) setSshConfigId(items[0].id);
    });
  }, [isOpen]);

  const submit = async () => {
    setError(null);
    try {
      let configId = sshConfigId;
      if (creating) {
        const created = await createSshConfig({
          ...form,
          port: form.port || 22,
        });
        configId = created.id;
        await testSshConnection(created.id).catch(() => undefined);
      }
      if (!configId || !remotePath.trim()) {
        setError(t("ssh:remotePath"));
        return;
      }
      await create({
        name: name.trim() || form.name || remotePath.trim(),
        workspace_type: "ssh",
        ssh_config_id: configId,
        remote_repo_path: remotePath.trim(),
      });
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <Dialog open={isOpen} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("git:remoteConnect")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
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
                onChange={(event) => setForm({ ...form, port: Number(event.target.value) || 22 })}
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
                        setForm({ ...form, private_key_path: path });
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
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button onClick={() => void submit()}>{t("common:confirm")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
