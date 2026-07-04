import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  CalendarDays,
  ChevronDown,
  FileText,
  FolderGit,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import { showError, showSuccess } from "../stores/toastStore";
import { useAppStore } from "../stores/appStore";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { EmptyState } from "../components/ui/EmptyState";
import { Skeleton, SkeletonCard } from "../components/ui/Skeleton";
import DailySummaryView from "../components/DailySummaryView";
import type {
  DailySummary,
  DailySummaryMeta,
  SummaryOutcome,
  SummaryValueMetrics,
} from "../types";

const PAGE_DAYS = 30;

function todayIso(): string {
  const d = new Date();
  const off = d.getTimezoneOffset();
  const local = new Date(d.getTime() - off * 60_000);
  return local.toISOString().slice(0, 10);
}

function addDays(iso: string, days: number): string {
  const d = new Date(iso + "T00:00:00");
  d.setDate(d.getDate() + days);
  const off = d.getTimezoneOffset();
  const local = new Date(d.getTime() - off * 60_000);
  return local.toISOString().slice(0, 10);
}

function formatDateLabel(iso: string): string {
  const d = new Date(iso + "T00:00:00");
  const today = todayIso();
  const yesterday = addDays(today, -1);
  if (iso === today) return "今天";
  if (iso === yesterday) return "昨天";
  return `${d.getMonth() + 1}月${d.getDate()}日`;
}

function formatWeekday(iso: string): string {
  const days = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
  return days[new Date(iso + "T00:00:00").getDay()];
}

function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export default function DailySummaries() {
  const setSettingsSubTab = useAppStore((state) => state.setSettingsSubTab);
  const setActiveTab = useAppStore((state) => state.setActiveTab);

  const [metrics, setMetrics] = useState<SummaryValueMetrics | null>(null);
  const [metricsLoading, setMetricsLoading] = useState(true);

  const endDate = todayIso();
  const [startDate, setStartDate] = useState<string>(addDays(todayIso(), -PAGE_DAYS + 1));
  const [summaries, setSummaries] = useState<DailySummaryMeta[]>([]);
  const [listLoading, setListLoading] = useState(true);
  const [hasMore, setHasMore] = useState(true);

  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [summary, setSummary] = useState<DailySummary | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [genMessage, setGenMessage] = useState<string | null>(null);

  const loadMetrics = useCallback(async () => {
    setMetricsLoading(true);
    try {
      const m = await invoke<SummaryValueMetrics>("get_summary_value_metrics");
      setMetrics(m);
    } catch (err) {
      setMetrics(null);
    } finally {
      setMetricsLoading(false);
    }
  }, []);

  const loadList = useCallback(
    async (start: string, end: string, append: boolean) => {
      setListLoading(true);
      try {
        const rows = await invoke<DailySummaryMeta[]>("list_daily_summaries", {
          startDate: start,
          endDate: end,
        });
        setSummaries((prev) => {
          if (append) {
            const existing = new Set(prev.map((p) => p.date));
            return [...prev, ...rows.filter((r) => !existing.has(r.date))];
          }
          return rows;
        });
        setHasMore(rows.length >= PAGE_DAYS);
      } catch (err) {
        const msg =
          typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`加载摘要列表失败：${msg}`);
      } finally {
        setListLoading(false);
      }
    },
    []
  );

  const loadSummary = useCallback(async (date: string) => {
    setSummaryLoading(true);
    setGenMessage(null);
    try {
      const s = await invoke<DailySummary | null>("get_daily_summary", { date });
      setSummary(s);
    } catch {
      setSummary(null);
    } finally {
      setSummaryLoading(false);
    }
  }, []);

  useEffect(() => {
    loadMetrics();
  }, [loadMetrics]);

  useEffect(() => {
    loadList(startDate, endDate, false);
  }, [startDate, endDate, loadList]);

  // Auto-select today if it has a summary, otherwise the most recent summary.
  // Only runs once on first load; does not override a user-made selection.
  useEffect(() => {
    if (listLoading) return;
    if (selectedDate) return;
    if (summaries.length === 0) {
      setSelectedDate(todayIso());
      return;
    }
    const today = todayIso();
    if (summaries.some((s) => s.date === today)) {
      setSelectedDate(today);
    } else {
      setSelectedDate(summaries[0].date);
    }
  }, [listLoading, summaries, selectedDate]);

  useEffect(() => {
    if (selectedDate) {
      loadSummary(selectedDate);
    }
  }, [selectedDate, loadSummary]);

  const handleLoadMore = () => {
    const newStart = addDays(startDate, -PAGE_DAYS);
    setStartDate(newStart);
  };

  const handleGenerate = async () => {
    const date = selectedDate ?? todayIso();
    if (generating) return;
    setGenerating(true);
    setGenMessage(null);
    try {
      const outcome = await invoke<SummaryOutcome>("generate_daily_summary", { date });
      if (outcome.status === "generated") {
        setSummary(outcome.summary);
        showSuccess("摘要生成成功");
        await loadMetrics();
        await loadList(startDate, endDate, false);
      } else if (outcome.status === "no_key") {
        setGenMessage("未配置 AI，请在「偏好设置」配置后再生成。");
      } else if (outcome.status === "no_sessions") {
        setGenMessage("当日无会话，无需生成摘要。去使用洞察采集数据后再试。");
      } else {
        setGenMessage(outcome.reason ?? "生成失败");
      }
    } catch (err) {
      const msg =
        typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      setGenMessage(`生成失败：${msg}`);
    } finally {
      setGenerating(false);
    }
  };

  const metricCards = useMemo(
    () => [
      { label: "覆盖天数", value: metrics?.covered_days ?? 0, icon: CalendarDays },
      { label: "累计摘要", value: metrics?.total_summaries ?? 0, icon: FileText },
      { label: "累计活动", value: metrics?.total_activities ?? 0, icon: Sparkles },
      { label: "涉及项目", value: metrics?.covered_projects ?? 0, icon: FolderGit },
    ],
    [metrics]
  );

  return (
    <div className="h-full overflow-auto p-8">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">每日摘要</h1>
          <p className="mt-1 text-sm text-secondary">
            把每一天与 Agent 的协作沉淀为可回顾的工作记忆
          </p>
        </div>
      </div>

      {/* Value metrics */}
      <div className="mb-6 grid grid-cols-2 gap-4 lg:grid-cols-4">
        {metricsLoading
          ? Array.from({ length: 4 }).map((_, i) => (
              <SkeletonCard key={i} />
            ))
          : metricCards.map((m) => (
              <Card key={m.label} className="p-5">
                <div className="flex items-center gap-2 text-sm text-secondary">
                  <m.icon className="h-4 w-4 text-accent" />
                  {m.label}
                </div>
                <div className="mt-2 text-2xl font-bold text-primary">
                  {formatNumber(m.value)}
                </div>
              </Card>
            ))}
      </div>

      <div className="grid gap-6 lg:grid-cols-[320px_1fr]">
        {/* Date list */}
        <Card className="flex h-[calc(100vh-340px)] min-h-[400px] flex-col p-0">
          <div className="border-b border-[var(--border-subtle)] px-5 py-4">
            <div className="flex items-center gap-2 text-sm font-semibold text-primary">
              <CalendarDays className="h-4 w-4 text-accent" />
              日期列表
            </div>
          </div>

          <div className="flex-1 overflow-auto p-3">
            {listLoading && summaries.length === 0 ? (
              <div className="space-y-2">
                {Array.from({ length: 8 }).map((_, i) => (
                  <Skeleton key={i} className="h-12 w-full rounded-lg" />
                ))}
              </div>
            ) : summaries.length === 0 ? (
              <div className="py-6 text-center text-sm text-tertiary">
                最近 30 天暂无摘要
              </div>
            ) : (
              <ul className="space-y-1.5">
                {summaries.map((s) => {
                  const active = selectedDate === s.date;
                  return (
                    <li key={s.date}>
                      <button
                        onClick={() => setSelectedDate(s.date)}
                        className={`flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left text-sm transition-colors ${
                          active
                            ? "bg-accent/10 text-accent"
                            : "text-primary hover:bg-tertiary/60"
                        }`}
                      >
                        <span
                          className={`h-2 w-2 rounded-full ${
                            active ? "bg-accent" : "bg-emerald-500"
                          }`}
                        />
                        <span className="flex-1 font-medium">
                          {formatDateLabel(s.date)}
                        </span>
                        <span className="text-xs text-secondary">
                          {formatWeekday(s.date)}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}

            {hasMore && (
              <button
                onClick={handleLoadMore}
                disabled={listLoading}
                className="mt-3 flex w-full items-center justify-center gap-1 rounded-lg py-2 text-xs text-secondary transition-colors hover:bg-tertiary/60 disabled:opacity-50"
              >
                {listLoading ? (
                  <RefreshCw className="h-3 w-3 animate-spin" />
                ) : (
                  <ChevronDown className="h-3 w-3" />
                )}
                加载更早
              </button>
            )}
          </div>
        </Card>

        {/* Detail panel */}
        <Card className="flex h-[calc(100vh-340px)] min-h-[400px] flex-col p-0">
          <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-5 py-4">
            <div className="flex items-center gap-2 text-sm font-semibold text-primary">
              <FileText className="h-4 w-4 text-accent" />
              {selectedDate
                ? `${selectedDate} · ${formatWeekday(selectedDate)}`
                : "摘要详情"}
            </div>
            <Button
              variant="primary"
              size="sm"
              onClick={handleGenerate}
              loading={generating}
              disabled={generating}
            >
              <RefreshCw className={`h-4 w-4 ${generating ? "animate-spin" : ""}`} />
              {summary ? "重新生成" : "生成摘要"}
            </Button>
          </div>

          <div className="flex-1 overflow-auto p-5">
            {summaryLoading ? (
              <div className="space-y-4">
                <Skeleton className="h-24 w-full rounded-xl" />
                <Skeleton className="h-32 w-full rounded-xl" />
                <Skeleton className="h-32 w-full rounded-xl" />
              </div>
            ) : genMessage ? (
              <EmptyState
                icon={Sparkles}
                title={summary ? "摘要已存在" : "尚未生成摘要"}
                description={genMessage}
                action={
                  genMessage.includes("偏好设置") ? (
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={() => {
                        setSettingsSubTab("preferences");
                        setActiveTab("settings");
                      }}
                    >
                      去偏好设置
                    </Button>
                  ) : genMessage.includes("使用洞察") ? (
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={() => setActiveTab("usage")}
                    >
                      去使用洞察
                    </Button>
                  ) : null
                }
              />
            ) : summary ? (
              <DailySummaryView summary={summary} />
            ) : (
              <EmptyState
                icon={Sparkles}
                title="该日暂无摘要"
                description="点击右上角「生成摘要」，AI 会基于当日 Agent 会话生成一份工作日报。"
                action={
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={handleGenerate}
                    loading={generating}
                    disabled={generating}
                  >
                    生成摘要
                  </Button>
                }
              />
            )}
          </div>
        </Card>
      </div>
    </div>
  );
}
