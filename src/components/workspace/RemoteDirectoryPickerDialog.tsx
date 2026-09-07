import { ArrowUp, Folder, FolderOpen, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { listRemoteDirectories } from "@/lib/backend";
import type { RemoteDirectoryList } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";

interface RemoteDirectoryPickerDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sshConfigId: string;
  initialPath?: string;
  onSelect: (path: string) => void;
}

export function RemoteDirectoryPickerDialog({
  open,
  onOpenChange,
  sshConfigId,
  initialPath,
  onSelect,
}: RemoteDirectoryPickerDialogProps) {
  const { t } = useTranslation(["ssh", "common"]);
  const [data, setData] = useState<RemoteDirectoryList | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedSubDir, setSelectedSubDir] = useState<string | null>(null);
  const [pathInput, setPathInput] = useState("");
  const requestId = useRef(0);

  const fetchDirectory = useCallback(
    async (targetPath?: string) => {
      if (!sshConfigId) return;
      const req = ++requestId.current;
      setLoading(true);
      setError(null);
      setSelectedSubDir(null);

      try {
        const res = await listRemoteDirectories(sshConfigId, targetPath);
        if (req !== requestId.current) return;
        setData(res);
        setPathInput(res.current_path);
      } catch (err) {
        if (req !== requestId.current) return;
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (req === requestId.current) {
          setLoading(false);
        }
      }
    },
    [sshConfigId],
  );

  useEffect(() => {
    if (open && sshConfigId) {
      void fetchDirectory(initialPath?.trim() || undefined);
    } else {
      setData(null);
      setError(null);
      setSelectedSubDir(null);
    }
  }, [open, sshConfigId, fetchDirectory, initialPath]);

  const currentPath = (data?.current_path ?? "").trim();
  const effectiveSelectedPath = selectedSubDir
    ? currentPath === "/"
      ? `/${selectedSubDir}`
      : `${currentPath.replace(/\/+$/, "")}/${selectedSubDir}`
    : currentPath;

  const handleConfirm = () => {
    if (effectiveSelectedPath) {
      onSelect(effectiveSelectedPath);
      onOpenChange(false);
    }
  };

  const handleGoParent = () => {
    if (data?.parent_path) {
      void fetchDirectory(data.parent_path);
    }
  };

  const handleEnterSubDir = (dirName: string) => {
    const next =
      currentPath === "/" ? `/${dirName}` : `${currentPath.replace(/\/+$/, "")}/${dirName}`;
    void fetchDirectory(next);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[626px] flex flex-col max-h-[85vh]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-base font-semibold">
            <FolderOpen className="h-5 w-5 text-primary" />
            {t("ssh:selectRemoteDirectory")}
          </DialogTitle>
        </DialogHeader>

        <div className="flex flex-col gap-3 py-1 flex-1 overflow-hidden">
          {/* 路径栏与上一级 */}
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="icon"
              className="h-8 w-8 shrink-0"
              disabled={loading || !data?.parent_path}
              onClick={handleGoParent}
              title={t("ssh:parentDirectory")}
            >
              <ArrowUp className="h-4 w-4" />
            </Button>
            <Input
              className="h-8 text-xs font-mono"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  void fetchDirectory(pathInput.trim() || undefined);
                }
              }}
              placeholder="/"
            />
            <Button
              type="button"
              variant="outline"
              size="icon"
              className="h-8 w-8 shrink-0"
              disabled={loading}
              onClick={() => void fetchDirectory(pathInput.trim() || currentPath || undefined)}
              title={t("common:refresh")}
            >
              <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
            </Button>
          </div>

          {/* 目录列表 */}
          <div className="rounded-md border bg-muted/20 flex-1 min-h-[260px] max-h-[360px] overflow-hidden flex flex-col">
            {loading ? (
              <div className="flex-1 flex flex-col items-center justify-center gap-2 text-muted-foreground py-12">
                <Loader2 className="h-6 w-6 animate-spin text-primary" />
                <span className="text-xs">{t("ssh:connecting")}</span>
              </div>
            ) : error ? (
              <div className="flex-1 flex flex-col items-center justify-center p-6 text-center text-xs text-destructive gap-3">
                <p className="max-w-md">{error}</p>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => void fetchDirectory(undefined)}
                >
                  {t("ssh:parentDirectory")} (~)
                </Button>
              </div>
            ) : data && data.directories.length === 0 ? (
              <div className="flex-1 flex items-center justify-center text-xs text-muted-foreground">
                {t("ssh:emptyDirectory")}
              </div>
            ) : data ? (
              <ScrollArea className="h-[280px]">
                <div className="p-1.5 space-y-0.5">
                  {data.directories.map((dir) => {
                    const isSelected = selectedSubDir === dir;
                    return (
                      <div
                        key={dir}
                        onClick={() => setSelectedSubDir(dir)}
                        onDoubleClick={() => handleEnterSubDir(dir)}
                        className={`flex items-center gap-2.5 px-3 py-1.5 rounded-md text-xs cursor-pointer select-none transition-colors ${
                          isSelected
                            ? "bg-primary/15 text-primary font-medium"
                            : "hover:bg-muted/70 text-foreground"
                        }`}
                      >
                        <Folder
                          className={`h-4 w-4 shrink-0 ${
                            isSelected ? "text-primary fill-primary/20" : "text-muted-foreground"
                          }`}
                        />
                        <span className="truncate">{dir}</span>
                      </div>
                    );
                  })}
                </div>
              </ScrollArea>
            ) : null}
          </div>

          {/* 当前选定绝对路径展示 */}
          <div className="flex items-center justify-between text-xs text-muted-foreground px-1">
            <div className="flex items-center gap-1.5 truncate">
              <span className="shrink-0">{t("ssh:currentPath")}:</span>
              <span className="font-mono text-foreground truncate select-all">
                {effectiveSelectedPath || "-"}
              </span>
            </div>
            {selectedSubDir && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-6 text-[11px] px-2 text-primary"
                onClick={() => setSelectedSubDir(null)}
              >
                取消选中子目录
              </Button>
            )}
          </div>
        </div>

        <DialogFooter className="mt-2">
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button
            type="button"
            disabled={loading || !effectiveSelectedPath}
            onClick={handleConfirm}
          >
            {t("ssh:selectThisDirectory")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
