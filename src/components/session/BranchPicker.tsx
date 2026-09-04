import { Check, GitBranch, Plus, Search } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { checkoutGitBranch, createGitBranch, listGitBranches } from "@/lib/backend";
import type { GitBranch as GitBranchType } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useDismissible } from "@/hooks/useDismissible";
import { useWorkspaceStore } from "@/stores/workspaceStore";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function BranchPicker() {
  const { t } = useTranslation(["git", "common"]);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const [branches, setBranches] = useState<GitBranchType[]>([]);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const closeMenu = useCallback(() => setOpen(false), []);
  useDismissible(open, closeMenu, rootRef);

  const current = branches.find((item) => item.is_current);

  const loadBranches = useCallback(() => {
    if (!workspaceId) {
      setBranches([]);
      return;
    }
    void listGitBranches(workspaceId)
      .then(setBranches)
      .catch(() => setBranches([]));
  }, [workspaceId]);

  useEffect(() => {
    loadBranches();
  }, [loadBranches]);

  const filtered = useMemo(
    () => branches.filter((item) => item.name.toLowerCase().includes(query.trim().toLowerCase())),
    [branches, query],
  );

  const toggleOpen = () => {
    setOpen((value) => {
      const next = !value;
      if (next) {
        setError(null);
        loadBranches();
      }
      return next;
    });
  };

  const selectBranch = (branch: GitBranchType) => {
    if (!workspaceId || busy) return;
    if (branch.is_current) {
      setOpen(false);
      return;
    }
    setError(null);
    setBusy(true);
    void checkoutGitBranch(workspaceId, branch.name)
      .then(() => listGitBranches(workspaceId).then(setBranches))
      .then(() => setOpen(false))
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false));
  };

  if (!workspaceId) return null;

  return (
    <div ref={rootRef} className="relative inline-flex items-center">
      <button
        type="button"
        onClick={toggleOpen}
        className="inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-lg border border-border/70 bg-background/80 px-2 text-xs font-medium text-foreground/90 shadow-2xs transition-all duration-150 outline-none hover:bg-muted/40"
      >
        <GitBranch className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="max-w-32 truncate">{current?.name ?? "—"}</span>
      </button>
      {open ? (
        <div className="absolute z-30 mt-1 w-80 rounded-lg border bg-popover p-2 shadow-lg">
          <div className="mb-2 flex items-center gap-2 px-1">
            <Search className="size-3.5 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("git:searchBranch")}
              className="h-7"
            />
          </div>
          <p className="px-2 pb-1 text-[11px] text-muted-foreground">{t("git:switchHint")}</p>
          {error ? <p className="px-2 pb-1 text-[11px] text-destructive">{error}</p> : null}
          <div className="max-h-48 overflow-y-auto">
            {filtered.map((branch) => (
              <button
                key={branch.name}
                type="button"
                disabled={busy}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent disabled:opacity-50"
                onClick={() => selectBranch(branch)}
              >
                <span className="flex-1 truncate">{branch.name}</span>
                {branch.is_current ? <Check className="size-3.5" /> : null}
              </button>
            ))}
          </div>
          <div className="mt-2 border-t pt-2">
            {creating ? (
              <div className="flex gap-2">
                <Input
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder={t("git:branchName")}
                  className="h-7"
                />
                <Button
                  size="sm"
                  disabled={!name.trim() || busy}
                  onClick={() => {
                    setError(null);
                    setBusy(true);
                    void createGitBranch(workspaceId, name.trim(), true)
                      .then(() => listGitBranches(workspaceId).then(setBranches))
                      .then(() => {
                        setCreating(false);
                        setName("");
                        setOpen(false);
                      })
                      .catch((err: unknown) => setError(errorMessage(err)))
                      .finally(() => setBusy(false));
                  }}
                >
                  {t("common:create")}
                </Button>
              </div>
            ) : (
              <button
                type="button"
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent"
                onClick={() => setCreating(true)}
              >
                <Plus className="size-3.5" />
                {t("git:createBranch")}
              </button>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}
