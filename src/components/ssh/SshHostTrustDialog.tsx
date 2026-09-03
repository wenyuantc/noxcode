import { useTranslation } from "react-i18next";

import { resolveSshHostTrust } from "@/lib/backend";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useSshTrustEvents } from "@/hooks/useSshTrustEvents";

export function SshHostTrustDialog() {
  const { t } = useTranslation("ssh");
  const { prompt, setPrompt, changed, setChanged } = useSshTrustEvents();

  return (
    <>
      <Dialog
        open={Boolean(prompt)}
        onOpenChange={(open) => {
          if (!open && prompt) {
            void resolveSshHostTrust(prompt.prompt_id, false);
            setPrompt(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("trustTitle")}</DialogTitle>
            <DialogDescription>
              {prompt ? t("trustBody", { host: prompt.host, port: prompt.port }) : null}
            </DialogDescription>
          </DialogHeader>
          <code className="block break-all rounded-md bg-muted px-3 py-2 text-xs">
            {prompt?.fingerprint_sha256}
          </code>
          {prompt?.known_hosts_path ? (
            <p className="break-all text-xs text-muted-foreground">{prompt.known_hosts_path}</p>
          ) : null}
          <DialogFooter>
            <Button
              variant="ghost"
              onClick={() => {
                if (!prompt) return;
                void resolveSshHostTrust(prompt.prompt_id, false);
                setPrompt(null);
              }}
            >
              {t("trustReject")}
            </Button>
            <Button
              onClick={() => {
                if (!prompt) return;
                void resolveSshHostTrust(prompt.prompt_id, true);
                setPrompt(null);
              }}
            >
              {t("trustAccept")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog open={Boolean(changed)} onOpenChange={() => setChanged(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="text-destructive">{t("keyChangedTitle")}</DialogTitle>
            <DialogDescription>
              {changed
                ? t("keyChangedBody", {
                    host: changed.host,
                    port: changed.port,
                    line: changed.line,
                  })
                : null}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button onClick={() => setChanged(null)}>{t("trustReject")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
