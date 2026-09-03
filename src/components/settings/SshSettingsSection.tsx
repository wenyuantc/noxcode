import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
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
import type {
  CreateSshConfigInput,
  SshConfig,
  SshConfigFileHost,
  SshKnownHostsMode,
} from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SettingCard } from "./SettingCard";

const EMPTY: CreateSshConfigInput = {
  name: "",
  host: "",
  port: 22,
  username: "",
  auth_type: "key",
  private_key_path: "",
  password: "",
  known_hosts_mode: "accept-new",
};

export function SshSettingsSection() {
  const { t } = useTranslation(["settings", "ssh", "common"]);
  const [configs, setConfigs] = useState<SshConfig[]>([]);
  const [hosts, setHosts] = useState<SshConfigFileHost[]>([]);
  const [editing, setEditing] = useState<string | "new" | null>(null);
  const [form, setForm] = useState(EMPTY);
  const [message, setMessage] = useState<string | null>(null);

  const reload = () => void listSshConfigs().then(setConfigs);
  useEffect(() => {
    void reload();
    void listSshConfigFileHosts()
      .then(setHosts)
      .catch(() => setHosts([]));
  }, []);

  const persist = async () => {
    try {
      if (editing === "new") await createSshConfig(form);
      else if (editing) await updateSshConfig(editing, form);
      setEditing(null);
      await reload();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <div className="space-y-4">
      <SettingCard title={t("settings:ssh.title")} description={t("settings:ssh.hint")}>
        <div className="space-y-2">
          {configs.map((config) => (
            <div key={config.id} className="flex items-center gap-2 rounded-md border px-3 py-2">
              <div className="min-w-0 flex-1 text-sm">
                <p className="font-medium">{config.name}</p>
                <p className="text-xs text-muted-foreground">
                  {config.username}@{config.host}:{config.port}
                </p>
              </div>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setEditing(config.id);
                  setForm({
                    name: config.name,
                    host: config.host,
                    port: config.port,
                    username: config.username,
                    auth_type: config.auth_type,
                    private_key_path: config.private_key_path ?? "",
                    known_hosts_mode: config.known_hosts_mode,
                  });
                }}
              >
                {t("common:edit")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  void deleteSshConfig(config.id)
                    .then(reload)
                    .catch((err: unknown) => setMessage(String(err)))
                }
              >
                {t("common:delete")}
              </Button>
            </div>
          ))}
          <Button
            size="sm"
            onClick={() => {
              setEditing("new");
              setForm(EMPTY);
            }}
          >
            {t("settings:ssh.new")}
          </Button>
        </div>
      </SettingCard>
      <SettingCard title={t("settings:ssh.import")} description={t("settings:ssh.importHint")}>
        <div className="space-y-2">
          {hosts.map((host) => (
            <button
              key={host.alias}
              type="button"
              className="block w-full rounded-md border px-3 py-2 text-left text-sm hover:bg-accent"
              onClick={() => {
                void importSshConfigFileHost(host.alias).then((imported) => {
                  if (imported.proxy_jump_unsupported) {
                    setMessage(t("settings:ssh.proxyJumpUnsupported"));
                  }
                  setEditing("new");
                  setForm({
                    name: imported.alias,
                    host: imported.host,
                    port: imported.port,
                    username: imported.username,
                    auth_type: "key",
                    private_key_path: imported.private_key_path ?? "",
                  });
                });
              }}
            >
              {host.alias} · {host.host}:{host.port}
              {host.has_proxy_jump ? " · ProxyJump" : ""}
            </button>
          ))}
        </div>
      </SettingCard>
      {editing ? (
        <SettingCard title={t("settings:ssh.new")}>
          <div className="grid grid-cols-2 gap-2">
            <Input
              placeholder={t("ssh:name")}
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
            <Input
              placeholder={t("ssh:host")}
              value={form.host}
              onChange={(e) => setForm({ ...form, host: e.target.value })}
            />
            <Input
              placeholder={t("ssh:username")}
              value={form.username}
              onChange={(e) => setForm({ ...form, username: e.target.value })}
            />
            <Input
              placeholder={t("ssh:port")}
              value={String(form.port ?? 22)}
              onChange={(e) => setForm({ ...form, port: Number(e.target.value) || 22 })}
            />
            <select
              className="h-8 rounded-md border px-2 text-sm"
              value={form.auth_type}
              onChange={(e) =>
                setForm({ ...form, auth_type: e.target.value as "key" | "password" })
              }
            >
              <option value="key">{t("ssh:key")}</option>
              <option value="password">{t("ssh:password")}</option>
            </select>
            <select
              className="h-8 rounded-md border px-2 text-sm"
              value={form.known_hosts_mode}
              onChange={(e) =>
                setForm({ ...form, known_hosts_mode: e.target.value as SshKnownHostsMode })
              }
            >
              <option value="accept-new">accept-new</option>
              <option value="strict">strict</option>
              <option value="ask">ask</option>
              <option value="off">off</option>
            </select>
            {form.auth_type === "key" ? (
              <Button
                variant="outline"
                onClick={() =>
                  void open({ multiple: false }).then(
                    (path) =>
                      typeof path === "string" && setForm({ ...form, private_key_path: path }),
                  )
                }
              >
                {form.private_key_path || t("ssh:privateKey")}
              </Button>
            ) : (
              <Input
                type="password"
                placeholder={t("ssh:password")}
                value={form.password ?? ""}
                onChange={(e) => setForm({ ...form, password: e.target.value })}
              />
            )}
          </div>
          <div className="mt-3 flex gap-2">
            <Button onClick={() => void persist()}>{t("common:save")}</Button>
            {editing !== "new" ? (
              <>
                <Button
                  variant="outline"
                  onClick={() => void testSshConnection(editing).then((r) => setMessage(r.message))}
                >
                  {t("settings:ssh.test")}
                </Button>
                <Button
                  variant="outline"
                  onClick={() =>
                    void probeSshPasswordAuth(editing).then((r) => setMessage(r.message))
                  }
                >
                  {t("settings:ssh.probe")}
                </Button>
              </>
            ) : null}
          </div>
        </SettingCard>
      ) : null}
      {message ? <p className="text-sm text-destructive">{message}</p> : null}
    </div>
  );
}
