import { AlertTriangle, Check, Copy, ShieldAlert } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { resolveNativeToolPermission } from "@/lib/backend";
import {
  fileAccessSelections,
  permissionDirectory,
  permissionTargetLabel,
} from "@/lib/nativeFileAccess";
import { resolveSessionRequest } from "@/lib/nativeRequestResolution";
import type { NativePermissionDecision, PermissionRuleScope } from "@/lib/types";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export interface ParsedPermissionSummary {
  riskReason: string | null;
  command: string | null;
  detail: string;
}

export function parsePermissionSummary(
  summary: string | undefined,
  toolName: string | undefined,
): ParsedPermissionSummary {
  if (!summary) {
    return { riskReason: null, command: null, detail: "" };
  }

  let riskReason: string | null = null;
  let detail = summary;

  const colonIndex = summary.indexOf("：");
  if (colonIndex !== -1) {
    riskReason = summary.slice(0, colonIndex).trim();
    detail = summary.slice(colonIndex + 1).trim();
  } else {
    const asciiIndex = summary.indexOf(": ");
    if (asciiIndex !== -1) {
      riskReason = summary.slice(0, asciiIndex).trim();
      detail = summary.slice(asciiIndex + 2).trim();
    }
  }

  const isBash = toolName?.toLowerCase() === "bash";

  if (isBash) {
    return {
      riskReason,
      command: detail,
      detail,
    };
  }

  return {
    riskReason,
    command: null,
    detail,
  };
}

export function NativePermissionDialog() {
  const { t } = useTranslation(["sessions", "common"]);
  const pending = useSessionStore((state) => {
    const selected = state.selectedSessionId
      ? Object.values(state.permissions[state.selectedSessionId] ?? {})[0]
      : undefined;
    return (
      selected ?? Object.values(state.permissions).flatMap((requests) => Object.values(requests))[0]
    );
  });
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
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

  const parsed = parsePermissionSummary(pending?.summary, pending?.tool_name);

  const handleCopy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // ignore
    }
  };

  return (
    <Dialog
      open={Boolean(pending)}
      onOpenChange={(open) => {
        if (!open) void resolve("deny");
      }}
    >
      <DialogContent className="flex max-h-[85dvh] w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-xl md:max-w-2xl">
        <DialogHeader className="shrink-0 border-b border-border/50 px-5 py-4 pr-12 text-left">
          <DialogTitle className="flex items-center gap-2 text-base font-semibold">
            <ShieldAlert className="size-5 shrink-0 text-amber-500" />
            <span>
              {access
                ? t("permissionFileTitle")
                : isRule
                  ? t("permissionRuleTitle")
                  : t("permissionTitle")}
            </span>
          </DialogTitle>
          <DialogDescription className="sr-only">{pending?.summary}</DialogDescription>
          <div className="flex flex-wrap items-center gap-1.5 pt-1 text-xs text-muted-foreground min-w-0">
            <span
              className="max-w-[220px] truncate font-medium text-foreground"
              title={requestSession?.title || pending?.session_record_id}
            >
              {requestSession?.title || pending?.session_record_id}
            </span>
            <span>·</span>
            {pending?.tool_name ? (
              <Badge variant="outline" className="h-5 px-1.5 py-0 font-mono text-[11px]">
                {pending.tool_name}
              </Badge>
            ) : null}
            <Badge variant="secondary" className="h-5 px-1.5 py-0 text-[11px]">
              {access ? t("permissionFileTitle") : pending?.kind}
            </Badge>
          </div>
        </DialogHeader>

        <div className="flex-1 min-h-0 space-y-4 overflow-y-auto overflow-x-hidden px-5 py-4">
          {parsed.riskReason ? (
            <div className="flex items-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs font-medium text-amber-700 dark:text-amber-400">
              <AlertTriangle className="size-4 shrink-0 text-amber-500" />
              <span className="break-all">{parsed.riskReason}</span>
            </div>
          ) : null}

          {parsed.command ? (
            <div className="space-y-1.5 min-w-0">
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span className="font-medium text-foreground">{t("permissionCommand")}</span>
                <button
                  type="button"
                  onClick={() => void handleCopy(parsed.command!)}
                  className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
                >
                  {copied ? (
                    <>
                      <Check className="size-3.5 text-emerald-500" />
                      <span className="text-emerald-500">{t("common:copied")}</span>
                    </>
                  ) : (
                    <>
                      <Copy className="size-3.5" />
                      <span>{t("common:copy")}</span>
                    </>
                  )}
                </button>
              </div>
              <div className="relative rounded-lg border border-border/70 bg-muted/40 dark:bg-zinc-950/70 p-3 min-w-0">
                <pre className="max-h-56 overflow-y-auto overflow-x-auto font-mono text-xs text-foreground whitespace-pre-wrap break-all leading-relaxed select-text [overflow-wrap:anywhere]">
                  <code>{parsed.command}</code>
                </pre>
              </div>
            </div>
          ) : parsed.detail ? (
            <div className="rounded-lg border border-border/70 bg-muted/30 dark:bg-zinc-950/50 p-3 text-xs leading-relaxed text-foreground break-all select-text [overflow-wrap:anywhere]">
              {parsed.detail}
            </div>
          ) : null}

          {access ? (
            <fieldset disabled={busy} className="min-w-0 space-y-3">
              <p className="text-xs font-medium text-foreground break-all">
                {permissionTargetLabel(access.target) || t("permissionLocalTarget")}
              </p>
              <ul className="divide-y divide-border min-w-0 max-h-52 overflow-y-auto rounded-lg border border-border/60 bg-muted/20 px-3">
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
                      <code className="block text-xs font-mono whitespace-pre-wrap break-all select-text rounded bg-muted/60 px-2 py-1">
                        {entry.path}
                      </code>
                      {entry.requested_path !== entry.path ? (
                        <p className="text-xs text-muted-foreground break-all select-text">
                          {entry.requested_path}
                        </p>
                      ) : null}
                      <label className="flex items-start gap-2 text-xs cursor-pointer">
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
                        <code className="block text-xs text-muted-foreground font-mono break-all select-text">
                          {savedPath}
                        </code>
                      ) : null}
                    </li>
                  );
                })}
              </ul>
              <label className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
                <span>{t("permissionSaveScope")}</span>
                <select
                  aria-label={t("permissionSaveScope")}
                  value={scope}
                  onChange={(event) =>
                    updateSelection(directories, event.target.value as PermissionRuleScope)
                  }
                  className="max-w-[65%] rounded-md border border-input bg-background px-2 py-1.5 text-xs text-foreground"
                >
                  {pending?.workspace_id ? (
                    <option value="workspace">{t("permissionScopeWorkspace")}</option>
                  ) : null}
                  <option value="global">{t("permissionScopeGlobal")}</option>
                </select>
              </label>
            </fieldset>
          ) : null}

          {suggestion?.plan_bash ? (
            <div className="flex items-center gap-1.5 rounded-md border border-border/50 bg-muted/25 px-3 py-2 text-xs text-muted-foreground min-w-0">
              <span className="shrink-0 font-medium">
                {permissionTargetLabel(suggestion.plan_bash.target) || t("permissionLocalTarget")}
              </span>
              <span>·</span>
              <span
                className="font-mono break-all select-text text-foreground/80"
                title={suggestion.plan_bash.workspace_root}
              >
                {suggestion.plan_bash.workspace_root}
              </span>
            </div>
          ) : null}

          {error?.requestId === pending?.request_id ? (
            <p role="alert" className="text-xs text-destructive whitespace-pre-wrap break-all">
              {error?.message}
            </p>
          ) : null}
        </div>

        <DialogFooter className="m-0 shrink-0 border-t border-border/50 bg-muted/20 px-5 py-3.5 sm:justify-end">
          <fieldset disabled={busy} className="flex w-full flex-col gap-2">
            <Button variant="outline" onClick={() => resolve("allow_once")}>
              {t("permissionAllowOnce")}
            </Button>
            {access ||
            pending?.allow_once_only ||
            pending?.tool_name === "Bash" ||
            suggestion?.plan_bash ||
            pending?.tool_name === "WorkspaceHooks" ? null : pending?.kind === "mcp" ? (
              <Button onClick={() => resolve("allow_server")}>{t("permissionAllowServer")}</Button>
            ) : (
              <Button onClick={() => resolve("allow_session")}>
                {t("permissionAllowSession")}
              </Button>
            )}
            {(access || suggestion) &&
            !pending?.allow_once_only &&
            pending?.tool_name !== "WorkspaceHooks" ? (
              <Button
                variant="secondary"
                className="h-auto min-h-8 whitespace-normal break-words [overflow-wrap:anywhere]"
                onClick={() => resolve("allow_always")}
              >
                {access || suggestion?.plan_bash
                  ? t("permissionAlways")
                  : t("permissionAllowAlways", { pattern: suggestion?.pattern })}
              </Button>
            ) : null}
            {pending?.tool_name === "Bash" && !access && !pending.allow_once_only ? (
              <Button
                className="h-auto min-h-8 whitespace-normal break-words [overflow-wrap:anywhere]"
                onClick={() => resolve("allow_session_commands")}
              >
                {t("permissionAllowSessionCommands")}
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
