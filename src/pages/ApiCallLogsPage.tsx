import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useHotkeys } from "react-hotkeys-hook";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import {
  Activity,
  AlertCircle,
  ArrowLeft,
  ChevronLeft,
  ChevronRight,
  Database,
  Filter,
  Layers,
  Loader2,
  RefreshCw,
  RotateCcw,
  SearchX,
  Timer,
  Zap,
} from "lucide-react";

import { ApiCallLogDetailDialog } from "@/components/apiLogs/ApiCallLogDetailDialog";
import { Button } from "@/components/ui/button";
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
import { cn, formatDate } from "@/lib/utils";

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

function StatusPill({ status, label }: { status: string; label: string }) {
  if (status === "success") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full border border-emerald-500/25 bg-emerald-500/10 px-2 py-0.5 text-[11px] font-medium text-emerald-600 dark:text-emerald-400">
        <span className="size-1.5 rounded-full bg-emerald-500" />
        {label}
      </span>
    );
  }
  if (status === "failed") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full border border-destructive/25 bg-destructive/10 px-2 py-0.5 text-[11px] font-medium text-destructive">
        <span className="size-1.5 rounded-full bg-destructive" />
        {label}
      </span>
    );
  }
  if (status === "cancelled") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full border border-border/70 bg-muted/60 px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
        <span className="size-1.5 rounded-full bg-muted-foreground/60" />
        {label}
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-border px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
      {label}
    </span>
  );
}

function KpiMetricCard({
  icon: Icon,
  title,
  value,
  subtext,
  iconClassName,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  value: React.ReactNode;
  subtext?: React.ReactNode;
  iconClassName?: string;
}) {
  return (
    <div className="rounded-2xl border border-border/70 bg-card/95 p-4 shadow-xs transition-all hover:border-border">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-muted-foreground">{title}</span>
        <div
          className={cn(
            "flex size-7 items-center justify-center rounded-lg bg-muted/50",
            iconClassName,
          )}
        >
          <Icon className="size-3.5" />
        </div>
      </div>
      <div className="mt-2 flex items-baseline gap-2">
        <div className="font-mono text-xl font-semibold tracking-tight text-foreground">
          {value}
        </div>
      </div>
      {subtext ? <div className="mt-1 text-[11px] text-muted-foreground/80">{subtext}</div> : null}
    </div>
  );
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
    <div className="flex h-screen flex-col bg-background select-none">
      {/* 顶部导航 Header */}
      <header className="flex h-14 shrink-0 items-center justify-between border-b border-border/70 bg-background/95 px-6 backdrop-blur-xs">
        <div className="flex items-center gap-3">
          <Link
            to="/settings/usage"
            className="group inline-flex items-center gap-1.5 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-all duration-150 hover:bg-muted/80 hover:text-foreground active:scale-[0.99]"
          >
            <ArrowLeft className="size-3.5 transition-transform duration-150 group-hover:-translate-x-0.5 text-muted-foreground group-hover:text-foreground" />
            <span>{t("settings:sections.usage")}</span>
          </Link>
          <div className="h-4 w-px bg-border/60" />
          <div className="flex items-center gap-2">
            <h1 className="text-sm font-semibold tracking-tight text-foreground">
              {t("listTitle")}
            </h1>
            {total > 0 && (
              <span className="rounded-full border border-border/60 bg-muted/40 px-2 py-0.5 font-mono text-[10px] font-medium text-muted-foreground">
                {total} 条记录
              </span>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void loadLogs(true, page)}
            disabled={loading || refreshing}
            className="h-8 gap-1.5 px-3 text-xs"
          >
            <RefreshCw className={cn("size-3.5", refreshing && "animate-spin")} />
            <span>{t("refresh")}</span>
            <kbd className="ml-1 inline-flex items-center rounded border border-border/70 bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
              {t("refreshHint")}
            </kbd>
          </Button>
        </div>
      </header>

      {/* 主工作区 */}
      <main className="min-h-0 flex-1 overflow-auto bg-muted/10 p-6">
        <div className="mx-auto max-w-7xl space-y-5">
          {errorMessage && (
            <div className="flex items-center gap-2.5 rounded-xl border border-destructive/30 bg-destructive/10 p-3.5 text-xs text-destructive">
              <AlertCircle className="size-4 shrink-0" />
              <span>{errorMessage}</span>
            </div>
          )}

          {/* KPI 统计卡片网格 */}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <KpiMetricCard
              icon={Activity}
              iconClassName="text-emerald-500 bg-emerald-500/10 dark:bg-emerald-500/15"
              title={t("statsCalls")}
              value={stats.total.toLocaleString()}
              subtext="全部发起请求记录"
            />
            <KpiMetricCard
              icon={Layers}
              iconClassName="text-primary bg-primary/10"
              title={t("statsTotal")}
              value={formatApiCallLogTokenCount(stats.total_tokens_sum, unknown)}
              subtext={
                <span>
                  {t("statsInput")}:{" "}
                  <span className="font-mono">
                    {formatApiCallLogTokenCount(stats.input_tokens, unknown)}
                  </span>{" "}
                  · {t("statsOutput")}:{" "}
                  <span className="font-mono">
                    {formatApiCallLogTokenCount(stats.output_tokens, unknown)}
                  </span>
                </span>
              }
            />
            <KpiMetricCard
              icon={Zap}
              iconClassName="text-amber-500 bg-amber-500/10 dark:bg-amber-500/15"
              title={t("statsCacheRate")}
              value={formatApiCallLogCacheRate(
                stats.input_tokens,
                stats.cached_tokens_sum,
                cacheRateLabels,
              )}
              subtext={
                <span>
                  {t("statsCached")}:{" "}
                  <span className="font-mono">
                    {formatApiCallLogTokenCount(stats.cached_tokens_sum, unknown)}
                  </span>
                </span>
              }
            />
            <KpiMetricCard
              icon={Timer}
              iconClassName="text-cyan-500 bg-cyan-500/10 dark:bg-cyan-500/15"
              title={t("statsAvgDuration")}
              value={formatApiCallLogDurationMs(stats.avg_duration_ms, unknown, lessThanOneSecond)}
              subtext={
                <span>
                  {t("statsAvgFirstToken")}:{" "}
                  <span className="font-mono">
                    {formatApiCallLogDurationMs(
                      stats.avg_first_token_ms,
                      unknown,
                      lessThanOneSecond,
                    )}
                  </span>
                </span>
              }
            />
          </div>

          {/* 筛选条件卡片 */}
          <div className="rounded-2xl border border-border/70 bg-card/95 p-4 shadow-xs space-y-3.5">
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-2">
                <Filter className="size-3.5 text-muted-foreground" />
                <span className="text-xs font-semibold tracking-tight text-foreground">
                  筛选日志
                </span>
                {hasActiveFilters && (
                  <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
                    已启用筛选
                  </span>
                )}
              </div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={!hasActiveFilters}
                onClick={() => {
                  setFilters(EMPTY_FILTERS);
                  setPage(1);
                }}
                className="h-7 gap-1.5 px-2 text-xs text-muted-foreground hover:text-foreground"
              >
                <RotateCcw className="size-3" />
                <span>{t("resetFilters")}</span>
              </Button>
            </div>

            <div className="grid gap-3 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-7">
              {/* Workspace */}
              <div className="space-y-1">
                <label
                  className="text-[11px] font-medium text-muted-foreground"
                  htmlFor="api-log-workspace"
                >
                  {t("workspace")}
                </label>
                <select
                  id="api-log-workspace"
                  className="h-8 w-full rounded-lg border border-input bg-background px-2.5 text-xs text-foreground transition-colors hover:bg-muted/40 focus:border-ring focus:outline-none"
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

              {/* Channel */}
              <div className="space-y-1">
                <label
                  className="text-[11px] font-medium text-muted-foreground"
                  htmlFor="api-log-channel"
                >
                  {t("channel")}
                </label>
                <Input
                  id="api-log-channel"
                  className="h-8 text-xs bg-background"
                  value={filters.channelName}
                  onChange={(event) =>
                    setFilters((current) => ({ ...current, channelName: event.target.value }))
                  }
                  placeholder={t("channelPlaceholder")}
                />
              </div>

              {/* Model */}
              <div className="space-y-1">
                <label
                  className="text-[11px] font-medium text-muted-foreground"
                  htmlFor="api-log-model"
                >
                  {t("model")}
                </label>
                <Input
                  id="api-log-model"
                  className="h-8 text-xs bg-background"
                  value={filters.model}
                  onChange={(event) =>
                    setFilters((current) => ({ ...current, model: event.target.value }))
                  }
                  placeholder={t("modelPlaceholder")}
                />
              </div>

              {/* Status */}
              <div className="space-y-1">
                <label
                  className="text-[11px] font-medium text-muted-foreground"
                  htmlFor="api-log-status"
                >
                  {t("status")}
                </label>
                <select
                  id="api-log-status"
                  className="h-8 w-full rounded-lg border border-input bg-background px-2.5 text-xs text-foreground transition-colors hover:bg-muted/40 focus:border-ring focus:outline-none"
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

              {/* Session ID */}
              <div className="space-y-1">
                <label
                  className="text-[11px] font-medium text-muted-foreground"
                  htmlFor="api-log-session-id"
                >
                  {t("sessionId")}
                </label>
                <Input
                  id="api-log-session-id"
                  className="h-8 text-xs bg-background"
                  value={filters.sessionId}
                  onChange={(event) =>
                    setFilters((current) => ({ ...current, sessionId: event.target.value }))
                  }
                  placeholder={t("sessionIdPlaceholder")}
                />
              </div>

              {/* Start Date */}
              <div className="space-y-1">
                <label
                  className="text-[11px] font-medium text-muted-foreground"
                  htmlFor="api-log-start-date"
                >
                  {t("startDate")}
                </label>
                <Input
                  id="api-log-start-date"
                  type="date"
                  className="h-8 text-xs bg-background"
                  value={filters.startDate}
                  onChange={(event) =>
                    setFilters((current) => ({ ...current, startDate: event.target.value }))
                  }
                />
              </div>

              {/* End Date */}
              <div className="space-y-1">
                <label
                  className="text-[11px] font-medium text-muted-foreground"
                  htmlFor="api-log-end-date"
                >
                  {t("endDate")}
                </label>
                <Input
                  id="api-log-end-date"
                  type="date"
                  className="h-8 text-xs bg-background"
                  value={filters.endDate}
                  onChange={(event) =>
                    setFilters((current) => ({ ...current, endDate: event.target.value }))
                  }
                />
              </div>
            </div>
          </div>

          {/* 表格主体卡片 */}
          <div className="rounded-2xl border border-border/70 bg-card/95 shadow-xs overflow-hidden">
            {loading ? (
              <div className="flex h-[26rem] flex-col items-center justify-center gap-2.5 text-sm text-muted-foreground">
                <Loader2 className="size-6 animate-spin text-primary" />
                <span>{t("loading")}</span>
              </div>
            ) : items.length === 0 ? (
              <div className="flex h-[26rem] flex-col items-center justify-center gap-2 text-center p-6">
                <div className="flex size-12 items-center justify-center rounded-2xl bg-muted/50 text-muted-foreground">
                  {hasActiveFilters ? (
                    <SearchX className="size-6" />
                  ) : (
                    <Database className="size-6" />
                  )}
                </div>
                <p className="text-sm font-medium text-foreground">
                  {hasInvalidDateRange
                    ? t("invalidDateRange")
                    : hasActiveFilters
                      ? t("emptyFiltered")
                      : t("empty")}
                </p>
                {hasActiveFilters && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setFilters(EMPTY_FILTERS);
                      setPage(1);
                    }}
                    className="mt-2 h-7 text-xs"
                  >
                    {t("resetFilters")}
                  </Button>
                )}
              </div>
            ) : (
              <div className="overflow-x-auto">
                <table className="min-w-full text-left text-xs">
                  <thead className="border-b border-border/70 bg-muted/40 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/80">
                    <tr>
                      <th className="px-4 py-3">{t("colChannel")}</th>
                      <th className="px-4 py-3">{t("colModel")}</th>
                      <th className="px-4 py-3">{t("colStatus")}</th>
                      <th className="px-4 py-3">{t("colThinking")}</th>
                      <th className="px-4 py-3">{t("colFormat")}</th>
                      <th className="px-4 py-3 text-right">{t("colInput")}</th>
                      <th className="px-4 py-3 text-right">{t("colOutput")}</th>
                      <th className="px-4 py-3 text-right">{t("colCached")}</th>
                      <th className="px-4 py-3 text-right">{t("colCacheRate")}</th>
                      <th className="px-4 py-3 text-right font-bold text-foreground">
                        {t("colTotal")}
                      </th>
                      <th className="px-4 py-3 text-right">{t("colFirstToken")}</th>
                      <th className="px-4 py-3 text-right">{t("colDuration")}</th>
                      <th className="px-4 py-3 text-right">{t("colThroughput")}</th>
                      <th className="px-4 py-3">{t("colCreatedAt")}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border/50">
                    {items.map((item) => (
                      <tr
                        key={item.id}
                        tabIndex={0}
                        className="group cursor-pointer transition-colors hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none"
                        onClick={() => openDetail(item.id)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            openDetail(item.id);
                          }
                        }}
                      >
                        {/* 渠道 */}
                        <td className="px-4 py-3 font-medium text-foreground">
                          <div
                            className="max-w-36 truncate font-medium"
                            title={item.channel_name ?? undefined}
                          >
                            {item.channel_name?.trim() || unknown}
                          </div>
                        </td>

                        {/* 模型 */}
                        <td className="px-4 py-3">
                          <span
                            className="inline-block max-w-44 truncate rounded-md border border-border/60 bg-muted/40 px-2 py-0.5 font-mono text-[11px] text-foreground"
                            title={item.model ?? undefined}
                          >
                            {item.model?.trim() || unknown}
                          </span>
                        </td>

                        {/* 状态 */}
                        <td className="px-4 py-3">
                          <StatusPill status={item.status} label={statusLabel(item.status)} />
                        </td>

                        {/* 思考等级 */}
                        <td className="px-4 py-3 text-muted-foreground">
                          {formatApiCallLogThinking(item, unknown, t("thinkingOff"))}
                        </td>

                        {/* 格式 */}
                        <td className="px-4 py-3 text-muted-foreground uppercase text-[10px]">
                          {item.request_format || unknown}
                        </td>

                        {/* 输入 Token */}
                        <td className="px-4 py-3 text-right font-mono tabular-nums text-muted-foreground">
                          {formatApiCallLogTokenCount(item.input_tokens, unknown)}
                        </td>

                        {/* 输出 Token */}
                        <td className="px-4 py-3 text-right font-mono tabular-nums text-muted-foreground">
                          {formatApiCallLogTokenCount(item.output_tokens, unknown)}
                        </td>

                        {/* 缓存 Token */}
                        <td className="px-4 py-3 text-right font-mono tabular-nums text-muted-foreground">
                          {formatApiCallLogTokenCount(item.cached_tokens, unknown)}
                        </td>

                        {/* 缓存命中率 */}
                        <td className="px-4 py-3 text-right font-mono tabular-nums text-muted-foreground">
                          {formatApiCallLogCacheRate(
                            item.input_tokens,
                            item.cached_tokens,
                            cacheRateLabels,
                          )}
                        </td>

                        {/* 总 Token */}
                        <td className="px-4 py-3 text-right font-mono tabular-nums font-semibold text-foreground">
                          {formatApiCallLogTokenCount(item.total_tokens, unknown)}
                        </td>

                        {/* 首字时间 */}
                        <td className="whitespace-nowrap px-4 py-3 text-right font-mono tabular-nums text-muted-foreground">
                          {formatApiCallLogDurationMs(
                            item.first_token_ms,
                            unknown,
                            lessThanOneSecond,
                          )}
                        </td>

                        {/* 总耗时 */}
                        <td className="whitespace-nowrap px-4 py-3 text-right font-mono tabular-nums text-muted-foreground">
                          {formatApiCallLogDurationMs(item.duration_ms, unknown, lessThanOneSecond)}
                        </td>

                        {/* 吞吐量 */}
                        <td className="whitespace-nowrap px-4 py-3 text-right font-mono tabular-nums text-muted-foreground">
                          {formatApiCallLogThroughput(
                            item.output_tokens,
                            item.duration_ms,
                            unknown,
                          )}
                        </td>

                        {/* 创建时间 */}
                        <td className="whitespace-nowrap px-4 py-3 text-[11px] text-muted-foreground">
                          {formatDate(item.created_at)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}

            {/* 分页条 */}
            <div className="flex items-center justify-between border-t border-border/70 bg-muted/20 px-4 py-3">
              <span className="text-xs text-muted-foreground">
                {total === 0
                  ? t("paginationEmpty")
                  : t("paginationRange", { start: rangeStart, end: rangeEnd, total })}
              </span>
              <div className="flex items-center gap-2.5">
                <span className="text-xs text-muted-foreground">
                  {total === 0
                    ? t("paginationPageEmpty")
                    : t("paginationPage", { page, totalPages })}
                </span>
                <div className="flex items-center gap-1">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setPage((current) => Math.max(1, current - 1))}
                    disabled={loading || page <= 1}
                    className="h-7 text-xs gap-1"
                  >
                    <ChevronLeft className="size-3.5" />
                    {t("prevPage")}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setPage((current) => current + 1)}
                    disabled={loading || total === 0 || page >= totalPages}
                    className="h-7 text-xs gap-1"
                  >
                    {t("nextPage")}
                    <ChevronRight className="size-3.5" />
                  </Button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </main>

      {/* 详情模态弹窗 */}
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
