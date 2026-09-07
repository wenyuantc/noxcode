import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  FolderOpen,
  FolderTree,
  HardDrive,
  Key,
  Loader2,
  Plus,
  Server,
  ShieldCheck,
} from "lucide-react";
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { RemoteDirectoryPickerDialog } from "./RemoteDirectoryPickerDialog";

const NEW_HOST_VALUE = "__new_host__";

function extractProjectName(path: string): string {
  const trimmed = path.trim().replace(/[\\/]+$/, "");
  if (!trimmed) return "";
  const parts = trimmed.split(/[\\/]/);
  return parts[parts.length - 1] || "";
}

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
  const [selectedHostId, setSelectedHostId] = useState<string>(NEW_HOST_VALUE);
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [remotePath, setRemotePath] = useState("");

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
  const [testingStatus, setTestingStatus] = useState<"idle" | "testing" | "passed" | "failed">(
    "idle",
  );
  const [testingMsg, setTestingMsg] = useState<string | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);

  const submitting = useRef(false);
  const generation = useRef(0);
  const configRequest = useRef(0);

  const isNewHost = selectedHostId === NEW_HOST_VALUE;
  const currentConfig = configs.find((c) => c.id === selectedHostId);

  // 加载 SSH 配置列表
  useEffect(() => {
    const current = ++generation.current;
    const request = ++configRequest.current;
    if (!isOpen) return;

    setError(null);
    setTestingStatus("idle");
    setTestingMsg(null);

    void listSshConfigs()
      .then((items) => {
        if (generation.current !== current || configRequest.current !== request) return;
        setConfigs(items);
        if (items.length > 0) {
          setSelectedHostId((prev) =>
            prev === NEW_HOST_VALUE || items.some((item) => item.id === prev)
              ? prev === NEW_HOST_VALUE && items.length > 0
                ? items[0].id
                : prev
              : items[0].id,
          );
        } else {
          setSelectedHostId(NEW_HOST_VALUE);
        }
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

  // 当选择已有主机或切换时重置测试状态
  const handleHostChange = (nextId: string) => {
    setSelectedHostId(nextId);
    setTestingStatus("idle");
    setTestingMsg(null);
    setError(null);
  };

  // 路径更新时，自动推导项目名称（若用户未手动修改过）
  const handleRemotePathChange = (path: string) => {
    setRemotePath(path);
    if (!nameEdited || !name.trim()) {
      const extracted = extractProjectName(path);
      if (extracted) {
        setName(extracted);
      }
    }
  };

  // 保存新建的主机配置
  const saveNewHostConfig = async (): Promise<SshConfig> => {
    if (!form.name.trim() || !form.host.trim() || !form.username.trim()) {
      throw new RemoteConnectionValidationError("requiredFields");
    }
    const port = form.port ?? 22;
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      throw new RemoteConnectionValidationError("port");
    }
    if (form.auth_type === "key" && !form.private_key_path?.trim()) {
      throw new RemoteConnectionValidationError("privateKey");
    }
    if (form.auth_type === "password" && !form.password) {
      throw new RemoteConnectionValidationError("password");
    }

    const created = await createSshConfig({
      ...form,
      name: form.name.trim(),
      host: form.host.trim(),
      username: form.username.trim(),
      port: form.port ?? 22,
      private_key_path: form.auth_type === "key" ? form.private_key_path?.trim() : null,
      password: form.auth_type === "password" ? form.password : null,
    });

    configRequest.current += 1;
    setConfigs((items) => [created, ...items.filter((item) => item.id !== created.id)]);
    setSelectedHostId(created.id);
    return created;
  };

  // 测试连接
  const handleTestConnection = async () => {
    setError(null);
    setTestingStatus("testing");
    setTestingMsg(null);

    try {
      let targetId = selectedHostId;
      if (isNewHost) {
        const saved = await saveNewHostConfig();
        targetId = saved.id;
      }

      if (!targetId || targetId === NEW_HOST_VALUE) {
        throw new RemoteConnectionValidationError("config");
      }

      const res = await testSshConnection(targetId);
      if (res.ok) {
        setTestingStatus("passed");
        setTestingMsg(res.uname ? `${res.message} (${res.uname})` : res.message);
      } else {
        setTestingStatus("failed");
        setTestingMsg(res.message);
      }
    } catch (err) {
      setTestingStatus("failed");
      const msg =
        err instanceof RemoteConnectionValidationError
          ? t(`ssh:validation.${err.field}`)
          : err instanceof Error
            ? err.message
            : String(err);
      setTestingMsg(msg);
      setError(msg);
    }
  };

  // 打开远程目录选择器
  const handleOpenFolderPicker = async () => {
    setError(null);
    try {
      let targetId = selectedHostId;
      if (isNewHost) {
        const saved = await saveNewHostConfig();
        targetId = saved.id;
      }
      if (!targetId || targetId === NEW_HOST_VALUE) {
        setError(t("ssh:validation.config"));
        return;
      }
      setPickerOpen(true);
    } catch (err) {
      const msg =
        err instanceof RemoteConnectionValidationError
          ? t(`ssh:validation.${err.field}`)
          : err instanceof Error
            ? err.message
            : String(err);
      setError(msg);
    }
  };

  // 从选择器选定目录回调
  const handleSelectDirectory = (chosenPath: string) => {
    handleRemotePathChange(chosenPath);
  };

  // 最终提交创建远程工作区
  const submit = async () => {
    if (submitting.current) return;
    submitting.current = true;
    setBusy(true);
    setError(null);

    const current = generation.current;
    const isCurrent = () => generation.current === current;

    try {
      const projectName =
        name.trim() || extractProjectName(remotePath) || (isNewHost ? form.name.trim() : "");
      if (!projectName) {
        throw new RemoteConnectionValidationError("requiredFields");
      }

      const completed = await submitRemoteConnection(
        {
          name: projectName,
          remotePath,
          sshConfigId: isNewHost ? undefined : selectedHostId,
          newConfig: isNewHost ? form : undefined,
        },
        {
          createSshConfig,
          testSshConnection,
          createWorkspace: create,
          isCurrent,
          onConfigCreated: (config) => {
            if (!isCurrent()) return;
            configRequest.current += 1;
            setConfigs((items) => [config, ...items.filter((item) => item.id !== config.id)]);
            setSelectedHostId(config.id);
            setForm((val) => ({ ...val, password: "" }));
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
    <>
      <Dialog
        open={isOpen}
        onOpenChange={(next) => {
          if (!submitting.current) onOpenChange(next);
        }}
      >
        {/* 加宽 50px：从 max-w-xl (576px) 增加到 sm:max-w-[626px] */}
        <DialogContent className="sm:max-w-[626px]">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-base font-semibold">
              <Server className="h-5 w-5 text-primary" />
              {t("git:remoteConnect")}
            </DialogTitle>
          </DialogHeader>

          <fieldset disabled={busy} className="space-y-4 py-1" aria-busy={busy}>
            {/* 卡片 1: SSH 服务器配置 */}
            <div className="rounded-lg border bg-muted/20 p-3.5 space-y-3">
              <div className="flex items-center justify-between">
                <label className="text-xs font-semibold text-foreground/90 flex items-center gap-1.5">
                  <Server className="h-3.5 w-3.5 text-primary" />
                  {t("ssh:serverSection")}
                </label>
                <div className="flex items-center gap-2">
                  {testingStatus === "testing" && (
                    <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
                      <Loader2 className="h-3 w-3 animate-spin" />
                      {t("ssh:testConnectionTesting")}
                    </span>
                  )}
                  {testingStatus === "passed" && (
                    <span className="flex items-center gap-1 text-[11px] text-emerald-600 font-medium">
                      <CheckCircle2 className="h-3.5 w-3.5" />
                      {t("ssh:testConnectionSuccess")}
                    </span>
                  )}
                  {testingStatus === "failed" && (
                    <span
                      className="flex items-center gap-1 text-[11px] text-destructive font-medium truncate max-w-[180px]"
                      title={testingMsg ?? ""}
                    >
                      <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                      {t("ssh:testConnectionFailed")}
                    </span>
                  )}
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-7 text-xs px-2.5 cursor-pointer"
                    disabled={busy || testingStatus === "testing"}
                    onClick={() => void handleTestConnection()}
                  >
                    {testingStatus === "testing" ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                    ) : null}
                    {t("ssh:testConnection")}
                  </Button>
                </div>
              </div>

              {/* 统一和首页思考等级一样的 DropdownMenu 下拉选择器 */}
              <div className="space-y-1">
                <DropdownMenu>
                  <DropdownMenuTrigger
                    disabled={busy}
                    className="flex h-9 w-full cursor-pointer items-center justify-between gap-2 rounded-lg border border-border/70 bg-background/80 px-3 text-sm font-medium text-foreground/90 shadow-2xs transition-colors duration-100 outline-none hover:bg-muted/40 disabled:opacity-60"
                  >
                    <div className="flex items-center gap-2 truncate">
                      {isNewHost ? (
                        <>
                          <Plus className="h-4 w-4 text-primary shrink-0" />
                          <span>{t("ssh:addNewHost")}</span>
                        </>
                      ) : (
                        <>
                          <HardDrive className="h-4 w-4 text-primary shrink-0" />
                          <span className="font-medium truncate">
                            {currentConfig?.name || selectedHostId}
                          </span>
                          <span className="text-xs text-muted-foreground truncate">
                            ({currentConfig?.username}@{currentConfig?.host}:{currentConfig?.port})
                          </span>
                        </>
                      )}
                    </div>
                    <ChevronDown className="size-3.5 shrink-0 text-muted-foreground/70" />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent
                    align="start"
                    side="bottom"
                    sideOffset={4}
                    className="w-(--anchor-width) min-w-0 duration-75 data-open:zoom-in-100 data-[side=bottom]:slide-in-from-top-0"
                  >
                    <DropdownMenuRadioGroup
                      value={selectedHostId}
                      onValueChange={(next) => next && handleHostChange(next)}
                    >
                      {configs.map((config) => (
                        <DropdownMenuRadioItem
                          key={config.id}
                          value={config.id}
                          closeOnClick
                          className="items-start py-2 cursor-pointer"
                        >
                          <HardDrive className="size-4 shrink-0 mt-0.5 text-muted-foreground" />
                          <span className="flex min-w-0 flex-col gap-0.5">
                            <span className="font-medium text-sm text-foreground">
                              {config.name}
                            </span>
                            <span className="text-xs text-muted-foreground font-mono">
                              {config.username}@{config.host}:{config.port}
                            </span>
                          </span>
                        </DropdownMenuRadioItem>
                      ))}
                      {configs.length > 0 && <DropdownMenuSeparator />}
                      <DropdownMenuRadioItem
                        value={NEW_HOST_VALUE}
                        closeOnClick
                        className="items-center py-2 text-primary font-medium cursor-pointer"
                      >
                        <Plus className="size-4 shrink-0 mr-1.5 text-primary" />
                        <span>{t("ssh:addNewHost")}</span>
                      </DropdownMenuRadioItem>
                    </DropdownMenuRadioGroup>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>

              {/* 新建主机表单展开区域 */}
              {isNewHost && (
                <div className="pt-2 border-t space-y-2.5 animate-in fade-in duration-200">
                  <div className="grid grid-cols-2 gap-2.5">
                    <div className="space-y-1">
                      <span className="text-[11px] text-muted-foreground font-medium">
                        {t("ssh:name")} *
                      </span>
                      <Input
                        placeholder="例如：新加坡测试机"
                        value={form.name}
                        onChange={(e) => setForm({ ...form, name: e.target.value })}
                        className="h-8 text-xs"
                      />
                    </div>
                    <div className="space-y-1">
                      <span className="text-[11px] text-muted-foreground font-medium">
                        {t("ssh:host")} *
                      </span>
                      <Input
                        placeholder="IP 或域名"
                        value={form.host}
                        onChange={(e) => setForm({ ...form, host: e.target.value })}
                        className="h-8 text-xs"
                      />
                    </div>
                  </div>

                  <div className="grid grid-cols-2 gap-2.5">
                    <div className="space-y-1">
                      <span className="text-[11px] text-muted-foreground font-medium">
                        {t("ssh:username")} *
                      </span>
                      <Input
                        placeholder="例如：root / ubuntu"
                        value={form.username}
                        onChange={(e) => setForm({ ...form, username: e.target.value })}
                        className="h-8 text-xs"
                      />
                    </div>
                    <div className="space-y-1">
                      <span className="text-[11px] text-muted-foreground font-medium">
                        {t("ssh:port")} *
                      </span>
                      <Input
                        placeholder="22"
                        value={String(form.port ?? 22)}
                        onChange={(e) =>
                          setForm({ ...form, port: Number(e.target.value.replace(/\D/g, "")) })
                        }
                        className="h-8 text-xs"
                      />
                    </div>
                  </div>

                  <div className="grid grid-cols-2 gap-2.5">
                    {/* 认证方式：同样采用统一风格的 DropdownMenu */}
                    <div className="space-y-1">
                      <span className="text-[11px] text-muted-foreground font-medium">
                        {t("ssh:authType")}
                      </span>
                      <DropdownMenu>
                        <DropdownMenuTrigger
                          disabled={busy}
                          className="flex h-8 w-full cursor-pointer items-center justify-between gap-2 rounded-lg border border-border/70 bg-background/80 px-2.5 text-xs font-medium text-foreground/90 shadow-2xs transition-colors duration-100 outline-none hover:bg-muted/40 disabled:opacity-60"
                        >
                          <div className="flex items-center gap-1.5 truncate">
                            {form.auth_type === "key" ? (
                              <>
                                <Key className="h-3.5 w-3.5 text-primary shrink-0" />
                                <span>{t("ssh:key")}</span>
                              </>
                            ) : (
                              <>
                                <ShieldCheck className="h-3.5 w-3.5 text-primary shrink-0" />
                                <span>{t("ssh:password")}</span>
                              </>
                            )}
                          </div>
                          <ChevronDown className="size-3 shrink-0 text-muted-foreground/70" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent
                          align="start"
                          side="bottom"
                          sideOffset={4}
                          className="w-(--anchor-width) min-w-0 duration-75 data-open:zoom-in-100 data-[side=bottom]:slide-in-from-top-0"
                        >
                          <DropdownMenuRadioGroup
                            value={form.auth_type}
                            onValueChange={(next) =>
                              next && setForm({ ...form, auth_type: next as "key" | "password" })
                            }
                          >
                            <DropdownMenuRadioItem
                              value="key"
                              closeOnClick
                              className="py-1.5 text-xs cursor-pointer"
                            >
                              <Key className="size-3.5 shrink-0 text-muted-foreground mr-1.5" />
                              {t("ssh:key")}
                            </DropdownMenuRadioItem>
                            <DropdownMenuRadioItem
                              value="password"
                              closeOnClick
                              className="py-1.5 text-xs cursor-pointer"
                            >
                              <ShieldCheck className="size-3.5 shrink-0 text-muted-foreground mr-1.5" />
                              {t("ssh:password")}
                            </DropdownMenuRadioItem>
                          </DropdownMenuRadioGroup>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </div>

                    <div className="space-y-1">
                      <span className="text-[11px] text-muted-foreground font-medium">
                        {form.auth_type === "key" ? t("ssh:privateKey") : t("ssh:password")} *
                      </span>
                      {form.auth_type === "key" ? (
                        <div className="flex gap-1">
                          <Input
                            placeholder="~/.ssh/id_rsa"
                            value={form.private_key_path ?? ""}
                            onChange={(e) => setForm({ ...form, private_key_path: e.target.value })}
                            className="h-8 text-xs font-mono truncate"
                          />
                          <Button
                            type="button"
                            variant="outline"
                            size="icon"
                            className="h-8 w-8 shrink-0 cursor-pointer"
                            onClick={() => {
                              void open({ multiple: false }).then((path) => {
                                if (typeof path === "string") {
                                  setForm((current) => ({ ...current, private_key_path: path }));
                                }
                              });
                            }}
                            title={t("ssh:privateKey")}
                          >
                            <Key className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      ) : (
                        <Input
                          type="password"
                          placeholder="请输入密码"
                          value={form.password ?? ""}
                          onChange={(e) => setForm({ ...form, password: e.target.value })}
                          className="h-8 text-xs"
                        />
                      )}
                    </div>
                  </div>
                </div>
              )}
            </div>

            {/* 卡片 2: 工作区与目录配置 */}
            <div className="rounded-lg border bg-muted/20 p-3.5 space-y-3">
              <label className="text-xs font-semibold text-foreground/90 flex items-center gap-1.5">
                <FolderTree className="h-3.5 w-3.5 text-primary" />
                {t("ssh:workspaceSection")}
              </label>

              {/* 远端仓库路径与目录选择按钮 */}
              <div className="space-y-1">
                <span className="text-[11px] text-muted-foreground font-medium">
                  {t("ssh:remotePath")} *
                </span>
                <div className="flex items-center gap-1.5">
                  <Input
                    placeholder={t("ssh:remotePathPlaceholder")}
                    value={remotePath}
                    onChange={(e) => handleRemotePathChange(e.target.value)}
                    className="h-9 text-xs font-mono"
                  />
                  <Button
                    type="button"
                    variant="outline"
                    size="icon"
                    className="h-9 w-9 shrink-0 text-muted-foreground hover:text-foreground cursor-pointer"
                    onClick={() => void handleOpenFolderPicker()}
                    title={t("ssh:openFolder")}
                  >
                    <FolderOpen className="h-4 w-4" />
                  </Button>
                </div>
              </div>

              {/* 项目名称输入框（解决第 4 张图的困惑） */}
              <div className="space-y-1">
                <div className="flex items-center justify-between">
                  <span className="text-[11px] text-muted-foreground font-medium">
                    {t("ssh:projectName")} *
                  </span>
                  <span className="text-[10px] text-muted-foreground">
                    {t("ssh:projectNameHint")}
                  </span>
                </div>
                <Input
                  placeholder={t("ssh:projectNamePlaceholder")}
                  value={name}
                  onChange={(e) => {
                    setName(e.target.value);
                    setNameEdited(true);
                  }}
                  className="h-9 text-xs"
                />
              </div>
            </div>

            {error ? (
              <div className="flex items-center gap-2 rounded-md bg-destructive/10 border border-destructive/20 px-3 py-2 text-xs text-destructive">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{error}</span>
              </div>
            ) : null}
          </fieldset>

          <DialogFooter className="flex items-center justify-between sm:justify-between w-full pt-1">
            <div className="text-[11px] text-muted-foreground">
              {isNewHost ? t("ssh:autoSaveHint") : ""}
            </div>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="ghost"
                disabled={busy}
                onClick={() => onOpenChange(false)}
              >
                {t("common:cancel")}
              </Button>
              <Button disabled={busy} onClick={() => void submit()}>
                {busy ? (
                  <>
                    <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />
                    {t("ssh:connecting")}
                  </>
                ) : (
                  t("ssh:openRemoteProject")
                )}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 远程目录树选择器子弹窗 */}
      <RemoteDirectoryPickerDialog
        open={pickerOpen}
        onOpenChange={setPickerOpen}
        sshConfigId={selectedHostId}
        initialPath={remotePath}
        onSelect={handleSelectDirectory}
      />
    </>
  );
}
