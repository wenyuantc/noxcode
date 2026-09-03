import { Check, GitBranch, Plus, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { createGitBranch, listGitBranches } from "@/lib/backend";
import type { GitBranch as GitBranchType } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function BranchPicker() {
  const { t } = useTranslation(["git", "common"]);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const [branches, setBranches] = useState<GitBranchType[]>([]);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");

  const current = branches.find((item) => item.is_current);

  useEffect(() => {
    if (!workspaceId) {
      setBranches([]);
      return;
    }
    void listGitBranches(workspaceId)
      .then(setBranches)
      .catch(() => setBranches([]));
  }, [workspaceId]);

  const filtered = useMemo(
    () => branches.filter((item) => item.name.toLowerCase().includes(query.trim().toLowerCase())),
    [branches, query],
  );

  if (!workspaceId) return null;

  return (
    <div className="relative">
      <Button type="button" variant="outline" size="sm" onClick={() => setOpen((value) => !value)}>
        <GitBranch className="size-3.5" />
        <span className="max-w-32 truncate">{current?.name ?? "—"}</span>
      </Button>
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
          <div className="max-h-48 overflow-y-auto">
            {filtered.map((branch) => (
              <div
                key={branch.name}
                className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm"
              >
                <span className="flex-1 truncate">{branch.name}</span>
                {branch.is_current ? <Check className="size-3.5" /> : null}
              </div>
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
                  disabled={!name.trim()}
                  onClick={() => {
                    void createGitBranch(workspaceId, name.trim(), true).then(() =>
                      listGitBranches(workspaceId).then(setBranches),
                    );
                    setCreating(false);
                    setName("");
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
