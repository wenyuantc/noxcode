import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { getNativeApiCallLog } from "@/lib/backend";
import {
  formatApiCallLogCacheRate,
  formatApiCallLogDurationMs,
  formatApiCallLogThinking,
  formatApiCallLogThroughput,
  formatApiCallLogTokenCount,
  isTruncatedFlag,
  prettyPrintJsonBody,
} from "@/lib/apiLogs";
import type { NativeApiCallLogDetail } from "@/lib/types";
import { cn, formatDate } from "@/lib/utils";

interface ApiCallLogDetailDialogProps {
  open: boolean;
  logId: string | null;
  onOpenChange: (open: boolean) => void;
}

function statusBadgeVariant(status: string): "default" | "destructive" | "secondary" | "outline" {
  if (status === "success") {
    return "default";
  }
  if (status === "failed") {
    return "destructive";
  }
  if (status === "cancelled") {
    return "secondary";
  }
  return "outline";
}

function statusLabel(status: string, t: (key: string) => string) {
  if (status === "success") {
    return t("statusSuccess");
  }
  if (status === "failed") {
    return t("statusFailed");
  }
  if (status === "cancelled") {
    return t("statusCancelled");
  }
  return status;
}

function MetaItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 space-y-1">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="truncate text-sm" title={value}>
        {value}
      </div>
    </div>
  );
}

export function ApiCallLogDetailDialog({ open, logId, onOpenChange }: ApiCallLogDetailDialogProps) {
  const { t } = useTranslation("apiLogs");
  const [detail, setDetail] = useState<NativeApiCallLogDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!open || !logId) {
      return;
    }

    let active = true;
    setLoading(true);
    setErrorMessage(null);
    setDetail(null);

    void getNativeApiCallLog(logId)
      .then((result) => {
        if (!active) {
          return;
        }
        setDetail(result);
      })
      .catch((error) => {
        if (!active) {
          return;
        }
        setErrorMessage(error instanceof Error ? error.message : t("loadFailed"));
      })
      .finally(() => {
        if (active) {
          setLoading(false);
        }
      });

    return () => {
      active = false;
    };
  }, [logId, open, t]);

  const unknown = t("unknown");
  const emptyValue = t("emptyValue");
  const lessThanOneSecond = t("lessThanOneSecond");
  const cacheRateLabels = { unknown, empty: emptyValue };
  const requestBody = prettyPrintJsonBody(detail?.request_body);
  const responseBody = prettyPrintJsonBody(detail?.response_body);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] w-[min(96vw,72rem)] max-w-[min(96vw,72rem)] flex-col overflow-hidden sm:max-w-[min(96vw,72rem)]">
        <DialogHeader>
          <DialogTitle>{t("detailTitle")}</DialogTitle>
          <DialogDescription>{t("detailDescription")}</DialogDescription>
        </DialogHeader>

        {loading ? (
          <div className="flex h-[32rem] items-center justify-center text-sm text-muted-foreground">
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            {t("loading")}
          </div>
        ) : errorMessage ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
            {errorMessage}
          </div>
        ) : !detail ? (
          <div className="flex h-[12rem] items-center justify-center text-sm text-muted-foreground">
            {t("empty")}
          </div>
        ) : (
          <div className="min-h-0 flex-1 space-y-4 overflow-auto pr-1">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={statusBadgeVariant(detail.status)}>
                {statusLabel(detail.status, t)}
              </Badge>
              {detail.http_status != null && (
                <Badge variant="outline">
                  {t("httpStatus")} {detail.http_status}
                </Badge>
              )}
              {detail.call_kind && (
                <Badge variant="outline">
                  {t(`callKind.${detail.call_kind}`, detail.call_kind)}
                </Badge>
              )}
            </div>

            <div className="grid gap-3 rounded-xl border border-border/70 p-3 sm:grid-cols-2 lg:grid-cols-3">
              <MetaItem label={t("colChannel")} value={detail.channel_name?.trim() || unknown} />
              <MetaItem label={t("colModel")} value={detail.model?.trim() || unknown} />
              <MetaItem
                label={t("colThinking")}
                value={formatApiCallLogThinking(detail, unknown, t("thinkingOff"))}
              />
              <MetaItem label={t("colFormat")} value={detail.request_format || unknown} />
              <MetaItem label={t("protocol")} value={detail.protocol || unknown} />
              <MetaItem label={t("callId")} value={detail.call_id || unknown} />
              <MetaItem
                label={t("colInput")}
                value={formatApiCallLogTokenCount(detail.input_tokens, unknown)}
              />
              <MetaItem
                label={t("colOutput")}
                value={formatApiCallLogTokenCount(detail.output_tokens, unknown)}
              />
              <MetaItem
                label={t("colCached")}
                value={formatApiCallLogTokenCount(detail.cached_tokens, unknown)}
              />
              <MetaItem
                label={t("colCacheRate")}
                value={formatApiCallLogCacheRate(
                  detail.input_tokens,
                  detail.cached_tokens,
                  cacheRateLabels,
                )}
              />
              <MetaItem
                label={t("colTotal")}
                value={formatApiCallLogTokenCount(detail.total_tokens, unknown)}
              />
              <MetaItem
                label={t("colFirstToken")}
                value={formatApiCallLogDurationMs(
                  detail.first_token_ms,
                  unknown,
                  lessThanOneSecond,
                )}
              />
              <MetaItem
                label={t("colDuration")}
                value={formatApiCallLogDurationMs(detail.duration_ms, unknown, lessThanOneSecond)}
              />
              <MetaItem
                label={t("colThroughput")}
                value={formatApiCallLogThroughput(
                  detail.output_tokens,
                  detail.duration_ms,
                  unknown,
                )}
              />
              <MetaItem label={t("colCreatedAt")} value={formatDate(detail.created_at)} />
              <MetaItem label={t("sessionId")} value={detail.session_id?.trim() || unknown} />
              <MetaItem label={t("workspace")} value={detail.workspace_name?.trim() || unknown} />
              <MetaItem label={t("subagentId")} value={detail.subagent_id?.trim() || unknown} />
              <MetaItem label={t("attempt")} value={String(detail.attempt)} />
            </div>

            {detail.error_message && (
              <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                <div className="mb-1 text-xs font-medium">{t("errorMessage")}</div>
                {detail.error_message}
              </div>
            )}

            <JsonBodySection
              title={t("requestBody")}
              body={requestBody}
              truncated={isTruncatedFlag(detail.request_truncated)}
              emptyLabel={t("emptyBody")}
              truncatedHint={t("truncatedHint")}
            />
            <JsonBodySection
              title={t("responseBody")}
              body={responseBody}
              truncated={isTruncatedFlag(detail.response_truncated)}
              emptyLabel={t("emptyBody")}
              truncatedHint={t("truncatedHint")}
            />
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function JsonBodySection({
  title,
  body,
  truncated,
  emptyLabel,
  truncatedHint,
}: {
  title: string;
  body: string;
  truncated: boolean;
  emptyLabel: string;
  truncatedHint: string;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-medium">{title}</h3>
        {truncated && (
          <span className="text-xs text-amber-600 dark:text-amber-400">{truncatedHint}</span>
        )}
      </div>
      {body ? (
        <pre className="h-64 overflow-auto rounded-md border border-border bg-background p-3 font-mono text-xs leading-5 whitespace-pre-wrap">
          {body}
        </pre>
      ) : (
        <div
          className={cn(
            "flex h-24 items-center justify-center rounded-md border border-dashed border-border text-sm text-muted-foreground",
          )}
        >
          {emptyLabel}
        </div>
      )}
    </div>
  );
}
