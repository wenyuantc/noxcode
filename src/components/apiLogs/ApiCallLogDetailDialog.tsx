import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertCircle, Check, Copy, FileCode2, Layers, Loader2, Timer } from "lucide-react";

import { CodeBlock } from "@/components/code/CodeBlock";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
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

function StatusPill({ status, label }: { status: string; label: string }) {
  if (status === "success") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full border border-emerald-500/25 bg-emerald-500/10 px-2.5 py-0.5 text-xs font-medium text-emerald-600 dark:text-emerald-400">
        <span className="size-1.5 rounded-full bg-emerald-500" />
        {label}
      </span>
    );
  }
  if (status === "failed") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full border border-destructive/25 bg-destructive/10 px-2.5 py-0.5 text-xs font-medium text-destructive">
        <span className="size-1.5 rounded-full bg-destructive" />
        {label}
      </span>
    );
  }
  if (status === "cancelled") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full border border-border/70 bg-muted/60 px-2.5 py-0.5 text-xs font-medium text-muted-foreground">
        <span className="size-1.5 rounded-full bg-muted-foreground/60" />
        {label}
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-border px-2.5 py-0.5 text-xs font-medium text-muted-foreground">
      {label}
    </span>
  );
}

function MetaItem({
  label,
  value,
  copyable,
  onCopy,
  copied,
}: {
  label: string;
  value: string;
  copyable?: boolean;
  onCopy?: () => void;
  copied?: boolean;
}) {
  return (
    <div className="group/meta min-w-0 rounded-xl border border-border/60 bg-muted/20 p-3 space-y-1 transition-colors hover:border-border hover:bg-muted/30">
      <div className="flex items-center justify-between gap-1 text-[11px] font-medium text-muted-foreground">
        <span>{label}</span>
        {copyable && onCopy && (
          <button
            type="button"
            onClick={onCopy}
            className="opacity-0 transition-opacity group-hover/meta:opacity-100 hover:text-foreground"
            title="复制"
          >
            {copied ? (
              <Check className="size-3 text-emerald-500" />
            ) : (
              <Copy className="size-3 text-muted-foreground" />
            )}
          </button>
        )}
      </div>
      <div className="truncate font-mono text-xs text-foreground font-medium" title={value}>
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
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const copyToClipboard = async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(key);
      setTimeout(() => {
        setCopiedKey((curr) => (curr === key ? null : curr));
      }, 1500);
    } catch {
      // ignore
    }
  };

  useEffect(() => {
    if (!open || !logId) {
      return;
    }

    let active = true;
    setLoading(true);
    setErrorMessage(null);
    setDetail(null);
    setCopiedKey(null);

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

  const statusLabel = (status: string) => {
    if (status === "success") return t("statusSuccess");
    if (status === "failed") return t("statusFailed");
    if (status === "cancelled") return t("statusCancelled");
    return status;
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[92vh] w-[min(96vw,72rem)] max-w-[min(96vw,72rem)] flex-col overflow-hidden p-6 sm:max-w-[min(96vw,72rem)]">
        <DialogHeader className="space-y-1.5 pb-2">
          <div className="flex flex-wrap items-center justify-between gap-3 pr-6">
            <div className="flex items-center gap-2.5">
              <div className="flex size-7 items-center justify-center rounded-lg bg-primary/10 text-primary">
                <FileCode2 className="size-4" />
              </div>
              <DialogTitle className="text-base font-semibold tracking-tight">
                {t("detailTitle")}
              </DialogTitle>
              {detail && (
                <div className="flex items-center gap-2">
                  <StatusPill status={detail.status} label={statusLabel(detail.status)} />
                  {detail.http_status != null && (
                    <span
                      className={cn(
                        "rounded-md border px-2 py-0.5 font-mono text-[11px] font-medium",
                        detail.http_status >= 200 && detail.http_status < 300
                          ? "border-emerald-500/20 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                          : "border-destructive/20 bg-destructive/10 text-destructive",
                      )}
                    >
                      HTTP {detail.http_status}
                    </span>
                  )}
                  {detail.call_kind && (
                    <Badge variant="outline" className="text-[11px]">
                      {t(`callKind.${detail.call_kind}`, detail.call_kind)}
                    </Badge>
                  )}
                </div>
              )}
            </div>
          </div>
          <DialogDescription className="text-xs text-muted-foreground">
            {detail ? (
              <span>
                {detail.channel_name} · <span className="font-mono">{detail.model}</span> ·{" "}
                {formatDate(detail.created_at)}
              </span>
            ) : (
              t("detailDescription")
            )}
          </DialogDescription>
        </DialogHeader>

        {loading ? (
          <div className="flex h-[28rem] flex-col items-center justify-center gap-2.5 text-sm text-muted-foreground">
            <Loader2 className="size-6 animate-spin text-primary" />
            <span>{t("loading")}</span>
          </div>
        ) : errorMessage ? (
          <div className="flex items-center gap-2.5 rounded-xl border border-destructive/30 bg-destructive/10 p-4 text-xs text-destructive">
            <AlertCircle className="size-4 shrink-0" />
            <span>{errorMessage}</span>
          </div>
        ) : !detail ? (
          <div className="flex h-[16rem] items-center justify-center text-sm text-muted-foreground">
            {t("empty")}
          </div>
        ) : (
          <Tabs defaultValue="overview" className="flex min-h-0 flex-1 flex-col">
            <TabsList className="grid h-9 w-full grid-cols-3 rounded-lg bg-muted/50 p-1">
              <TabsTrigger value="overview" className="text-xs">
                指标与概览
              </TabsTrigger>
              <TabsTrigger value="request" className="text-xs">
                {t("requestBody")} {requestBody ? `(${requestBody.split("\n").length} 行)` : ""}
              </TabsTrigger>
              <TabsTrigger value="response" className="text-xs">
                {t("responseBody")} {responseBody ? `(${responseBody.split("\n").length} 行)` : ""}
              </TabsTrigger>
            </TabsList>

            {/* 概览与指标 Tab */}
            <TabsContent
              value="overview"
              className="min-h-0 flex-1 overflow-y-auto pt-4 space-y-4 pr-1"
            >
              {detail.error_message && (
                <div className="flex items-start gap-3 rounded-xl border border-destructive/35 bg-destructive/10 p-3.5 text-xs text-destructive">
                  <AlertCircle className="size-4 shrink-0 mt-0.5" />
                  <div className="space-y-1">
                    <span className="font-semibold">{t("errorMessage")}</span>
                    <p className="leading-relaxed font-mono">{detail.error_message}</p>
                  </div>
                </div>
              )}

              {/* 指标统计面板 */}
              <div className="grid gap-3 sm:grid-cols-2">
                {/* Token 卡片 */}
                <div className="rounded-xl border border-border/70 bg-muted/20 p-3.5 space-y-2.5">
                  <div className="flex items-center gap-2 text-xs font-semibold text-foreground">
                    <Layers className="size-3.5 text-primary" />
                    <span>Token 消耗分布</span>
                  </div>
                  <div className="grid grid-cols-2 gap-2 text-xs">
                    <div className="rounded-lg border border-border/40 bg-background/50 p-2">
                      <span className="text-[11px] text-muted-foreground">{t("colInput")}</span>
                      <div className="mt-0.5 font-mono font-semibold text-foreground">
                        {formatApiCallLogTokenCount(detail.input_tokens, unknown)}
                      </div>
                    </div>
                    <div className="rounded-lg border border-border/40 bg-background/50 p-2">
                      <span className="text-[11px] text-muted-foreground">{t("colOutput")}</span>
                      <div className="mt-0.5 font-mono font-semibold text-foreground">
                        {formatApiCallLogTokenCount(detail.output_tokens, unknown)}
                      </div>
                    </div>
                    <div className="rounded-lg border border-border/40 bg-background/50 p-2">
                      <span className="text-[11px] text-muted-foreground">{t("colCached")}</span>
                      <div className="mt-0.5 font-mono font-semibold text-foreground">
                        {formatApiCallLogTokenCount(detail.cached_tokens, unknown)}
                      </div>
                    </div>
                    <div className="rounded-lg border border-border/40 bg-background/50 p-2">
                      <span className="text-[11px] text-muted-foreground">{t("colCacheRate")}</span>
                      <div className="mt-0.5 font-mono font-semibold text-foreground">
                        {formatApiCallLogCacheRate(
                          detail.input_tokens,
                          detail.cached_tokens,
                          cacheRateLabels,
                        )}
                      </div>
                    </div>
                  </div>
                  <div className="flex items-center justify-between border-t border-border/40 pt-2 text-xs">
                    <span className="text-muted-foreground font-medium">{t("colTotal")}</span>
                    <span className="font-mono text-sm font-bold text-foreground">
                      {formatApiCallLogTokenCount(detail.total_tokens, unknown)}
                    </span>
                  </div>
                </div>

                {/* 性能与延迟卡片 */}
                <div className="rounded-xl border border-border/70 bg-muted/20 p-3.5 space-y-2.5">
                  <div className="flex items-center gap-2 text-xs font-semibold text-foreground">
                    <Timer className="size-3.5 text-cyan-500" />
                    <span>耗时与速率</span>
                  </div>
                  <div className="grid grid-cols-2 gap-2 text-xs">
                    <div className="rounded-lg border border-border/40 bg-background/50 p-2">
                      <span className="text-[11px] text-muted-foreground">
                        {t("colFirstToken")}
                      </span>
                      <div className="mt-0.5 font-mono font-semibold text-foreground">
                        {formatApiCallLogDurationMs(
                          detail.first_token_ms,
                          unknown,
                          lessThanOneSecond,
                        )}
                      </div>
                    </div>
                    <div className="rounded-lg border border-border/40 bg-background/50 p-2">
                      <span className="text-[11px] text-muted-foreground">{t("colDuration")}</span>
                      <div className="mt-0.5 font-mono font-semibold text-foreground">
                        {formatApiCallLogDurationMs(detail.duration_ms, unknown, lessThanOneSecond)}
                      </div>
                    </div>
                  </div>
                  <div className="flex items-center justify-between border-t border-border/40 pt-2 text-xs">
                    <span className="text-muted-foreground font-medium">{t("colThroughput")}</span>
                    <span className="font-mono text-sm font-bold text-foreground">
                      {formatApiCallLogThroughput(
                        detail.output_tokens,
                        detail.duration_ms,
                        unknown,
                      )}
                    </span>
                  </div>
                </div>
              </div>

              {/* 元数据网格 */}
              <div className="space-y-2">
                <span className="text-xs font-semibold text-foreground">调用上下文元数据</span>
                <div className="grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
                  <MetaItem
                    label={t("colChannel")}
                    value={detail.channel_name?.trim() || unknown}
                  />
                  <MetaItem label={t("colModel")} value={detail.model?.trim() || unknown} />
                  <MetaItem
                    label={t("colThinking")}
                    value={formatApiCallLogThinking(detail, unknown, t("thinkingOff"))}
                  />
                  <MetaItem label={t("colFormat")} value={detail.request_format || unknown} />
                  <MetaItem label={t("protocol")} value={detail.protocol || unknown} />
                  <MetaItem
                    label={t("callId")}
                    value={detail.call_id || unknown}
                    copyable={Boolean(detail.call_id)}
                    onCopy={() => detail.call_id && copyToClipboard(detail.call_id, "call_id")}
                    copied={copiedKey === "call_id"}
                  />
                  <MetaItem
                    label={t("sessionId")}
                    value={detail.session_id?.trim() || unknown}
                    copyable={Boolean(detail.session_id)}
                    onCopy={() =>
                      detail.session_id && copyToClipboard(detail.session_id, "session_id")
                    }
                    copied={copiedKey === "session_id"}
                  />
                  <MetaItem
                    label={t("workspace")}
                    value={detail.workspace_name?.trim() || unknown}
                  />
                  <MetaItem label={t("subagentId")} value={detail.subagent_id?.trim() || unknown} />
                  <MetaItem label={t("attempt")} value={String(detail.attempt)} />
                  <MetaItem label={t("colCreatedAt")} value={formatDate(detail.created_at)} />
                </div>
              </div>
            </TabsContent>

            {/* 请求参数 Tab */}
            <TabsContent
              value="request"
              className="min-h-0 flex-1 overflow-hidden pt-3 flex flex-col space-y-2"
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground">
                    {requestBody
                      ? `${requestBody.split("\n").length} 行 · ${requestBody.length} 字符`
                      : t("emptyBody")}
                  </span>
                  {isTruncatedFlag(detail.request_truncated) && (
                    <span className="rounded bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-600 dark:text-amber-400">
                      {t("truncatedHint")}
                    </span>
                  )}
                </div>
                {requestBody && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => copyToClipboard(requestBody, "request_body")}
                    className="h-7 text-xs gap-1.5"
                  >
                    {copiedKey === "request_body" ? (
                      <Check className="size-3 text-emerald-500" />
                    ) : (
                      <Copy className="size-3" />
                    )}
                    <span>{copiedKey === "request_body" ? "已复制" : "复制 JSON"}</span>
                  </Button>
                )}
              </div>
              {requestBody ? (
                <CodeBlock
                  className="min-h-0 flex-1 rounded-xl"
                  code={requestBody}
                  language="json"
                />
              ) : (
                <div className="flex h-36 items-center justify-center rounded-xl border border-dashed border-border text-xs text-muted-foreground">
                  {t("emptyBody")}
                </div>
              )}
            </TabsContent>

            {/* 返回内容 Tab */}
            <TabsContent
              value="response"
              className="min-h-0 flex-1 overflow-hidden pt-3 flex flex-col space-y-2"
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground">
                    {responseBody
                      ? `${responseBody.split("\n").length} 行 · ${responseBody.length} 字符`
                      : t("emptyBody")}
                  </span>
                  {isTruncatedFlag(detail.response_truncated) && (
                    <span className="rounded bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-600 dark:text-amber-400">
                      {t("truncatedHint")}
                    </span>
                  )}
                </div>
                {responseBody && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => copyToClipboard(responseBody, "response_body")}
                    className="h-7 text-xs gap-1.5"
                  >
                    {copiedKey === "response_body" ? (
                      <Check className="size-3 text-emerald-500" />
                    ) : (
                      <Copy className="size-3" />
                    )}
                    <span>{copiedKey === "response_body" ? "已复制" : "复制 JSON"}</span>
                  </Button>
                )}
              </div>
              {responseBody ? (
                <CodeBlock
                  className="min-h-0 flex-1 rounded-xl"
                  code={responseBody}
                  language="json"
                />
              ) : (
                <div className="flex h-36 items-center justify-center rounded-xl border border-dashed border-border text-xs text-muted-foreground">
                  {t("emptyBody")}
                </div>
              )}
            </TabsContent>
          </Tabs>
        )}
      </DialogContent>
    </Dialog>
  );
}
