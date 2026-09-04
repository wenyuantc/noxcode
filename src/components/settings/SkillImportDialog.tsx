import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { importExternalSkills, scanExternalSkills } from "@/lib/backend";
import type { ExternalSkillGroup, ImportExternalSkillsResult } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

interface SkillImportDialogProps {
  open: boolean;
  workspaceId: string | null;
  onOpenChange: (open: boolean) => void;
  onImported: () => void;
}

export function SkillImportDialog({
  open,
  workspaceId,
  onOpenChange,
  onImported,
}: SkillImportDialogProps) {
  const { t } = useTranslation(["settings", "common"]);
  const [groups, setGroups] = useState<ExternalSkillGroup[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [target, setTarget] = useState<"global" | "project">("global");
  const [mode, setMode] = useState<"copy" | "symlink">("copy");
  const [scanning, setScanning] = useState(false);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ImportExternalSkillsResult | null>(null);

  const importable = useMemo(
    () => groups.flatMap((group) => group.skills.filter((skill) => skill.importable)),
    [groups],
  );

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setScanning(true);
    setError(null);
    setResult(null);
    void scanExternalSkills(workspaceId)
      .then((doc) => {
        if (cancelled) return;
        setGroups(doc.groups);
        setSelected(
          new Set(
            doc.groups.flatMap((group) =>
              group.skills.filter((skill) => skill.importable).map((skill) => skill.source_path),
            ),
          ),
        );
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setScanning(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, workspaceId]);

  const toggle = (path: string, next: boolean) => {
    setSelected((current) => {
      const copy = new Set(current);
      if (next) copy.add(path);
      else copy.delete(path);
      return copy;
    });
  };

  const submit = async () => {
    if (target === "project" && !workspaceId) {
      setError(t("skills.import.needWorkspace"));
      return;
    }
    const items = importable
      .filter((skill) => selected.has(skill.source_path))
      .map((skill) => ({ source_path: skill.source_path, name: skill.name }));
    setImporting(true);
    setError(null);
    try {
      const imported = await importExternalSkills({
        workspace_id: workspaceId,
        target,
        mode,
        items,
      });
      setResult(imported);
      onImported();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setImporting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("skills.import.title")}</DialogTitle>
          <DialogDescription>{t("skills.import.description")}</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="text-xs font-medium text-muted-foreground">
                {t("skills.import.target")}
              </label>
              <Select
                value={target}
                disabled={importing}
                onValueChange={(value) => {
                  if (value === "global" || value === "project") setTarget(value);
                }}
              >
                <SelectTrigger className="mt-1 bg-background">
                  <SelectValue>
                    {() =>
                      target === "project"
                        ? t("skills.import.targetProject")
                        : t("skills.import.targetGlobal")
                    }
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="global">{t("skills.import.targetGlobal")}</SelectItem>
                  <SelectItem value="project">{t("skills.import.targetProject")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <label className="text-xs font-medium text-muted-foreground">
                {t("skills.import.mode")}
              </label>
              <Select
                value={mode}
                disabled={importing}
                onValueChange={(value) => {
                  if (value === "copy" || value === "symlink") setMode(value);
                }}
              >
                <SelectTrigger className="mt-1 bg-background">
                  <SelectValue>
                    {() =>
                      mode === "symlink"
                        ? t("skills.import.modeSymlink")
                        : t("skills.import.modeCopy")
                    }
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="copy">{t("skills.import.modeCopy")}</SelectItem>
                  <SelectItem value="symlink">{t("skills.import.modeSymlink")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <p className="text-[11px] text-muted-foreground">
            {mode === "symlink"
              ? t("skills.import.modeSymlinkHint")
              : t("skills.import.modeCopyHint")}
          </p>
          {scanning ? (
            <p className="text-xs text-muted-foreground">{t("skills.import.scanning")}</p>
          ) : importable.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t("skills.import.empty")}</p>
          ) : (
            <div className="max-h-64 space-y-3 overflow-y-auto rounded-xl border border-border/60 p-2">
              <div className="flex items-center justify-between px-1">
                <p className="text-[11px] text-muted-foreground">
                  {t("skills.import.summary", { count: importable.length })}
                </p>
                <div className="flex gap-1">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 px-2 text-[11px]"
                    onClick={() =>
                      setSelected(new Set(importable.map((skill) => skill.source_path)))
                    }
                  >
                    {t("skills.import.selectAll")}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 px-2 text-[11px]"
                    onClick={() => setSelected(new Set())}
                  >
                    {t("skills.import.clearAll")}
                  </Button>
                </div>
              </div>
              {groups.map((group) => (
                <div key={group.id}>
                  <p className="px-1 text-[11px] font-medium text-muted-foreground">
                    {group.label}
                  </p>
                  {group.skills.map((skill) => (
                    <label
                      key={skill.source_path}
                      className="mt-1 flex items-start gap-2 rounded-lg px-1.5 py-1 text-xs"
                    >
                      <input
                        type="checkbox"
                        className="mt-0.5"
                        disabled={!skill.importable || importing}
                        checked={selected.has(skill.source_path)}
                        onChange={(event) => toggle(skill.source_path, event.target.checked)}
                      />
                      <span className="min-w-0">
                        <span className="font-medium">{skill.name}</span>
                        <span className="ml-2 text-muted-foreground">
                          {skill.importable ? skill.description : t("skills.import.skipExists")}
                        </span>
                      </span>
                    </label>
                  ))}
                </div>
              ))}
            </div>
          )}
          {result ? (
            <p className="text-xs text-muted-foreground">
              {t("skills.import.complete")} {t("skills.import.imported")} {result.imported.length} ·{" "}
              {t("skills.import.skipped")} {result.skipped.length} · {t("skills.import.failed")}{" "}
              {result.failed.length}
            </p>
          ) : null}
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
        </div>
        <DialogFooter>
          <Button variant="outline" disabled={importing} onClick={() => onOpenChange(false)}>
            {result ? t("skills.import.finish") : t("common:cancel")}
          </Button>
          <Button
            disabled={importing || scanning || selected.size === 0}
            onClick={() => void submit()}
          >
            {importing ? t("skills.import.importing") : t("skills.import.start")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
