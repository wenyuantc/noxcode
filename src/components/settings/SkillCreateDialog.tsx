import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { createNativeSkill } from "@/lib/backend";
import { shortWorkspaceDirLabel, workspaceSkillPath } from "@/lib/skillWorkspaceDir";
import type { Workspace } from "@/lib/types";
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
import { Textarea } from "@/components/ui/textarea";
import { useWorkspaceStore } from "@/stores/workspaceStore";

const SCOPE_GLOBAL = "global";

interface SkillCreateDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}

export function SkillCreateDialog({ open, onOpenChange, onCreated }: SkillCreateDialogProps) {
  const { t } = useTranslation(["settings", "common"]);
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [target, setTarget] = useState(SCOPE_GLOBAL);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const localWorkspaces = useMemo(
    () =>
      workspaces
        .filter((workspace) => workspace.workspace_type === "local" && workspace.repo_path?.trim())
        .sort((left, right) => left.name.localeCompare(right.name, undefined, { numeric: true })),
    [workspaces],
  );

  const reset = () => {
    setName("");
    setDescription("");
    setTarget(SCOPE_GLOBAL);
    setError(null);
  };

  const workspaceLabel = (workspace: Workspace) => {
    const path = workspaceSkillPath(workspace);
    return path ? `${workspace.name} · ${shortWorkspaceDirLabel(path)}` : workspace.name;
  };

  const targetLabel = (value: string) => {
    if (value === SCOPE_GLOBAL) return t("skills.create.scopeGlobal");
    const workspace = localWorkspaces.find((item) => item.id === value);
    return workspace ? workspaceLabel(workspace) : value;
  };

  const submit = async () => {
    const workspaceId = target === SCOPE_GLOBAL ? null : target;
    if (workspaceId && !localWorkspaces.some((workspace) => workspace.id === workspaceId)) {
      setError(t("skills.create.needWorkspace"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await createNativeSkill({
        scope: workspaceId ? "project" : "global",
        name: name.trim(),
        description: description.trim(),
        workspace_id: workspaceId,
      });
      reset();
      onOpenChange(false);
      onCreated();
    } catch (err) {
      setError(err instanceof Error ? err.message : t("skills.create.failed"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) reset();
        onOpenChange(next);
      }}
    >
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("skills.create.title")}</DialogTitle>
          <DialogDescription>{t("skills.create.nameHint")}</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div>
            <label className="text-xs font-medium text-muted-foreground">
              {t("skills.create.name")}
            </label>
            <Input
              className="mt-1"
              value={name}
              disabled={busy}
              onChange={(event) => setName(event.target.value)}
              placeholder="code-review"
            />
          </div>
          <div>
            <label className="text-xs font-medium text-muted-foreground">
              {t("skills.create.description")}
            </label>
            <Textarea
              className="mt-1 min-h-20"
              value={description}
              disabled={busy}
              onChange={(event) => setDescription(event.target.value)}
            />
          </div>
          <div>
            <label className="text-xs font-medium text-muted-foreground">
              {t("skills.create.scope")}
            </label>
            <Select
              value={target}
              disabled={busy}
              onValueChange={(value) => {
                if (typeof value === "string") setTarget(value);
              }}
            >
              <SelectTrigger className="mt-1 bg-background">
                <SelectValue>{() => targetLabel(target)}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={SCOPE_GLOBAL}>{t("skills.create.scopeGlobal")}</SelectItem>
                {localWorkspaces.map((workspace) => (
                  <SelectItem
                    key={workspace.id}
                    value={workspace.id}
                    title={workspaceSkillPath(workspace) || workspace.name}
                  >
                    {workspaceLabel(workspace)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
        </div>
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button
            disabled={busy || !name.trim() || !description.trim()}
            onClick={() => void submit()}
          >
            {t("skills.create.submit")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
