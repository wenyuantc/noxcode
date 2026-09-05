import { useTranslation } from "react-i18next";
import { useState } from "react";

import { resolveNativeToolPermission } from "@/lib/backend";
import { resolveSessionRequest } from "@/lib/nativeRequestResolution";
import type { NativePermissionDecision } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function NativePermissionDialog() {
  const { t } = useTranslation("sessions");
  const pending = useSessionStore((state) => {
    const selected = state.selectedSessionId
      ? Object.values(state.permissions[state.selectedSessionId] ?? {})[0]
      : undefined;
    return (
      selected ?? Object.values(state.permissions).flatMap((requests) => Object.values(requests))[0]
    );
  });
  const [busy, setBusy] = useState(false);
  const requestSession = useWorkspaceStore((state) =>
    state.sessions.find((item) => item.id === pending?.session_record_id),
  );
  const [error, setError] = useState<{ requestId: string; message: string } | null>(null);

  const resolve = async (decision: NativePermissionDecision) => {
    if (!pending || busy) return;
    const current = pending;
    setBusy(true);
    setError(null);
    try {
      await resolveSessionRequest({ ...current, kind: "permission" }, () =>
        resolveNativeToolPermission(current.session_record_id, current.request_id, decision),
      );
      if (
        decision === "allow_session" &&
        current.kind !== "mcp" &&
        current.tool_name !== "WorkspaceHooks"
      ) {
        const state = useSessionStore.getState();
        const runtime = state.configurationBySession[current.session_record_id];
        if (runtime)
          state.setConfiguration(current.session_record_id, {
            ...runtime,
            permission_mode: "yolo",
          });
      }
    } catch (reason) {
      setError({ requestId: current.request_id, message: String(reason) });
    } finally {
      setBusy(false);
    }
  };

  const suggestion = pending?.suggested_rule ?? null;
  const isRule = pending?.kind === "rule";

  return (
    <Dialog
      open={Boolean(pending)}
      onOpenChange={(open) => {
        if (!open) void resolve("deny");
      }}
    >
      <DialogContent className="max-w-md max-h-[85dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{isRule ? t("permissionRuleTitle") : t("permissionTitle")}</DialogTitle>
          <DialogDescription className="whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
            {pending?.summary}
          </DialogDescription>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          {requestSession?.title || pending?.session_record_id} · {pending?.tool_name} ·{" "}
          {pending?.kind}
        </p>
        {error?.requestId === pending?.request_id ? (
          <p role="alert" className="text-sm text-destructive">
            {error?.message}
          </p>
        ) : null}
        <DialogFooter className="flex-col gap-2 sm:flex-col">
          <fieldset disabled={busy} className="flex w-full flex-col gap-2">
            {pending?.tool_name === "WorkspaceHooks" ? null : pending?.kind === "mcp" ? (
              <Button onClick={() => resolve("allow_server")}>{t("permissionAllowServer")}</Button>
            ) : (
              <Button onClick={() => resolve("allow_session")}>
                {t("permissionAllowSession")}
              </Button>
            )}
            {suggestion && pending?.tool_name !== "WorkspaceHooks" ? (
              <Button variant="secondary" onClick={() => resolve("allow_always")}>
                {t("permissionAllowAlways", { pattern: suggestion.pattern })}
              </Button>
            ) : null}
            <Button variant="outline" onClick={() => resolve("allow_once")}>
              {t("permissionAllowOnce")}
            </Button>
            <Button variant="ghost" onClick={() => resolve("deny")}>
              {t("permissionDeny")}
            </Button>
          </fieldset>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
