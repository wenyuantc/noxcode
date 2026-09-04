import { Check, Cloud, FolderOpen, Search, X } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { ensureScratchWorkspace } from "@/lib/backend";
import { openLocalWorkspace } from "@/lib/openLocalWorkspace";
import { Input } from "@/components/ui/input";
import { RemoteConnectDialog } from "@/components/workspace/RemoteConnectDialog";
import { useDismissible } from "@/hooks/useDismissible";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function WorkspacePicker({ onRequestOpen }: { onRequestOpen?: () => void }) {
  const { t } = useTranslation("git");
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const activeId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const setActive = useWorkspaceStore((state) => state.setActive);
  const active = workspaces.find((item) => item.id === activeId);
  const [openMenu, setOpenMenu] = useState(false);
  const [query, setQuery] = useState("");
  const [remoteOpen, setRemoteOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const closeMenu = useCallback(() => setOpenMenu(false), []);
  useDismissible(openMenu, closeMenu, rootRef);

  const filtered = useMemo(
    () => workspaces.filter((item) => item.name.toLowerCase().includes(query.trim().toLowerCase())),
    [workspaces, query],
  );

  const openFolder = async () => {
    if (await openLocalWorkspace()) setOpenMenu(false);
  };

  return (
    <div ref={rootRef} className="relative inline-flex items-center">
      <div className="group/wp inline-flex h-7 items-center rounded-lg border border-border/70 bg-background/80 px-2 text-xs font-medium text-foreground/90 shadow-2xs transition-all duration-150 hover:bg-muted/40">
        <button
          type="button"
          className="flex cursor-pointer items-center gap-1.5 outline-none"
          onClick={() => {
            setOpenMenu((value) => !value);
            onRequestOpen?.();
          }}
        >
          <FolderOpen className="size-3.5 shrink-0 text-muted-foreground" />
          <span className="max-w-40 truncate">{active?.name ?? t("searchWorkspace")}</span>
        </button>
        {active ? (
          <button
            type="button"
            className="ml-1.5 -mr-0.5 cursor-pointer rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-muted hover:text-foreground"
            title={t("clearWorkspace")}
            aria-label={t("clearWorkspace")}
            onClick={(event) => {
              event.stopPropagation();
              void setActive(null);
            }}
          >
            <X className="size-3" />
          </button>
        ) : null}
      </div>
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
