import { useTranslation } from "react-i18next";

import { resolveNativeToolPermission } from "@/lib/backend";
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

export function NativePermissionDialog() {
  const { t } = useTranslation("sessions");
  const pending = useSessionStore((state) => state.permission);
  const setPermission = useSessionStore((state) => state.setPermission);

  const resolve = (decision: NativePermissionDecision) => {
    if (!pending) return;
    const current = pending;
    setPermission(null);
    void resolveNativeToolPermission(current.session_record_id, current.request_id, decision);
  };

  return (
    <Dialog open={Boolean(pending)} onOpenChange={(open) => !open && pending && resolve("deny")}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("permissionTitle")}</DialogTitle>
          <DialogDescription>{pending?.summary}</DialogDescription>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          {pending?.tool_name} · {pending?.kind}
        </p>
        <DialogFooter className="flex-col gap-2 sm:flex-col">
          {pending?.kind === "mcp" ? (
            <Button onClick={() => resolve("allow_server")}>{t("permissionAllowServer")}</Button>
          ) : (
            <Button onClick={() => resolve("allow_session")}>{t("permissionAllowSession")}</Button>
          )}
          <Button variant="outline" onClick={() => resolve("allow_once")}>
            {t("permissionAllowOnce")}
          </Button>
          <Button variant="ghost" onClick={() => resolve("deny")}>
            {t("permissionDeny")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
