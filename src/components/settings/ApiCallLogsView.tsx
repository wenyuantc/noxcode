import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { getNativeApiCallLog, listNativeApiCallLogs, listWorkspaces } from "@/lib/backend";
import { formatRelativeTime, formatTokenCount } from "@/lib/utils";
import type {
  NativeApiCallLogDetail,
  NativeApiCallLogListItem,
  NativeApiCallLogPage,
  Workspace,
} from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";

export function ApiCallLogsView() {
  const { t, i18n } = useTranslation(["apiLogs", "common"]);
  const [page, setPage] = useState<NativeApiCallLogPage | null>(null);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspaceId, setWorkspaceId] = useState("");
  const [detail, setDetail] = useState<NativeApiCallLogDetail | null>(null);

  const reload = () => {
    void listNativeApiCallLogs({
      workspace_id: workspaceId || null,
      limit: 80,
      include_total: true,
    }).then(setPage);
  };

  useEffect(() => {
    void listWorkspaces().then(setWorkspaces);
  }, []);

  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- filters are the reload inputs
  }, [workspaceId]);

  return (
    <div className="flex h-screen flex-col bg-background">
      <header className="flex items-center gap-3 border-b px-6 py-3">
        <Link to="/settings/usage" className="text-sm text-muted-foreground hover:text-foreground">
          ← {t("apiLogs:title")}
        </Link>
        <span className="flex-1" />
        <select
          className="h-8 rounded-md border px-2 text-sm"
          value={workspaceId}
          onChange={(event) => setWorkspaceId(event.target.value)}
        >
          <option value="">{t("apiLogs:workspace")}</option>
          {workspaces.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </select>
        <Button size="sm" variant="outline" onClick={reload}>
          {t("apiLogs:refresh")}
        </Button>
      </header>
      <main className="min-h-0 flex-1 overflow-auto">
        {(page?.items.length ?? 0) === 0 ? (
          <p className="p-6 text-sm text-muted-foreground">{t("apiLogs:empty")}</p>
        ) : (
          <table className="w-full text-left text-sm">
            <thead className="sticky top-0 bg-background text-xs text-muted-foreground">
              <tr>
                <th className="px-4 py-2">{t("apiLogs:model")}</th>
                <th className="px-4 py-2">{t("apiLogs:channel")}</th>
                <th className="px-4 py-2">{t("apiLogs:status")}</th>
                <th className="px-4 py-2">{t("apiLogs:tokens")}</th>
                <th className="px-4 py-2">{t("apiLogs:duration")}</th>
                <th className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {(page?.items ?? []).map((item) => (
                <LogRow
                  key={item.id}
                  item={item}
                  locale={i18n.language}
                  onOpen={() => void getNativeApiCallLog(item.id).then(setDetail)}
                />
              ))}
            </tbody>
          </table>
        )}
      </main>
      <Dialog open={Boolean(detail)} onOpenChange={(open) => !open && setDetail(null)}>
        <DialogContent className="max-h-[80vh] overflow-auto sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t("apiLogs:detail")}</DialogTitle>
          </DialogHeader>
          {detail ? (
            <div className="space-y-3 text-xs">
              <p>
                {detail.model} · {detail.channel_name} · {detail.status}
              </p>
              {detail.error_message ? (
                <pre className="whitespace-pre-wrap rounded-md bg-destructive/10 p-2">
                  {detail.error_message}
                </pre>
              ) : null}
              <section>
                <p className="mb-1 font-medium">{t("apiLogs:request")}</p>
                <pre className="max-h-56 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-2">
                  {detail.request_body}
                </pre>
              </section>
              <section>
                <p className="mb-1 font-medium">{t("apiLogs:response")}</p>
                <pre className="max-h-56 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-2">
                  {detail.response_body}
                </pre>
              </section>
            </div>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}

function LogRow({
  item,
  locale,
  onOpen,
}: {
  item: NativeApiCallLogListItem;
  locale: string;
  onOpen: () => void;
}) {
  return (
    <tr className="border-t hover:bg-muted/40">
      <td className="px-4 py-2">{item.model ?? "—"}</td>
      <td className="px-4 py-2">{item.channel_name ?? "—"}</td>
      <td className="px-4 py-2">{item.status}</td>
      <td className="px-4 py-2">
        {formatTokenCount(item.input_tokens ?? 0)} / {formatTokenCount(item.output_tokens ?? 0)}
      </td>
      <td className="px-4 py-2">
        {item.duration_ms != null
          ? `${item.duration_ms}ms`
          : formatRelativeTime(item.created_at, locale)}
      </td>
      <td className="px-4 py-2 text-right">
        <Button size="sm" variant="ghost" onClick={onOpen}>
          →
        </Button>
      </td>
    </tr>
  );
}
