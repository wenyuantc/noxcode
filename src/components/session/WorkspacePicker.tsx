import { open } from "@tauri-apps/plugin-dialog";
import { Check, Cloud, FolderOpen, Search, X } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { ensureScratchWorkspace } from "@/lib/backend";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RemoteConnectDialog } from "@/components/workspace/RemoteConnectDialog";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function WorkspacePicker({ onRequestOpen }: { onRequestOpen?: () => void }) {
  const { t } = useTranslation("git");
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const activeId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const setActive = useWorkspaceStore((state) => state.setActive);
  const create = useWorkspaceStore((state) => state.create);
  const active = workspaces.find((item) => item.id === activeId);
  const [openMenu, setOpenMenu] = useState(false);
  const [query, setQuery] = useState("");
  const [remoteOpen, setRemoteOpen] = useState(false);

  const filtered = useMemo(
    () => workspaces.filter((item) => item.name.toLowerCase().includes(query.trim().toLowerCase())),
    [workspaces, query],
  );

  const openFolder = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    const name = selected.split("/").filter(Boolean).pop() ?? selected;
    await create({ name, workspace_type: "local", repo_path: selected });
    setOpenMenu(false);
  };

  return (
    <div className="relative">
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={() => {
          setOpenMenu((value) => !value);
          onRequestOpen?.();
        }}
      >
        <FolderOpen className="size-3.5" />
        <span className="max-w-40 truncate">{active?.name ?? t("searchWorkspace")}</span>
      </Button>
      {active ? (
        <button
          type="button"
          className="ml-1 text-muted-foreground"
          title={t("clearWorkspace")}
          onClick={() => void setActive(null)}
        >
          <X className="size-3.5" />
        </button>
      ) : null}
      {openMenu ? (
        <div className="absolute z-30 mt-1 w-72 rounded-lg border bg-popover p-2 shadow-lg">
          <div className="mb-2 flex items-center gap-2 px-1">
            <Search className="size-3.5 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("searchWorkspace")}
              className="h-7"
            />
          </div>
          <div className="max-h-56 overflow-y-auto">
            {filtered.map((workspace) => (
              <button
                key={workspace.id}
                type="button"
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent"
                onClick={() => {
                  void setActive(workspace.id);
                  setOpenMenu(false);
                }}
              >
                <span className="flex-1 truncate text-left">{workspace.name}</span>
                {workspace.id === activeId ? <Check className="size-3.5" /> : null}
              </button>
            ))}
          </div>
          <div className="mt-2 space-y-1 border-t pt-2">
            <button
              type="button"
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent"
              onClick={() => void openFolder()}
            >
              <FolderOpen className="size-3.5" />
              {t("openFolder")}
            </button>
            <button
              type="button"
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent"
              onClick={() => {
                setRemoteOpen(true);
                setOpenMenu(false);
              }}
            >
              <Cloud className="size-3.5" />
              {t("remoteConnect")}
            </button>
            <button
              type="button"
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent"
              onClick={() => {
                void ensureScratchWorkspace().then((workspace) =>
                  useWorkspaceStore
                    .getState()
                    .load()
                    .then(() => setActive(workspace.id)),
                );
                setOpenMenu(false);
              }}
            >
              {t("noProject")}
            </button>
          </div>
        </div>
      ) : null}
      <RemoteConnectDialog open={remoteOpen} onOpenChange={setRemoteOpen} />
    </div>
  );
}
