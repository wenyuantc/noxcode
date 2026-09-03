import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useHotkeys } from "react-hotkeys-hook";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { ChevronLeft, ChevronRight, Loader2, RefreshCw } from "lucide-react";

import { ApiCallLogDetailDialog } from "@/components/apiLogs/ApiCallLogDetailDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  API_CALL_LOG_PAGE_SIZE,
  API_CALL_LOG_STATUSES,
  emptyApiCallLogStats,
  formatApiCallLogCacheRate,
  formatApiCallLogDurationMs,
  formatApiCallLogThinking,
  formatApiCallLogThroughput,
  formatApiCallLogTokenCount,
  nextApiCallLogRequestPage,
  resolveApiCallLogListPage,
} from "@/lib/apiLogs";
import { listNativeApiCallLogs, listWorkspaces } from "@/lib/backend";
import type { NativeApiCallLogListItem, NativeApiCallLogStats, Workspace } from "@/lib/types";
import { formatDate } from "@/lib/utils";

interface ApiCallLogFilterState {
  workspaceId: string;
  channelName: string;
  model: string;
  status: string;
  sessionId: string;
  startDate: string;
  endDate: string;
}

const EMPTY_FILTERS: ApiCallLogFilterState = {
  workspaceId: "",
  channelName: "",
  model: "",
  status: "all",
  sessionId: "",
  startDate: "",
  endDate: "",
};

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

export default function ApiCallLogsPage() {
  const { t } = useTranslation(["apiLogs", "settings"]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [filters, setFilters] = useState<ApiCallLogFilterState>(EMPTY_FILTERS);
  const [debouncedFilters, setDebouncedFilters] = useState<ApiCallLogFilterState>(EMPTY_FILTERS);
  const [items, setItems] = useState<NativeApiCallLogListItem[]>([]);
  const [stats, setStats] = useState<NativeApiCallLogStats>(emptyApiCallLogStats);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [detailOpen, setDetailOpen] = useState(false);
  const [selectedLogId, setSelectedLogId] = useState<string | null>(null);
  const loadGenerationRef = useRef(0);
  const lastRequestedPageRef = useRef(1);

  useEffect(() => {
    void listWorkspaces().then(setWorkspaces);
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedFilters(filters), 250);
    return () => window.clearTimeout(timer);
  }, [filters]);

  const hasInvalidDateRange = Boolean(
    debouncedFilters.startDate &&
    debouncedFilters.endDate &&
    debouncedFilters.startDate > debouncedFilters.endDate,
  );
  const hasActiveFilters =
    Boolean(filters.workspaceId) ||
    Boolean(filters.channelName.trim()) ||
    Boolean(filters.model.trim()) ||
    filters.status !== "all" ||
    Boolean(filters.sessionId.trim()) ||
    Boolean(filters.startDate) ||
    Boolean(filters.endDate);
  const unknown = t("unknown");
  const emptyValue = t("emptyValue");
  const lessThanOneSecond = t("lessThanOneSecond");
  const cacheRateLabels = { unknown, empty: emptyValue };
  const totalPages = total > 0 ? Math.ceil(total / API_CALL_LOG_PAGE_SIZE) : 0;
  const rangeStart = total === 0 ? 0 : (page - 1) * API_CALL_LOG_PAGE_SIZE + 1;
  const rangeEnd = total === 0 ? 0 : Math.min(page * API_CALL_LOG_PAGE_SIZE, total);

  const currentQuery = useMemo(
    () => ({
      workspaceId: debouncedFilters.workspaceId || null,
      channelName: debouncedFilters.channelName.trim() || null,
      model: debouncedFilters.model.trim() || null,
      status: debouncedFilters.status === "all" ? null : debouncedFilters.status,
      sessionId: debouncedFilters.sessionId.trim() || null,
      startDate: debouncedFilters.startDate || null,
      endDate: debouncedFilters.endDate || null,
    }),
    [debouncedFilters],
  );

  const closeDetail = () => {
    setDetailOpen(false);
    setSelectedLogId(null);
  };

  const loadLogs = useCallback(
    async (silent = false, requestedPage = 1) => {
      const generation = ++loadGenerationRef.current;
      const nextPage = nextApiCallLogRequestPage(false, requestedPage);
      lastRequestedPageRef.current = nextPage;
      if (hasInvalidDateRange) {
        if (generation !== loadGenerationRef.current) {
          return;
        }
        setItems([]);
        setTotal(0);
        setPage(1);
        lastRequestedPageRef.current = 1;
        setStats(emptyApiCallLogStats());
        setErrorMessage(t("invalidDateRange"));
        setLoading(false);
        setRefreshing(false);
        return;
      }

      if (silent) {
        setRefreshing(true);
      } else {
        setLoading(true);
      }
      setErrorMessage(null);

      try {
        const result = await listNativeApiCallLogs({
          limit: API_CALL_LOG_PAGE_SIZE,
          offset: (nextPage - 1) * API_CALL_LOG_PAGE_SIZE,
          workspace_id: currentQuery.workspaceId,
          channel_name: currentQuery.channelName,
          model: currentQuery.model,
          status: currentQuery.status,
          session_id: currentQuery.sessionId,
          start_date: currentQuery.startDate,
          end_date: currentQuery.endDate,
          include_total: true,
        });
        if (generation !== loadGenerationRef.current) {
          return;
        }

        const resolved = resolveApiCallLogListPage(nextPage, result);
        setItems(resolved.items);
        setTotal(resolved.total);
        setStats(result.stats);
        lastRequestedPageRef.current = resolved.page;
        setPage(resolved.page);

        if (resolved.needsRefetch) {
          void loadLogs(silent, resolved.page);
        }
      } catch (error) {
        if (generation !== loadGenerationRef.current) {
          return;
        }
        setItems([]);
        setTotal(0);
        setStats(emptyApiCallLogStats());
        setErrorMessage(error instanceof Error ? error.message : t("loadFailed"));
      } finally {
        if (generation === loadGenerationRef.current) {
          setLoading(false);
          setRefreshing(false);
        }
      }
    },
    [
      currentQuery.channelName,
      currentQuery.endDate,
      currentQuery.model,
      currentQuery.sessionId,
      currentQuery.startDate,
      currentQuery.status,
      currentQuery.workspaceId,
      hasInvalidDateRange,
      t,
    ],
  );

  useHotkeys(
    "r",
    (event) => {
      event.preventDefault();
      if (!loading && !refreshing) {
        void loadLogs(true, page);
      }
    },
    { enableOnFormTags: false },
  );

  const filterResetKey = [
    currentQuery.workspaceId ?? "",
    currentQuery.channelName ?? "",
    currentQuery.endDate ?? "",
    currentQuery.model ?? "",
    currentQuery.sessionId ?? "",
    currentQuery.startDate ?? "",
    currentQuery.status ?? "",
  ].join("|");

  useEffect(() => {
    closeDetail();
    setPage(1);
    void loadLogs(false, 1);
  }, [filterResetKey, loadLogs]);

  useEffect(() => {
    if (page === lastRequestedPageRef.current) {
      return;
    }
    void loadLogs(false, page);
    // Filter/scope changes always fetch page 1 directly. This effect only
    // loads when the user actually changes pages, including out-of-range
    // fallbacks that already wrote items/total from the latest response.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page]);

  const statusLabel = (status: string) => {
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
  };

  const openDetail = (id: string) => {
    setSelectedLogId(id);
    setDetailOpen(true);
  };

  return (
    <div className="flex h-screen flex-col bg-background">
      <header className="flex items-center gap-3 border-b px-6 py-3">
        <Link to="/settings/usage" className="text-sm text-muted-foreground hover:text-foreground">
          ← {t("settings:sections.usage")}
        </Link>
      </header>
      <main className="min-h-0 flex-1 overflow-auto p-6">
        <div className="space-y-4">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h2 className="text-lg font-semibold">{t("listTitle")}</h2>
            </div>
            <Button
              type="button"
              variant="outline"
              onClick={() => void loadLogs(true, page)}
              disabled={loading || refreshing}
            >
              {refreshing ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="mr-2 h-4 w-4" />
              )}
              {t("refresh")}
              <span className="ml-1.5 rounded border px-1 text-[10px] text-muted-foreground">
                {t("refreshHint")}
              </span>
            </Button>
          </div>

          {errorMessage && (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              {errorMessage}
            </div>
          )}

          <Card>
            <CardContent className="space-y-4">
              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                <div className="space-y-1.5">
                  <label className="text-sm font-medium" htmlFor="api-log-workspace">
                    {t("workspace")}
                  </label>
                  <select
                    id="api-log-workspace"
                    className="h-8 w-full rounded-md border border-input bg-background px-2 text-sm"
                    value={filters.workspaceId}
                    onChange={(event) =>
                      setFilters((current) => ({ ...current, workspaceId: event.target.value }))
                    }
                  >
                    <option value="">{t("allWorkspaces")}</option>
                    {workspaces.map((item) => (
                      <option key={item.id} value={item.id}>
                        {item.name}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="space-y-1.5">
                  <label className="text-sm font-medium" htmlFor="api-log-channel">
                    {t("channel")}
                  </label>
                  <Input
                    id="api-log-channel"
                    value={filters.channelName}
                    onChange={(event) =>
                      setFilters((current) => ({ ...current, channelName: event.target.value }))
                    }
                    placeholder={t("channelPlaceholder")}
                  />
                </div>
                <div className="space-y-1.5">
                  <label className="text-sm font-medium" htmlFor="api-log-model">
                    {t("model")}
                  </label>
                  <Input
                    id="api-log-model"
                    value={filters.model}
                    onChange={(event) =>
                      setFilters((current) => ({ ...current, model: event.target.value }))
                    }
                    placeholder={t("modelPlaceholder")}
                  />
                </div>
                <div className="space-y-1.5">
                  <label className="text-sm font-medium" htmlFor="api-log-status">
                    {t("status")}
                  </label>
                  <select
                    id="api-log-status"
                    className="h-8 w-full rounded-md border border-input bg-background px-2 text-sm"
                    value={filters.status}
                    onChange={(event) =>
                      setFilters((current) => ({ ...current, status: event.target.value }))
                    }
                  >
                    <option value="all">{t("allStatuses")}</option>
                    {API_CALL_LOG_STATUSES.map((status) => (
                      <option key={status} value={status}>
                        {statusLabel(status)}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="space-y-1.5">
                  <label className="text-sm font-medium" htmlFor="api-log-session-id">
                    {t("sessionId")}
                  </label>
                  <Input
                    id="api-log-session-id"
                    value={filters.sessionId}
                    onChange={(event) =>
                      setFilters((current) => ({ ...current, sessionId: event.target.value }))
                    }
                    placeholder={t("sessionIdPlaceholder")}
                  />
                </div>
                <div className="space-y-1.5">
                  <label className="text-sm font-medium" htmlFor="api-log-start-date">
                    {t("startDate")}
                  </label>
                  <Input
                    id="api-log-start-date"
                    type="date"
                    value={filters.startDate}
                    onChange={(event) =>
                      setFilters((current) => ({ ...current, startDate: event.target.value }))
                    }
                  />
                </div>
                <div className="space-y-1.5">
                  <label className="text-sm font-medium" htmlFor="api-log-end-date">
                    {t("endDate")}
                  </label>
                  <Input
                    id="api-log-end-date"
                    type="date"
                    value={filters.endDate}
                    onChange={(event) =>
                      setFilters((current) => ({ ...current, endDate: event.target.value }))
                    }
                  />
                </div>
              </div>

              <div className="flex justify-end">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={!hasActiveFilters}
                  onClick={() => {
                    setFilters(EMPTY_FILTERS);
                    setPage(1);
                  }}
                >
                  {t("resetFilters")}
                </Button>
              </div>

              <div className="rounded-xl border border-border/70 p-3">
                <div className="mb-2 text-sm font-medium">{t("statsTitle")}</div>
                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-8">
                  <StatCell label={t("statsCalls")} value={String(stats.total)} />
                  <StatCell
                    label={t("statsInput")}
                    value={formatApiCallLogTokenCount(stats.input_tokens, unknown)}
                  />
                  <StatCell
                    label={t("statsOutput")}
                    value={formatApiCallLogTokenCount(stats.output_tokens, unknown)}
                  />
                  <StatCell
                    label={t("statsCached")}
                    value={formatApiCallLogTokenCount(stats.cached_tokens_sum, unknown)}
                  />
                  <StatCell
                    label={t("statsCacheRate")}
                    value={formatApiCallLogCacheRate(
                      stats.input_tokens,
                      stats.cached_tokens_sum,
                      cacheRateLabels,
                    )}
                  />
                  <StatCell
                    label={t("statsTotal")}
                    value={formatApiCallLogTokenCount(stats.total_tokens_sum, unknown)}
                  />
                  <StatCell
                    label={t("statsAvgFirstToken")}
                    value={formatApiCallLogDurationMs(
                      stats.avg_first_token_ms,
                      unknown,
                      lessThanOneSecond,
                    )}
                  />
                  <StatCell
                    label={t("statsAvgDuration")}
                    value={formatApiCallLogDurationMs(
                      stats.avg_duration_ms,
                      unknown,
                      lessThanOneSecond,
                    )}
                  />
                </div>
              </div>

              {loading ? (
                <div className="flex h-[28rem] items-center justify-center rounded-xl border border-border/70 text-sm text-muted-foreground">
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  {t("loading")}
                </div>
              ) : items.length === 0 ? (
                <div className="flex h-[28rem] items-center justify-center rounded-xl border border-border/70 text-sm text-muted-foreground">
                  {hasInvalidDateRange
                    ? t("invalidDateRange")
                    : hasActiveFilters
                      ? t("emptyFiltered")
                      : t("empty")}
                </div>
              ) : (
                <div className="overflow-hidden rounded-xl border border-border/70">
                  <div className="overflow-x-auto">
                    <table className="min-w-full text-sm">
                      <thead className="bg-muted/40 text-left">
                        <tr className="border-b border-border">
                          <th className="px-4 py-3 font-medium">{t("colChannel")}</th>
                          <th className="px-4 py-3 font-medium">{t("colModel")}</th>
                          <th className="px-4 py-3 font-medium">{t("colThinking")}</th>
                          <th className="px-4 py-3 font-medium">{t("colFormat")}</th>
                          <th className="px-4 py-3 font-medium">{t("colInput")}</th>
                          <th className="px-4 py-3 font-medium">{t("colOutput")}</th>
                          <th className="px-4 py-3 font-medium">{t("colCached")}</th>
                          <th className="px-4 py-3 font-medium">{t("colCacheRate")}</th>
                          <th className="px-4 py-3 font-medium">{t("colTotal")}</th>
                          <th className="px-4 py-3 font-medium">{t("colFirstToken")}</th>
                          <th className="px-4 py-3 font-medium">{t("colDuration")}</th>
                          <th className="px-4 py-3 font-medium">{t("colThroughput")}</th>
                          <th className="px-4 py-3 font-medium">{t("colCreatedAt")}</th>
                          <th className="px-4 py-3 font-medium">{t("colStatus")}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {items.map((item) => (
                          <tr
                            key={item.id}
                            tabIndex={0}
                            className="cursor-pointer border-b border-border/60 align-top last:border-b-0 hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none"
                            onClick={() => openDetail(item.id)}
                            onKeyDown={(event) => {
                              if (event.key === "Enter" || event.key === " ") {
                                event.preventDefault();
                                openDetail(item.id);
                              }
                            }}
                          >
                            <td className="px-4 py-3">
                              <div
                                className="max-w-40 truncate"
                                title={item.channel_name ?? undefined}
                              >
                                {item.channel_name?.trim() || unknown}
                              </div>
                            </td>
                            <td className="px-4 py-3">
                              <div
                                className="max-w-40 truncate font-mono text-xs"
                                title={item.model ?? undefined}
                              >
                                {item.model?.trim() || unknown}
                              </div>
                            </td>
                            <td className="px-4 py-3">
                              {formatApiCallLogThinking(item, unknown, t("thinkingOff"))}
                            </td>
                            <td className="px-4 py-3">{item.request_format || unknown}</td>
                            <td className="px-4 py-3">
                              {formatApiCallLogTokenCount(item.input_tokens, unknown)}
                            </td>
                            <td className="px-4 py-3">
                              {formatApiCallLogTokenCount(item.output_tokens, unknown)}
                            </td>
                            <td className="px-4 py-3">
                              {formatApiCallLogTokenCount(item.cached_tokens, unknown)}
                            </td>
                            <td className="px-4 py-3">
                              {formatApiCallLogCacheRate(
                                item.input_tokens,
                                item.cached_tokens,
                                cacheRateLabels,
                              )}
                            </td>
                            <td className="px-4 py-3">
                              {formatApiCallLogTokenCount(item.total_tokens, unknown)}
                            </td>
                            <td className="whitespace-nowrap px-4 py-3">
                              {formatApiCallLogDurationMs(
                                item.first_token_ms,
                                unknown,
                                lessThanOneSecond,
                              )}
                            </td>
                            <td className="whitespace-nowrap px-4 py-3">
                              {formatApiCallLogDurationMs(
                                item.duration_ms,
                                unknown,
                                lessThanOneSecond,
                              )}
                            </td>
                            <td className="whitespace-nowrap px-4 py-3">
                              {formatApiCallLogThroughput(
                                item.output_tokens,
                                item.duration_ms,
                                unknown,
                              )}
                            </td>
                            <td className="whitespace-nowrap px-4 py-3 text-xs text-muted-foreground">
                              {formatDate(item.created_at)}
                            </td>
                            <td className="px-4 py-3">
                              <Badge variant={statusBadgeVariant(item.status)}>
                                {statusLabel(item.status)}
                              </Badge>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}

              <div className="flex items-center justify-between gap-3">
                <span className="text-xs text-muted-foreground">
                  {total === 0
                    ? t("paginationEmpty")
                    : t("paginationRange", { start: rangeStart, end: rangeEnd, total })}
                </span>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground">
                    {total === 0
                      ? t("paginationPageEmpty")
                      : t("paginationPage", { page, totalPages })}
                  </span>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setPage((current) => Math.max(1, current - 1))}
                    disabled={loading || page <= 1}
                  >
                    <ChevronLeft className="h-3.5 w-3.5" />
                    {t("prevPage")}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setPage((current) => current + 1)}
                    disabled={loading || total === 0 || page >= totalPages}
                  >
                    {t("nextPage")}
                    <ChevronRight className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      </main>

      <ApiCallLogDetailDialog
        open={detailOpen}
        logId={selectedLogId}
        onOpenChange={(open) => {
          if (open) {
            setDetailOpen(true);
            return;
          }
          closeDetail();
        }}
      />
    </div>
  );
}

function StatCell({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="truncate text-sm font-medium" title={value}>
        {value}
      </div>
    </div>
  );
}
