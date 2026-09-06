import {
  Archive,
  ArchiveRestore,
  Check,
  Copy,
  FolderOpen,
  Hash,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  X,
} from "lucide-react";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

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
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { openAgentSessionDirectory } from "@/lib/backend";
import { isSessionBusy, resolveSessionDirectory } from "@/lib/sessionActions";
import { isMac } from "@/lib/shortcuts";
import type { AgentSession } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function SessionMenu({
  session,
  children,
}: {
  session: AgentSession;
  children?: (button: ReactNode) => ReactNode;
}) {
  const { t } = useTranslation(["layout", "common"]);
  const triggerId = useId();
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const pending = useWorkspaceStore((state) => Boolean(state.sessionMutations[session.id]));
  const working = useSessionStore((state) => isSessionBusy(session.id, state));
  const { path, remote } = resolveSessionDirectory(session, workspaces);
  const archived = Boolean(session.archived);
  const pinned = Boolean(session.pinned);
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<{ getBoundingClientRect: () => DOMRect }>();
  const returnFocus = useRef<HTMLElement | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [name, setName] = useState("");
  const [renameError, setRenameError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<{ text: string; error: boolean } | null>(null);

  useEffect(() => {
    if (!feedback || feedback.error) return;
    const timer = window.setTimeout(() => setFeedback(null), 2000);
    return () => window.clearTimeout(timer);
  }, [feedback]);

  const run = async (action: () => Promise<unknown>, copied = false) => {
    try {
      await action();
      if (copied) setFeedback({ text: t("common:copied"), error: false });
    } catch (error) {
      setFeedback({ text: error instanceof Error ? error.message : String(error), error: true });
    }
  };

  const openAt = (x: number, y: number, target: HTMLElement) => {
    returnFocus.current = target;
    setAnchor({ getBoundingClientRect: () => new DOMRect(x, y, 0, 0) });
    setOpen(true);
  };

  const button = (
    <DropdownMenuTrigger
      id={triggerId}
      render={<Button size="icon-sm" variant="ghost" />}
      className={cn(
        "size-7 shrink-0 text-muted-foreground hover:text-foreground",
        children &&
          "size-5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 data-popup-open:opacity-100",
      )}
      title={t("sessionMenu.more")}
      aria-label={t("sessionMenu.more")}
      onClick={(event) => event.stopPropagation()}
    >
      <MoreHorizontal className="size-3.5" />
    </DropdownMenuTrigger>
  );

  const disabledReason = archived ? t("sessionMenu.restoreFirst") : t("sessionMenu.busy");

  return (
    <>
      <DropdownMenu
        open={open}
        triggerId={triggerId}
        onOpenChange={(next, details) => {
          if (next && details.reason === "trigger-press") setAnchor(undefined);
          setOpen(next);
        }}
      >
        <div
          className="contents"
          onContextMenu={(event) => {
            if (!children || !event.currentTarget.contains(event.target as Node)) return;
            event.preventDefault();
            event.stopPropagation();
            const row = (event.target as HTMLElement).closest<HTMLElement>("[data-session-row]");
            if (row) openAt(event.clientX, event.clientY, row);
          }}
          onKeyDown={(event) => {
            if (
              !children ||
              (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10"))
            )
              return;
            if (!event.currentTarget.contains(event.target as Node)) return;
            const row = (event.target as HTMLElement).closest<HTMLElement>("[data-session-row]");
            if (!row) return;
            event.preventDefault();
            event.stopPropagation();
            const rect = row.getBoundingClientRect();
            openAt(rect.left + 12, rect.bottom, row);
          }}
        >
          {children ? children(button) : button}
        </div>
        <DropdownMenuContent
          anchor={anchor}
          align={children ? "start" : "end"}
          className="w-56 max-w-[calc(100vw-16px)]"
          finalFocus={renameOpen ? false : anchor ? returnFocus : true}
          onClick={(event) => event.stopPropagation()}
          onKeyDown={(event) => event.stopPropagation()}
        >
          <div title={archived ? disabledReason : undefined}>
            <DropdownMenuItem
              disabled={pending || archived}
              onClick={() =>
                void run(() => useWorkspaceStore.getState().setSessionPinned(session.id, !pinned))
              }
            >
              {pinned ? <PinOff /> : <Pin />}
              {t(pinned ? "sessionMenu.unpin" : "sessionMenu.pin")}
            </DropdownMenuItem>
          </div>
          <DropdownMenuItem
            disabled={pending}
            onClick={() => {
              setName(session.title ?? "");
              setRenameError(null);
              setRenameOpen(true);
            }}
          >
            <Pencil />
            {t("sessionMenu.rename")}
          </DropdownMenuItem>
          <div title={!archived && working ? disabledReason : undefined}>
            <DropdownMenuItem
              disabled={pending || (!archived && working)}
              onClick={() =>
                void run(() =>
                  useWorkspaceStore.getState().setSessionArchived(session.id, !archived),
                )
              }
            >
              {archived ? <ArchiveRestore /> : <Archive />}
              {t(archived ? "sessionMenu.restore" : "sessionMenu.archive")}
            </DropdownMenuItem>
          </div>
          <DropdownMenuSeparator />
          <div
            title={
              remote
                ? t("sessionMenu.remoteDirectory")
                : !path
                  ? t("sessionMenu.noDirectory")
                  : undefined
            }
          >
            <DropdownMenuItem
              disabled={!path || remote}
              onClick={() => void run(() => openAgentSessionDirectory(session.id))}
            >
              <FolderOpen />
              {t(isMac() ? "sessionMenu.openFinder" : "sessionMenu.openDirectory")}
            </DropdownMenuItem>
          </div>
          <DropdownMenuItem
            disabled={!path}
            onClick={() => path && void run(() => navigator.clipboard.writeText(path), true)}
          >
            <Copy />
            {t("sessionMenu.copyPath")}
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() => void run(() => navigator.clipboard.writeText(session.id), true)}
          >
            <Hash />
            {t("sessionMenu.copyId")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Dialog open={renameOpen} onOpenChange={(next) => !pending && setRenameOpen(next)}>
        <DialogContent>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (pending || !name.trim()) return;
              setRenameError(null);
              void useWorkspaceStore
                .getState()
                .renameSession(session.id, name)
                .then(
                  () => setRenameOpen(false),
                  (error: unknown) =>
                    setRenameError(error instanceof Error ? error.message : String(error)),
                );
            }}
          >
            <DialogHeader>
              <DialogTitle>{t("sessionMenu.rename")}</DialogTitle>
            </DialogHeader>
            <label htmlFor={`${triggerId}-name`} className="mt-4 mb-2 block text-sm">
              {t("sessionMenu.name")}
            </label>
            <Input
              id={`${triggerId}-name`}
              autoFocus
              value={name}
              disabled={pending}
              onChange={(event) => setName(event.target.value)}
            />
            {renameError ? (
              <p role="alert" className="mt-2 text-sm break-words text-destructive">
                {renameError}
              </p>
            ) : null}
            <DialogFooter className="mt-4">
              <Button
                type="button"
                variant="outline"
                disabled={pending}
                onClick={() => setRenameOpen(false)}
              >
                {t("common:cancel")}
              </Button>
              <Button type="submit" disabled={pending || !name.trim()}>
                {t("common:save")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
      {feedback && typeof document !== "undefined"
        ? createPortal(
            <div
              role={feedback.error ? "alert" : "status"}
              className="fixed right-4 bottom-4 z-[100] flex max-w-[min(28rem,calc(100vw-32px))] items-center gap-2 rounded-lg border bg-popover px-3 py-2 text-sm text-popover-foreground shadow-md"
            >
              {feedback.error ? null : <Check className="size-4 shrink-0 text-emerald-600" />}
              <span className="min-w-0 break-words">{feedback.text}</span>
              {feedback.error ? (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="shrink-0"
                  aria-label={t("common:close")}
                  onClick={() => setFeedback(null)}
                >
                  <X className="size-4" />
                </Button>
              ) : null}
            </div>,
            document.body,
          )
        : null}
    </>
  );
}

export function RestoreSessionButton({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("layout");
  const pending = useWorkspaceStore((state) => Boolean(state.sessionMutations[sessionId]));
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="flex flex-col items-center gap-2 py-3">
      <Button
        variant="outline"
        disabled={pending}
        onClick={() => {
          setError(null);
          void useWorkspaceStore
            .getState()
            .setSessionArchived(sessionId, false)
            .catch((reason: unknown) =>
              setError(reason instanceof Error ? reason.message : String(reason)),
            );
        }}
      >
        <ArchiveRestore className="size-4" />
        {t("sessionMenu.restore")}
      </Button>
      {error ? (
        <p role="alert" className="text-sm break-words text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}
