import { useEffect, useMemo, useState } from "react";
import { invoke } from "../lib/invoke";
import {
  ArrowRight,
  Bot,
  CheckCircle2,
  Database,
  RefreshCw,
  ScanSearch,
  Settings,
  Sparkles,
} from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { useCollectionStore } from "../stores/collectionStore";
import { useDiscoveryDecision } from "../hooks/useDiscoveryDecision";
import { humanizeError, showError, showInfo, showSuccess } from "../stores/toastStore";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import type {
  Agent,
  CollectedSource,
  DailySummary,
  Discovery,
  DiscoveryKind,
  DismissReason,
  GrowthMetrics,
  SkillUsageSummary,
  SummaryOutcome,
  SyncAllResult,
  SyncTarget,
  WindowMetrics,
} from "../types";

function todayIso(): string {
  const d = new Date();
  const off = d.getTimezoneOffset();
  const local = new Date(d.getTime() - off * 60_000);
  return local.toISOString().slice(0, 10);
}

function yesterdayIso(): string {
  return addDays(todayIso(), -1);
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

function formatTimeAgo(ts: number): string {
  const diff = Math.max(0, Date.now() - ts * 1000);
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  return `${Math.floor(diff / 86_400_000)} 天前`;
}

interface DailySummaryMeta {
  date: string;
  created_at: number;
}

export default function Today() {
  const agents = useAppStore((state) => state.agents);
  const syncTargets = useAppStore((state) => state.syncTargets);
  const settings = useAppStore((state) => state.settings);
  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const navigateToSettings = useAppStore((state) => state.navigateToSettings);
  const loadData = useAppStore((state) => state.loadData);
  const navigateToSkillEditor = useAppStore((state) => state.navigateToSkillEditor);
  const bumpInboxRefresh = useAppStore((state) => state.bumpInboxRefresh);

  const hasAiKey = useMemo(() => {
    const ai = settings.ai;
    return ai?.models?.some((m) => m.api_key && m.api_key.trim().length > 0) ?? false;
  }, [settings.ai]);

  const collectionSources = useCollectionStore((state) => state.sources);
  const loadCollectionStatus = useCollectionStore((state) => state.loadStatus);
  const startCollect = useCollectionStore((state) => state.startCollect);
  const collecting = useCollectionStore((state) => state.collecting);

  const [summary, setSummary] = useState<DailySummary | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(true);
  const [fallbackMetrics, setFallbackMetrics] = useState<WindowMetrics | null>(null);
  const [discoveries, setDiscoveries] = useState<Discovery[] | null>(null);
  const [skillUsage, setSkillUsage] = useState<SkillUsageSummary[]>([]);
  const [growthMetrics, setGrowthMetrics] = useState<GrowthMetrics | null>(null);
  const [dailyMetas, setDailyMetas] = useState<DailySummaryMeta[]>([]);
  const [weeklySpark, setWeeklySpark] = useState<{ date: string; sessions: number }[]>([]);
  const [scanning, setScanning] = useState(false);
  const [syncPanelOpen, setSyncPanelOpen] = useState(false);

  const yesterday = yesterdayIso();

  const hasAgents = agents.length > 0;
  const hasCollectedSessions = useMemo(
    () => collectionSources.some((s) => s.record_count > 0),
    [collectionSources]
  );

  // On first mount, refresh collection status so onboarding checks are current.
  useEffect(() => {
    loadCollectionStatus();
  }, [loadCollectionStatus]);

  // Load yesterday's daily summary.
  useEffect(() => {
    let cancelled = false;
    setSummaryLoading(true);
    invoke<DailySummary | null>("get_daily_summary", { date: yesterday })
      .then((s) => {
        if (!cancelled) setSummary(s);
      })
      .catch(() => {
        if (!cancelled) setSummary(null);
      })
      .finally(() => {
        if (!cancelled) setSummaryLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [yesterday]);

  // Fallback metrics for the story card when no summary exists.
  useEffect(() => {
    if (summary) return;
    let cancelled = false;
    invoke<WindowMetrics>("get_window_metrics", { kind: "day", refDate: yesterday })
      .then((m) => {
        if (!cancelled) setFallbackMetrics(m);
      })
      .catch(() => {
        if (!cancelled) setFallbackMetrics(null);
      });
    return () => {
      cancelled = true;
    };
  }, [summary, yesterday]);

  // Pending discoveries for the inbox strip.
  useEffect(() => {
    let cancelled = false;
    invoke<Discovery[]>("list_discoveries", { status: "pending" })
      .then((rows) => {
        if (!cancelled) setDiscoveries(rows.filter((r) => r.status === "pending"));
      })
      .catch(() => {
        // M1: command missing is expected; hide the strip silently.
        if (!cancelled) setDiscoveries(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Skill usage for monthly citations.
  useEffect(() => {
    invoke<SkillUsageSummary[]>("get_skill_usage", { days: 30 })
      .then((rows) => setSkillUsage(rows))
      .catch(() => setSkillUsage([]));
  }, []);

  // Growth metrics for the asset pulse.
  useEffect(() => {
    invoke<GrowthMetrics>("get_growth_metrics")
      .then((m) => setGrowthMetrics(m))
      .catch(() => setGrowthMetrics(null));
  }, []);

  // Daily summaries for consecutive-day calculation.
  useEffect(() => {
    invoke<DailySummaryMeta[]>("list_daily_summaries", {
      startDate: addDays(todayIso(), -90),
      endDate: todayIso(),
    })
      .then((rows) => setDailyMetas(rows))
      .catch(() => setDailyMetas([]));
  }, []);

  // Build 7-day session sparkline from per-day window metrics.
  useEffect(() => {
    let cancelled = false;
    const dates = Array.from({ length: 7 }, (_, i) => addDays(yesterday, -6 + i));
    Promise.all(
      dates.map((date) =>
        invoke<WindowMetrics>("get_window_metrics", { kind: "day", refDate: date }).catch(
          () => null
        )
      )
    ).then((results) => {
      if (cancelled) return;
      setWeeklySpark(
        dates.map((date, idx) => {
          const m = results[idx];
          const sessions = m?.token_dimension?.scale?.model_calls ?? 0;
          return { date, sessions };
        })
      );
    });
    return () => {
      cancelled = true;
    };
  }, [yesterday]);

  const consecutiveDays = useMemo(() => {
    const dates = new Set(dailyMetas.map((m) => m.date));
    let count = 0;
    let d = yesterday;
    while (dates.has(d)) {
      count++;
      d = addDays(d, -1);
    }
    return count;
  }, [dailyMetas, yesterday]);

  const monthlyCitations = useMemo(
    () => skillUsage.reduce((sum, s) => sum + (s.usage_count ?? 0), 0),
    [skillUsage]
  );

  const assetTotal = useMemo(() => {
    if (growthMetrics?.skill_count_by_source) {
      return Object.values(growthMetrics.skill_count_by_source).reduce((a, b) => a + (b ?? 0), 0);
    }
    return skillUsage.length;
  }, [growthMetrics, skillUsage.length]);

  const handleScanAgents = async () => {
    if (scanning) return;
    setScanning(true);
    try {
      const scanned = await invoke<Agent[]>("scan_agents");
      await loadData();
      showSuccess(`扫描完成，发现 ${scanned.length} 个 Agent`);
    } catch (err) {
      showError(humanizeError(err, { context: "扫描 Agent" }));
    } finally {
      setScanning(false);
    }
  };

  const handleCollect = async () => {
    const jobId = await startCollect();
    if (jobId) {
      showInfo("采集任务已启动");
    }
  };

  const goToAiSettings = () => navigateToSettings("aiAnalysis");
  const goToInbox = () => setActiveTab("inbox");
  const goToUsage = () => setActiveTab("usage");

  const handleDiscoveryDecided = (id: string, action: "accept" | "accept_edited" | "dismiss", createdSkillId?: string) => {
    setDiscoveries((prev) => prev?.filter((d) => d.id !== id) ?? null);
    bumpInboxRefresh();
    if ((action === "accept" || action === "accept_edited") && createdSkillId) {
      navigateToSkillEditor(createdSkillId);
    }
  };

  // Full-page onboarding when there is no collection data at all.
  if (!hasAgents || !hasCollectedSessions) {
    return (
      <div className="h-full overflow-auto p-8">
        <div className="mx-auto max-w-2xl">
          <div className="mb-8">
            <h1 className="text-xl font-bold text-primary">欢迎来到 SkillMint</h1>
            <p className="mt-1 text-sm text-secondary">
              经验成长从记录开始。完成以下三步，明天就能在「今天」页看到第一份日报。
            </p>
          </div>

          <div className="space-y-4">
            <OnboardingStep
              number={1}
              title="扫描本机 Agent"
              description="发现你常用的 Agent 目录，建立同步目标。"
              done={hasAgents}
              action={
                <Button
                  variant={hasAgents ? "secondary" : "primary"}
                  size="sm"
                  onClick={handleScanAgents}
                  loading={scanning}
                  disabled={scanning}
                >
                  {hasAgents ? (
                    <>
                      <CheckCircle2 className="h-4 w-4" />
                      已扫描
                    </>
                  ) : (
                    <>
                      <ScanSearch className="h-4 w-4" />
                      立即扫描
                    </>
                  )}
                </Button>
              }
            />

            <OnboardingStep
              number={2}
              title="采集使用数据"
              description="读取本地 Agent 会话，为日报和发现提供原材料。"
              done={hasCollectedSessions}
              action={
                <Button
                  variant={hasCollectedSessions ? "secondary" : "primary"}
                  size="sm"
                  onClick={handleCollect}
                  loading={collecting}
                  disabled={collecting || !hasAgents}
                >
                  {hasCollectedSessions ? (
                    <>
                      <CheckCircle2 className="h-4 w-4" />
                      已采集
                    </>
                  ) : (
                    <>
                      <Database className="h-4 w-4" />
                      开始采集
                    </>
                  )}
                </Button>
              }
            />

            <OnboardingStep
              number={3}
              title="明早回来看你的第一份日报"
              description="采集完成后，夜间会生成昨日日报。你现在也可以去「偏好设置」配置 AI 增强。"
              done={false}
              action={
                <Button variant="secondary" size="sm" onClick={goToAiSettings}>
                  <Settings className="h-4 w-4" />
                  配置 AI 模型
                </Button>
              }
            />
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full overflow-auto p-8">
      <div className="mb-6 flex items-baseline justify-between">
        <div className="flex items-baseline gap-3">
          <h1 className="text-xl font-bold text-primary">今天</h1>
          <span className="text-xs text-tertiary font-mono">
            {todayIso()} {formatWeekday(todayIso())}
          </span>
        </div>
      </div>

      {/* 1. Yesterday story card */}
      <StoryCard
        summary={summary}
        fallbackMetrics={fallbackMetrics}
        loading={summaryLoading}
        onDeepDive={goToUsage}
        hasAiKey={hasAiKey}
        agents={agents}
        onGenerate={async () => {
          const outcome = await invoke<SummaryOutcome>("generate_daily_summary", { date: yesterday });
          if (outcome.status === "generated") {
            setSummary(outcome.summary);
            showSuccess("摘要生成成功");
          } else if (outcome.status === "no_key") {
            showInfo("配置 AI 模型后可自动生成");
          } else if (outcome.status === "no_sessions") {
            showInfo("昨日无会话，无需生成摘要");
          } else {
            showError(outcome.reason ?? "生成失败");
          }
        }}
        onGoSettings={goToAiSettings}
      />

      {/* 2. Pending discoveries strip */}
      {discoveries !== null && discoveries.length > 0 && (
        <section className="mt-5">
          <div className="mb-2.5 flex items-baseline justify-between">
            <div className="flex items-baseline gap-2">
              <h2 className="text-sm font-semibold text-secondary">待你裁决</h2>
              <span className="text-xs text-amber font-mono">
                {discoveries.length} 条发现
              </span>
            </div>
            <button
              onClick={goToInbox}
              className="flex items-center gap-1 text-xs text-accent hover:underline"
            >
              进收件箱逐条处理
              <ArrowRight className="h-3 w-3" />
            </button>
          </div>
          <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
            {discoveries.slice(0, 3).map((d, idx) => (
              <DiscoveryCard
                key={d.id}
                discovery={d}
                glowing={idx === 0}
                onDecided={handleDiscoveryDecided}
              />
            ))}
          </div>
        </section>
      )}

      {/* 3. Growth pulse */}
      <section className="mt-5 grid grid-cols-1 gap-3 lg:grid-cols-2">
        <Card className="flex items-center gap-4 p-4">
          <div>
            <div className="text-2xl font-semibold text-amber font-mono tabular-nums">
              {consecutiveDays}
            </div>
            <div className="text-xs text-secondary">连续记录天数</div>
          </div>
          <Sparkline
            data={weeklySpark.map((s) => s.sessions)}
            className="ml-auto"
            color="amber"
          />
        </Card>

        <Card className="flex items-center gap-4 p-4">
          <div>
            <div className="text-2xl font-semibold text-accent font-mono tabular-nums">
              {formatNumber(assetTotal)}
            </div>
            <div className="text-xs text-secondary">
              Skill 资产
              {monthlyCitations > 0 && ` · 本月被引用 ${monthlyCitations} 次`}
            </div>
          </div>
          <Sparkline
            data={skillUsage.slice(0, 7).map((s) => s.usage_count)}
            className="ml-auto"
            color="accent"
          />
        </Card>
      </section>

      {/* 4. Health bar */}
      <HealthBar
        syncTargets={syncTargets}
        sources={collectionSources}
        onOpenSyncPanel={() => setSyncPanelOpen(true)}
      />
      {syncPanelOpen && (
        <SyncStatusPanel
          syncTargets={syncTargets}
          agents={agents}
          onClose={() => setSyncPanelOpen(false)}
          onSync={async () => {
            try {
              const result = await invoke<SyncAllResult>("sync_all_command");
              if (result.failure_count > 0) {
                showError(
                  `同步完成：${result.success_count} 成功，${result.failure_count} 失败`,
                  5000
                );
              } else {
                showSuccess(`同步完成：${result.success_count} 个目标成功`);
              }
              await loadData();
            } catch (err) {
              showError(err, { context: "同步" });
            }
          }}
        />
      )}
    </div>
  );
}

function OnboardingStep({
  number,
  title,
  description,
  done,
  action,
}: {
  number: number;
  title: string;
  description: string;
  done: boolean;
  action: React.ReactNode;
}) {
  return (
    <Card
      className={`flex items-start gap-4 p-5 ${done ? "border-success/20 bg-success/5" : ""}`}
    >
      <div
        className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-sm font-semibold ${
          done
            ? "bg-success text-white"
            : "bg-tertiary text-secondary"
        }`}
      >
        {done ? <CheckCircle2 className="h-4 w-4" /> : number}
      </div>
      <div className="min-w-0 flex-1">
        <div className="font-medium text-primary">{title}</div>
        <p className="mt-0.5 text-sm text-secondary">{description}</p>
      </div>
      <div className="shrink-0">{action}</div>
    </Card>
  );
}

function StoryCard({
  summary,
  fallbackMetrics,
  loading,
  onDeepDive,
  hasAiKey,
  agents,
  onGenerate,
  onGoSettings,
}: {
  summary: DailySummary | null;
  fallbackMetrics: WindowMetrics | null;
  loading: boolean;
  onDeepDive: () => void;
  hasAiKey: boolean;
  agents: Agent[];
  onGenerate: () => Promise<void>;
  onGoSettings: () => void;
}) {
  if (loading) {
    return (
      <Card className="p-5">
        <div className="space-y-3">
          <div className="h-3 w-24 rounded bg-tertiary/50" />
          <div className="h-4 w-full rounded bg-tertiary/50" />
          <div className="h-4 w-2/3 rounded bg-tertiary/50" />
        </div>
      </Card>
    );
  }

  const hasSummary = summary && (summary.highlights.length > 0 || summary.activities.length > 0);
  const [generating, setGenerating] = useState(false);
  const [genHint, setGenHint] = useState<string | null>(null);

  const handleGenerate = async () => {
    if (generating) return;
    setGenerating(true);
    setGenHint(null);
    try {
      await onGenerate();
    } finally {
      setGenerating(false);
    }
  };

  if (!hasSummary) {
    const scale = fallbackMetrics?.token_dimension?.scale;
    const projectDistribution = fallbackMetrics?.token_dimension?.distribution?.by_project ?? {};
    const projects = Object.keys(projectDistribution).slice(0, 3);
    const mostActiveProject = projects.length > 0
      ? projects.reduce((a, b) => (projectDistribution[a] > projectDistribution[b] ? a : b))
      : null;
    const sessions = scale?.model_calls ?? 0;
    const tokens = scale?.total_tokens ?? 0;
    const agentCount = agents.length;

    if (sessions === 0) {
      return (
        <Card className="p-5">
          <div className="text-2xs font-mono uppercase tracking-wider text-tertiary mb-2">
            昨日 · {formatDateLabel(yesterdayIso())}
          </div>
          <p className="text-base text-primary">
            昨天还没有采集到会话数据。今天与 Agent 协作后，明早会在这里看到日报。
          </p>
        </Card>
      );
    }

    // SPEC-F4 T7: rule-based narrative when no LLM is configured.
    const narrative = hasAiKey
      ? `昨天共 ${sessions} 次会话。AI 摘要尚未生成。`
      : `昨天与 ${agentCount} 个 Agent 协作 ${sessions} 次会话，消耗 ${formatNumber(tokens)} token。${
          mostActiveProject ? `最活跃的项目是「${mostActiveProject}」。` : ""
        }`;

    return (
      <Card className="p-5">
        <div className="text-2xs font-mono uppercase tracking-wider text-tertiary mb-2">
          昨日 · {formatDateLabel(yesterdayIso())}
        </div>
        <p className="text-base text-primary">{narrative}</p>
        {genHint && <p className="mt-2 text-sm text-warning">{genHint}</p>}
        <div className="mt-3 flex flex-wrap items-center gap-3 border-t border-[var(--border-subtle)] pt-3 text-2xs text-tertiary font-mono">
          <span>
            会话 <b className="text-secondary">{sessions}</b>
          </span>
          <span>
            token <b className="text-secondary">{formatNumber(tokens)}</b>
          </span>
          {hasAiKey ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={handleGenerate}
              loading={generating}
              disabled={generating}
              className="ml-auto"
            >
              <Sparkles className="mr-1 h-3 w-3" />
              生成今日摘要
            </Button>
          ) : (
            <button onClick={onGoSettings} className="ml-auto text-accent hover:underline">
              配置 AI 模型后可自动生成 →
            </button>
          )}
        </div>
      </Card>
    );
  }

  const activities = summary!.activities.slice(0, 2);
  const scale = fallbackMetrics?.token_dimension?.scale;
  const sessions = scale?.model_calls ?? 0;
  const tokens = scale?.total_tokens ?? 0;
  const projects = Object.keys(fallbackMetrics?.token_dimension?.distribution?.by_project ?? {});

  return (
    <Card className="p-5">
      <div className="text-2xs font-mono uppercase tracking-wider text-tertiary mb-2">
        昨日 · {formatDateLabel(yesterdayIso())} · AI 记下了这些
      </div>
      <p className="text-base leading-relaxed text-primary">
        {summary!.highlights[0] ?? `${activities.map((a) => a.summary).join("；")}。`}
      </p>
      {summary!.highlights.length > 1 && (
        <div className="mt-3 flex flex-wrap gap-2">
          {summary!.highlights.slice(1).map((h, i) => (
            <span
              key={i}
              className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--border-subtle)] px-2.5 py-1 text-xs text-secondary"
            >
              <span className="h-1 w-1 rounded-full bg-info" />
              高亮 · {h}
            </span>
          ))}
        </div>
      )}
      <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 border-t border-[var(--border-subtle)] pt-3 text-2xs text-tertiary font-mono">
        {sessions > 0 && (
          <span>
            会话 <b className="text-secondary">{sessions}</b>
          </span>
        )}
        {tokens > 0 && (
          <span>
            token <b className="text-secondary">{formatNumber(tokens)}</b>
          </span>
        )}
        {projects.length > 0 && (
          <span>
            项目 <b className="text-secondary">{projects.length}</b>
          </span>
        )}
        <button onClick={onDeepDive} className="ml-auto text-accent hover:underline">
          深挖 → 洞察档案
        </button>
      </div>
    </Card>
  );
}

function discoveryKindLabel(kind: DiscoveryKind): string {
  switch (kind) {
    case "repeat_pattern":
      return "重复模式";
    case "high_value_prompt":
      return "高价值 Prompt";
    case "skill_feedback":
      return "Skill 效果反馈";
    case "capability_gap":
      return "能力缺口";
    default:
      return "发现";
  }
}

function DiscoveryCard({
  discovery,
  glowing,
  onDecided,
}: {
  discovery: Discovery;
  glowing: boolean;
  onDecided: (id: string, action: "accept" | "accept_edited" | "dismiss", createdSkillId?: string) => void;
}) {
  const evidence = discovery.payload?.evidence ?? [];
  const topEvidence = evidence[0];
  const [statsExpanded, setStatsExpanded] = useState(false);
  const [busy, setBusy] = useState(false);
  // SPEC-C2 T1: inline dismiss reason picker (shared with Inbox via hook).
  const [dismissOpen, setDismissOpen] = useState(false);

  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const setSkillLibrarySubTab = useAppStore((state) => state.setSkillLibrarySubTab);
  const requestNewSkill = useAppStore((state) => state.requestNewSkill);
  const setDiscoverSearchTerm = useAppStore((state) => state.setDiscoverSearchTerm);
  const { decide } = useDiscoveryDecision();

  const goDiscover = (term: string) => {
    setDiscoverSearchTerm(term);
    setActiveTab("discover");
  };

  const openNewSkillDialog = () => {
    setActiveTab("skillLibrary");
    setSkillLibrarySubTab("skills");
    requestNewSkill();
  };

  const handleDismiss = async (reason?: DismissReason) => {
    setBusy(true);
    try {
      await decide(discovery, "dismiss", reason);
      onDecided(discovery.id, "dismiss");
      showSuccess("已拒绝");
    } catch (err) {
      showError(humanizeError(err, { context: "拒绝发现" }));
    } finally {
      setBusy(false);
      setDismissOpen(false);
    }
  };

  // SPEC-C2 T1: accept (adopt-and-sync direct path).
  const handleAccept = async () => {
    setBusy(true);
    try {
      const result = await decide(discovery, "accept");
      if (result.mock) {
        showInfo("mock 环境：采纳并同步已降级为划掉");
        onDecided(discovery.id, "dismiss");
        return;
      }
      if (!result.created_skill_id) {
        showError("沉淀失败：未返回 Skill ID");
        return;
      }
      const summary = result.sync_summary;
      const agentCount = summary?.success ?? 0;
      if (summary && summary.failed > 0) {
        showInfo(`已入库，${summary.failed} 个 Agent 同步失败`);
      } else {
        showSuccess(agentCount > 0 ? `已入库并同步到 ${agentCount} 个 Agent` : "已入库");
      }
      onDecided(discovery.id, "accept", result.created_skill_id);
    } catch (err) {
      showError(humanizeError(err, { context: "采纳并同步" }));
    } finally {
      setBusy(false);
    }
  };

  // SPEC-C2 T1: accept_edited (create draft, navigate to editor).
  const handleAcceptEdited = async () => {
    setBusy(true);
    try {
      const result = await decide(discovery, "accept_edited");
      if (result.mock) {
        showInfo("mock 环境：修改后采纳已降级为划掉");
        onDecided(discovery.id, "dismiss");
        return;
      }
      if (!result.created_skill_id) {
        showError("创建草稿失败：未返回 Skill ID");
        return;
      }
      onDecided(discovery.id, "accept_edited", result.created_skill_id);
      showSuccess("草稿已创建，去编辑");
    } catch (err) {
      showError(humanizeError(err, { context: "修改后采纳" }));
    } finally {
      setBusy(false);
    }
  };

  const primaryAction = () => {
    switch (discovery.kind) {
      case "repeat_pattern":
      case "high_value_prompt":
        handleAccept();
        break;
      case "skill_feedback":
        setStatsExpanded((v) => !v);
        break;
      case "capability_gap":
        goDiscover(discovery.payload?.note || discovery.title);
        break;
    }
  };

  const hasThreeChoices =
    discovery.kind === "repeat_pattern" || discovery.kind === "high_value_prompt";
  const stats = discovery.payload?.stats;

  return (
    <Card
      className={`relative overflow-hidden border-l-2 border-l-amber p-4 ${
        glowing ? "shadow-[0_0_0_1px_var(--amber-glow),0_4px_24px_rgba(217,162,72,0.10)]" : ""
      }`}
    >
      <div className="mb-1.5 flex items-center gap-2 text-2xs font-mono uppercase tracking-wider text-amber">
        <span>◆ {discoveryKindLabel(discovery.kind)}</span>
        <span className="ml-auto text-tertiary normal-case tracking-normal">
          置信 {discovery.confidence.toFixed(2)}
        </span>
      </div>
      <h3 className="text-sm font-semibold leading-snug text-primary">{discovery.title}</h3>
      {topEvidence && (
        <div className="mt-2 truncate rounded-md border border-[var(--border-subtle)] bg-primary px-2 py-1.5 text-2xs text-tertiary font-mono">
          {topEvidence.prompt_text ?? topEvidence.session_id ?? "相关会话"}
        </div>
      )}

      {discovery.kind === "skill_feedback" && statsExpanded && stats && (
        <div className="mt-3 rounded-md border border-[var(--border-subtle)] bg-primary p-2.5">
          <div className="mb-1.5 font-mono text-2xs text-tertiary">使用效果对比</div>
          <div className="grid grid-cols-2 gap-2">
            {Object.entries(stats).map(([k, v]) => (
              <div key={k} className="rounded bg-secondary px-2 py-1">
                <div className="text-2xs text-secondary">{k}</div>
                <div className="font-mono text-xs text-primary">{v}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* SPEC-C2 T1: three-choice actions (compact version for Today strip). */}
      <div className="mt-3">
        {hasThreeChoices && !dismissOpen ? (
          <div className="flex items-center gap-1.5">
            <Button variant="primary" size="sm"
              className="!bg-amber !text-inverse hover:brightness-105"
              onClick={handleAccept} loading={busy} disabled={busy}>
              采纳并同步
            </Button>
            <Button variant="ghost" size="sm"
              onClick={handleAcceptEdited} disabled={busy}>
              修改
            </Button>
            <Button variant="ghost" size="sm"
              onClick={() => setDismissOpen(true)} disabled={busy}>
              拒绝
            </Button>
          </div>
        ) : hasThreeChoices && dismissOpen ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-2xs text-secondary">原因：</span>
            {([
              { v: "wrong", l: "内容不对" },
              { v: "trivial", l: "太琐碎" },
              { v: "duplicate", l: "重复" },
            ] as const).map((opt) => (
              <button key={opt.v} disabled={busy}
                onClick={() => handleDismiss(opt.v)}
                className="rounded border border-[var(--border-subtle)] bg-primary px-2 py-1 text-2xs text-secondary hover:text-danger">
                {opt.l}
              </button>
            ))}
            <button onClick={() => setDismissOpen(false)}
              className="ml-auto text-2xs text-tertiary hover:text-secondary">
              取消
            </button>
          </div>
        ) : discovery.kind === "skill_feedback" ? (
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="sm" onClick={primaryAction} disabled={busy}>
              {statsExpanded ? "收起对比" : "查看对比"}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => handleDismiss()} disabled={busy}>
              知道了
            </Button>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="sm" onClick={primaryAction} disabled={busy}>
              去发现页找
            </Button>
            <Button variant="ghost" size="sm" onClick={openNewSkillDialog} disabled={busy}>
              自己写
            </Button>
            <Button variant="ghost" size="sm" onClick={() => handleDismiss()} disabled={busy}>
              划掉
            </Button>
          </div>
        )}
      </div>
    </Card>
  );
}

function Sparkline({
  data,
  className,
  color,
}: {
  data: number[];
  className?: string;
  color: "amber" | "accent";
}) {
  const max = Math.max(1, ...data);
  const fillClass = color === "amber" ? "bg-amber" : "bg-accent";
  const dimClass = color === "amber" ? "bg-amber-dim" : "bg-accent-dim";

  return (
    <div className={`flex h-7 items-end gap-1 ${className ?? ""}`}>
      {data.map((v, i) => {
        const h = Math.max(2, Math.round((v / max) * 28));
        const hot = v >= max * 0.7;
        return (
          <div
            key={i}
            className={`w-1 rounded-sm ${hot ? fillClass : dimClass}`}
            style={{ height: `${h}px` }}
          />
        );
      })}
    </div>
  );
}

function HealthBar({
  syncTargets,
  sources,
  onOpenSyncPanel,
}: {
  syncTargets: SyncTarget[];
  sources: CollectedSource[];
  onOpenSyncPanel: () => void;
}) {
  const total = syncTargets.length;
  const synced = syncTargets.filter((t) => t.status === "synced").length;
  const conflict = syncTargets.filter((t) => t.status === "conflict").length;

  const lastCollectedAt = useMemo(() => {
    const timestamps = sources
      .map((s) => s.last_collected_at)
      .filter((ts): ts is number => typeof ts === "number" && ts > 0);
    return timestamps.length > 0 ? Math.max(...timestamps) : null;
  }, [sources]);

  return (
    <div className="mt-5 flex flex-wrap items-center gap-x-6 gap-y-2 text-2xs text-tertiary font-mono">
      <button
        onClick={onOpenSyncPanel}
        className="inline-flex items-center gap-1.5 hover:text-primary"
      >
        {conflict > 0 ? (
          <>
            <span className="h-1.5 w-1.5 rounded-full bg-danger" />
            同步 <span className="text-danger">{synced}/{total}</span>
          </>
        ) : (
          <>
            <span className="h-1.5 w-1.5 rounded-full bg-success" />
            同步健康 <span className="text-success">{synced}/{total}</span>
          </>
        )}
      </button>

      {lastCollectedAt ? (
        <span className="inline-flex items-center gap-1.5">
          <RefreshCw className="h-3 w-3" />
          采集 {formatTimeAgo(lastCollectedAt)}
        </span>
      ) : (
        <span className="inline-flex items-center gap-1.5">
          <RefreshCw className="h-3 w-3" />
          尚未采集
        </span>
      )}

      <span className="ml-auto inline-flex items-center gap-1.5">
        <Bot className="h-3 w-3" />
        skillmint ok
      </span>
    </div>
  );
}

function SyncStatusPanel({
  syncTargets,
  agents,
  onClose,
  onSync,
}: {
  syncTargets: SyncTarget[];
  agents: Agent[];
  onClose: () => void;
  onSync: () => Promise<void>;
}) {
  const [syncing, setSyncing] = useState(false);
  const agentMap = useMemo(() => {
    const map = new Map<string, Agent>();
    agents.forEach((a) => map.set(a.id, a));
    return map;
  }, [agents]);

  const doSync = async () => {
    setSyncing(true);
    try {
      await onSync();
    } finally {
      setSyncing(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={onClose}
    >
      <div
        className="max-h-[80vh] w-full max-w-md overflow-auto rounded-xl bg-primary p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-lg font-semibold text-primary">同步状态明细</h3>
          <button onClick={onClose} className="text-secondary hover:text-primary">
            ✕
          </button>
        </div>

        {syncTargets.length === 0 ? (
          <p className="py-4 text-center text-sm text-secondary">暂无同步目标</p>
        ) : (
          <div className="mb-4 space-y-2">
            {syncTargets.map((t) => (
              <div
                key={t.id}
                className="flex items-center justify-between rounded-lg border border-[var(--border-subtle)] bg-secondary px-4 py-3"
              >
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium text-primary">
                    {t.skill_name ?? t.skill_id}
                  </div>
                  <div className="text-xs text-secondary">
                    {agentMap.get(t.agent_id)?.name ?? t.agent_id} · {t.mode}
                  </div>
                </div>
                <div className="shrink-0 text-right text-xs">
                  <div
                    className={
                      t.status === "synced"
                        ? "text-success"
                        : t.status === "conflict"
                          ? "text-danger"
                          : "text-warning"
                    }
                  >
                    {t.status === "synced"
                      ? "已同步"
                      : t.status === "conflict"
                        ? "冲突"
                        : t.status === "local_changed"
                          ? "本地有变更"
                          : t.status === "center_changed"
                            ? "中心有变更"
                            : t.status}
                  </div>
                  {t.last_sync_at && (
                    <div className="text-tertiary">{formatTimeAgo(t.last_sync_at)}</div>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onClose}>
            关闭
          </Button>
          <Button variant="primary" size="sm" onClick={doSync} loading={syncing} disabled={syncing}>
            <RefreshCw className="mr-1 h-4 w-4" />
            立即同步
          </Button>
        </div>
      </div>
    </div>
  );
}
