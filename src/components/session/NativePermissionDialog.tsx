import { useTranslation } from "react-i18next";
import { useState } from "react";

import { resolveNativeToolPermission } from "@/lib/backend";
import { resolveSessionRequest } from "@/lib/nativeRequestResolution";
import {
  fileAccessSelections,
  permissionDirectory,
  permissionTargetLabel,
} from "@/lib/nativeFileAccess";
import type { NativePermissionDecision, PermissionRuleScope } from "@/lib/types";
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
  const [selection, setSelection] = useState<{
    requestId: string;
    directories: Record<number, boolean>;
    scope: PermissionRuleScope;
  } | null>(null);
  const selected = selection?.requestId === pending?.request_id ? selection : null;
  const directories = selected?.directories ?? {};
  const scope = selected?.scope ?? (pending?.workspace_id ? "workspace" : "global");
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
        resolveNativeToolPermission(
          current.session_record_id,
          current.request_id,
          decision,
          decision === "allow_always" && current.file_access
            ? fileAccessSelections(current.file_access, directories)
            : undefined,
          decision === "allow_always" ? scope : undefined,
        ),
      );
      if (
        decision === "allow_session" &&
        current.kind !== "mcp" &&
        !current.file_access &&
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
  const access = pending?.file_access;
  const updateSelection = (nextDirectories: Record<number, boolean>, nextScope = scope) => {
    if (pending)
      setSelection({
        requestId: pending.request_id,
        directories: nextDirectories,
        scope: nextScope,
      });
  };

  return (
    <Dialog
      open={Boolean(pending)}
      onOpenChange={(open) => {
        if (!open) void resolve("deny");
      }}
    >
      <DialogContent className="max-w-md max-h-[85dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {access
              ? t("permissionFileTitle")
              : isRule
                ? t("permissionRuleTitle")
                : t("permissionTitle")}
          </DialogTitle>
          <DialogDescription className="whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
            {pending?.summary}
          </DialogDescription>
        </DialogHeader>
        <p className="text-xs text-muted-foreground break-words [overflow-wrap:anywhere]">
          {requestSession?.title || pending?.session_record_id} · {pending?.tool_name} ·{" "}
          {access ? t("permissionFileTitle") : pending?.kind}
        </p>
        {access ? (
          <fieldset disabled={busy} className="min-w-0 space-y-3">
            <p className="text-sm break-words [overflow-wrap:anywhere]">
              {permissionTargetLabel(access.target) || t("permissionLocalTarget")}
            </p>
            <ul className="divide-y divide-border min-w-0">
              {access.paths.map((entry, index) => {
                const directory = directories[index] ?? entry.scope === "subtree";
                const savedPath =
                  directory && entry.scope === "exact"
                    ? permissionDirectory(entry.path)
                    : entry.path;
                return (
                  <li key={`${index}:${entry.path}`} className="min-w-0 space-y-2 py-3">
                    <p className="text-xs font-medium">
                      {t(`permissionOperation.${entry.operation}`)}
                    </p>
                    <code className="block text-xs whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
                      {entry.path}
                    </code>
                    {entry.requested_path !== entry.path ? (
                      <p className="text-xs text-muted-foreground break-words [overflow-wrap:anywhere]">
                        {entry.requested_path}
                      </p>
                    ) : null}
                    <label className="flex items-start gap-2 text-xs">
                      <input
                        type="checkbox"
                        checked={directory}
                        disabled={entry.scope === "subtree"}
                        onChange={(event) =>
                          updateSelection({ ...directories, [index]: event.target.checked })
                        }
                        className="mt-0.5 shrink-0"
                      />
                      <span>{t("permissionIncludeDirectory")}</span>
                    </label>
                    {directory ? (
                      <code className="block text-xs text-muted-foreground break-words [overflow-wrap:anywhere]">
                        {savedPath}
                      </code>
                    ) : null}
                  </li>
                );
              })}
            </ul>
            <label className="flex items-center justify-between gap-3 text-xs">
              {t("permissionSaveScope")}
              <select
                aria-label={t("permissionSaveScope")}
                value={scope}
                onChange={(event) =>
                  updateSelection(directories, event.target.value as PermissionRuleScope)
                }
                className="max-w-[65%] rounded-md border border-input bg-background px-2 py-1.5"
              >
                {pending?.workspace_id ? (
                  <option value="workspace">{t("permissionScopeWorkspace")}</option>
                ) : null}
                <option value="global">{t("permissionScopeGlobal")}</option>
              </select>
            </label>
          </fieldset>
        ) : null}
        {error?.requestId === pending?.request_id ? (
          <p
            role="alert"
            className="text-sm text-destructive whitespace-pre-wrap break-words [overflow-wrap:anywhere]"
          >
            {error?.message}
          </p>
        ) : null}
        <DialogFooter className="flex-col gap-2 sm:flex-col">
          <fieldset disabled={busy} className="flex w-full flex-col gap-2">
            <Button variant="outline" onClick={() => resolve("allow_once")}>
              {t("permissionAllowOnce")}
            </Button>
            {access || pending?.tool_name === "WorkspaceHooks" ? null : pending?.kind === "mcp" ? (
              <Button onClick={() => resolve("allow_server")}>{t("permissionAllowServer")}</Button>
            ) : (
              <Button onClick={() => resolve("allow_session")}>
                {t("permissionAllowSession")}
              </Button>
            )}
            {(access || suggestion) && pending?.tool_name !== "WorkspaceHooks" ? (
              <Button
                variant="secondary"
                className="h-auto min-h-8 whitespace-normal break-words [overflow-wrap:anywhere]"
                onClick={() => resolve("allow_always")}
              >
                {access
                  ? t("permissionAlways")
                  : t("permissionAllowAlways", { pattern: suggestion?.pattern })}
              </Button>
            ) : null}
            <Button variant="ghost" onClick={() => resolve("deny")}>
              {t("permissionDeny")}
            </Button>
          </fieldset>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
