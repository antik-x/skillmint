import { useCallback, useEffect, useState } from "react";
import { invoke } from "../lib/invoke";
import {
  Check,
  FileText,
  Lightbulb,
  RefreshCw,
  TrendingDown,
  TrendingUp,
  X,
} from "lucide-react";
import { showError, showSuccess } from "../stores/toastStore";
import { useCollectionStore } from "../stores/collectionStore";
import { useAppStore } from "../stores/appStore";
import { Button } from "../components/ui/Button";
import DataCollectionPanel from "../components/DataCollectionPanel";
import { HelpTip } from "../components/ui/HelpTip";
import { Skeleton, SkeletonList } from "../components/ui/Skeleton";
import type {
  AgentUsageSummary,
  HighValuePrompt,
  WindowMetrics,
  Delta,
  EntityMetrics,
  ClassifyResult,
  SummaryOutcome,
  DailySummary,
  CellPrompt,
  Skill,
} from "../types";

type DaysOption = 1 | 7 | 30 | 0;
type WindowKind = "day" | "week" | "month";

const DAYS_OPTIONS: { value: DaysOption; label: string }[] = [
  { value: 1, label: "今天" },
  { value: 7, label: "近 7 天" },
  { value: 30, label: "近 30 天" },
  { value: 0, label: "全部" },
];

function isPreset(days: number): days is DaysOption {
  return days === 1 || days === 7 || days === 30 || days === 0;
}

const PROMPT_TAG_RE = /<\/?[^>]+>|local-command-caveat/g;

/**
 * SPEC-F5 T2: agent-injected human-readable noise patterns, applied per-line
 * after tag stripping and before whitespace collapse. Each entry is either a
 * prefix match (drop the whole line) or a regex (remove the matched span).
 *
 * Append-only — keep this list as a constant so future noise can be added in
 * one place.
 */
const PROMPT_NOISE_PATTERNS: { kind: "line-prefix" | "regex"; pattern: RegExp }[] = [
  // `Caveat: ...` sentences: strip from "Caveat:" to the next period or EOL.
  { kind: "regex", pattern: /Caveat:.*?(\.|$)/g },
  // `[Request interrupted...]`, `[Image: ...]`, `[File: ...]`, `[Attachment: ...]`
  { kind: "regex", pattern: /\[(?:Request interrupted[^\]]*|Image:[^\]]*|File:[^\]]*|Attachment:[^\]]*)\]/g },
  // `(Bash completed...)`, `(Tool ...)`
  { kind: "regex", pattern: /\((?:Bash completed[^)]*|Tool[^)]*)\)/g },
  // `The TodoWrite tool ...` to end of line.
  { kind: "line-prefix", pattern: /^\s*The TodoWrite tool.*$/ },
];

/** SPEC-F4 T10 / SPEC-F5 T2: strip tags/caveats + agent noise, collapse whitespace, truncate with expand. */
export function cleanPromptText(text: string, limit = 160): { display: string; truncated: boolean } {
  // Reset global-regex state up front: the PROMPT_NOISE_PATTERNS and tag regexes
  // are module-level `/g` objects, so their `lastIndex` would otherwise leak
  // across calls (e.g. when the list renders many prompts in sequence) and cause
  // later calls to silently miss matches.
  PROMPT_TAG_RE.lastIndex = 0;
  const afterTags = text.replace(PROMPT_TAG_RE, "");

  // Apply noise patterns line-by-line so prefix rules can drop whole lines.
  const lines = afterTags.split(/\r?\n/);
  const cleaned = lines
    .map((line) => {
      let out = line;
      for (const rule of PROMPT_NOISE_PATTERNS) {
        rule.pattern.lastIndex = 0;
        if (rule.kind === "line-prefix") {
          if (rule.pattern.test(out)) {
            rule.pattern.lastIndex = 0;
            out = "";
          }
        } else {
          out = out.replace(rule.pattern, "");
        }
      }
      return out;
    })
    .join("\n");

  const stripped = cleaned.replace(/\s+/g, " ").trim();
  if (!stripped) {
    // Everything was noise — fall back to the original text (tag-stripped) so we
    // never render an empty card.
    const fallback = afterTags.replace(/\s+/g, " ").trim().slice(0, limit).trimEnd();
    return { display: fallback, truncated: text.length > limit };
  }
  if (stripped.length <= limit) return { display: stripped, truncated: false };
  return { display: stripped.slice(0, limit).trimEnd(), truncated: true };
}

export function sourceLabel(source?: string): string {
  if (!source) return "未知来源";
  switch (source.toLowerCase()) {
    case "claude-code":
      return "Claude Code";
    case "codex":
      return "Codex";
    case "zcode":
      return "ZCode";
    case "cursor":
      return "Cursor";
    default:
      return source;
  }
}

/** Format a raw token count into a compact human string (supports negatives and B). */
function formatTokens(n: number): string {
  const abs = Math.abs(n);
  const sign = n < 0 ? "-" : "";
  if (abs >= 1_000_000_000) return `${sign}${(abs / 1_000_000_000).toFixed(2)}B`;
  if (abs >= 1_000_000) return `${sign}${(abs / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `${sign}${(abs / 1_000).toFixed(1)}K`;
  return `${sign}${abs}`;
}

function formatCny(n: number): string {
  const abs = Math.abs(n);
  const sign = n < 0 ? "-" : "";
  if (abs >= 1_000_000) return `${sign}¥${(abs / 1_000_000).toFixed(2)}M`;
  if (abs >= 1_000) return `${sign}¥${(abs / 1_000).toFixed(2)}k`;
  return `${sign}¥${abs.toFixed(2)}`;
}

const BILLING_LABELS: Record<string, string> = {
  pay_as_you_go: "按量付费",
  subscription: "订阅",
};

function formatBillingMix(mix: Record<string, number>): string {
  const entries = Object.entries(mix).filter(([, v]) => v !== 0);
  if (entries.length === 0) return "—";
  return entries
    .map(([k, v]) => `${BILLING_LABELS[k] ?? k} ${formatCny(v)}`)
    .join(" · ");
}

function todayIso(): string {
  const d = new Date();
  const off = d.getTimezoneOffset();
  const local = new Date(d.getTime() - off * 60_000);
  return local.toISOString().slice(0, 10);
}

/** Render a 环比/同比 delta as a colored arrow + pct, or "—" when unavailable. */
function DeltaPill({ delta }: { delta?: Delta | null }) {
  if (!delta) return <span className="text-tertiary">同比 —</span>;
  if (delta.pct === null || delta.pct === undefined) {
    return <span className="text-tertiary">环比 基线为0</span>;
  }
  const up = delta.pct >= 0;
  return (
    <span className={up ? "text-success" : "text-danger"}>
      环比 {up ? <TrendingUp className="inline h-3 w-3" /> : <TrendingDown className="inline h-3 w-3" />} {Math.abs(delta.pct * 100).toFixed(1)}%
    </span>
  );
}

/** A horizontal distribution bar: renders entries as proportional segments. */
function DistributionBars({
  data,
  total,
  colors,
}: {
  data: Record<string, number>;
  total: number;
  colors: string[];
}) {
  const entries = Object.entries(data)
    .filter(([, v]) => v > 0)
    .sort((a, b) => b[1] - a[1]);
  if (entries.length === 0 || total <= 0) {
    return <div className="text-xs text-tertiary">暂无数据</div>;
  }
  return (
    <ul className="space-y-1.5">
      {entries.map(([label, value], i) => {
        const pct = (value / total) * 100;
        const color = colors[i % colors.length];
        return (
          <li key={label} className="flex items-center gap-3 text-xs">
            <span className="w-20 shrink-0 truncate text-primary" title={label}>
              {label}
            </span>
            <div className="h-2 flex-1 overflow-hidden rounded-full bg-tertiary/50">
              <div className={`h-full rounded-full ${color}`} style={{ width: `${pct}%` }} />
            </div>
            <span className="w-16 shrink-0 text-right text-secondary">
              {pct.toFixed(1)}%
            </span>
          </li>
        );
      })}
    </ul>
  );
}

const SEGMENT_COLORS = [
  "bg-accent",
  "bg-sky-500",
  "bg-violet-500",
  "bg-amber-500",
  "bg-emerald-500",
  "bg-rose-500",
  "bg-indigo-400",
  "bg-teal-400",
];

// PRD-08 P3-A: the selectable KPI catalog. Each entry maps a comparison-table
// key to its display metadata. The user picks which 4 to show as cards; the
// selection persists in localStorage.
type KpiColor = "accent" | "success" | "warning" | "info";
interface KpiDef {
  key: string;
  title: string;
  color: KpiColor;
  format: (v: number, m: WindowMetrics) => string;
  hint?: (m: WindowMetrics) => string | undefined;
}

const KPI_DEFS: KpiDef[] = [
  { key: "sessions", title: "会话数", color: "info", format: (v) => String(v) },
  {
    key: "total_tokens",
    title: "总 Token",
    color: "accent",
    format: (v) => formatTokens(v),
    hint: (m) => `fresh ${formatTokens(m.token_dimension.scale?.fresh_tokens ?? 0)}`,
  },
  {
    key: "est_cost_cny",
    title: "估算成本",
    color: "warning",
    format: (v) => formatCny(v),
    hint: () => "参考估算（CNY）",
  },
  {
    key: "quality_score",
    title: "质量评分",
    color: "success",
    format: (v) => `${v} / 100`,
    hint: (m) => `分类率 ${(m.prompt_dimension.semantics.classified_ratio * 100).toFixed(0)}%`,
  },
  { key: "fresh_tokens", title: "Fresh Token", color: "accent", format: (v) => formatTokens(v) },
  { key: "cache_ratio", title: "缓存命中率", color: "info", format: (v) => `${(v * 100).toFixed(1)}%` },
  { key: "prompts", title: "Prompt 数", color: "success", format: (v) => String(v) },
  { key: "agent_coeff_min", title: "Agent 系数", color: "warning", format: (v) => `${v.toFixed(1)} min` },
  { key: "tool_calls", title: "工具调用", color: "info", format: (v) => String(v) },
];

const KPI_DEFAULT = ["sessions", "total_tokens", "est_cost_cny", "quality_score"];
const KPI_STORAGE_KEY = "skillmint.kpi-cards";

function loadKpiSelection(): string[] {
  try {
    const raw = localStorage.getItem(KPI_STORAGE_KEY);
    if (!raw) return KPI_DEFAULT;
    const arr = JSON.parse(raw);
    if (Array.isArray(arr)) {
      // Keep only known keys; pad/truncate to exactly 4.
      const valid = arr.filter((k) => KPI_DEFS.some((d) => d.key === k));
      return valid.slice(0, 4).concat(KPI_DEFAULT.filter((k) => !valid.includes(k))).slice(0, 4);
    }
  } catch {
    /* fall through to default */
  }
  return KPI_DEFAULT;
}

/** PRD-08 P3-A: the customizable 4-card KPI grid. */
function KpiGrid({ metrics }: { metrics: WindowMetrics }) {
  const [selection, setSelection] = useState<string[]>(loadKpiSelection);
  const [editing, setEditing] = useState(false);

  const saveSelection = (next: string[]) => {
    setSelection(next);
    try {
      localStorage.setItem(KPI_STORAGE_KEY, JSON.stringify(next));
    } catch {
      /* localStorage may be unavailable; selection is in-memory only then */
    }
  };

  const toggle = (key: string) => {
    if (selection.includes(key)) {
      // Don't allow fewer than 1 card.
      if (selection.length <= 1) return;
      saveSelection(selection.filter((k) => k !== key));
    } else {
      // Replace the last card rather than growing past 4.
      saveSelection([...selection.slice(0, 3), key]);
    }
  };

  return (
    <div>
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        {selection.map((key) => {
          const def = KPI_DEFS.find((d) => d.key === key)!;
          const entry = metrics.comparison[key];
          const value = entry ? def.format(entry.current, metrics) : "—";
          const hint = entry ? def.hint?.(metrics) : undefined;
          return (
            <KpiCard
              key={key}
              title={def.title}
              value={value}
              hint={hint}
              delta={entry?.mom}
              yoy={entry?.yoy}
              color={def.color}
            />
          );
        })}
      </div>
      <div className="mt-3">
        <button
          onClick={() => setEditing((e) => !e)}
          className="text-xs text-secondary underline-offset-2 hover:text-accent hover:underline"
        >
          {editing ? "完成" : "自定义卡片"}
        </button>
        {editing && (
          <div className="mt-2 flex flex-wrap gap-2">
            {KPI_DEFS.map((def) => {
              const active = selection.includes(def.key);
              return (
                <button
                  key={def.key}
                  onClick={() => toggle(def.key)}
                  className={`rounded-lg border px-2.5 py-1 text-xs transition-colors ${
                    active
                      ? "border-accent bg-accent/20 text-accent"
                      : "border-[var(--border-prominent)] text-secondary hover:bg-tertiary"
                  }`}
                >
                  {active ? <Check className="inline h-3 w-3 mr-1" /> : null}
                  {def.title}
                </button>
              );
            })}
            <button
              onClick={() => saveSelection(KPI_DEFAULT)}
              className="rounded-lg px-2.5 py-1 text-xs text-secondary hover:text-primary"
            >
              重置
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

export default function Usage() {
  const sources = useCollectionStore((state) => state.sources);
  const collecting = useCollectionStore((state) => state.collecting);
  const loadStatus = useCollectionStore((state) => state.loadStatus);
  const startCollect = useCollectionStore((state) => state.startCollect);

  const [usages, setUsages] = useState<AgentUsageSummary[]>([]);
  const [days, setDays] = useState<number>(7);

  // PRD-08: window-metrics state for the deep-analysis panel.
  const [winKind, setWinKind] = useState<WindowKind>("week");
  const [winRef, setWinRef] = useState<string>(todayIso());
  const [winMetrics, setWinMetrics] = useState<WindowMetrics | null>(null);
  const [winLoading, setWinLoading] = useState(false);
  // PRD-08 P1: semantic classification + daily summary state.
  const [classifying, setClassifying] = useState(false);
  const [classifyMsg, setClassifyMsg] = useState<string | null>(null);

  const loadUsage = useCallback(async (srcs: typeof sources, range: number) => {
    if (srcs.length === 0) {
      setUsages([]);
      return;
    }
    try {
      const results = await Promise.all(
        srcs.map((s) =>
          invoke<AgentUsageSummary>("get_agent_usage", { source: s.source, days: range }).catch(
            () => null,
          ),
        ),
      );
      setUsages(results.filter((r): r is AgentUsageSummary => r !== null));
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`读取使用数据失败：${msg}`);
    }
  }, []);

  const loadWindowMetrics = useCallback(async (kind: WindowKind, ref: string) => {
    setWinLoading(true);
    try {
      const m = await invoke<WindowMetrics>("get_window_metrics", {
        kind,
        refDate: ref,
        source: null,
      });
      setWinMetrics(m);
    } catch (err) {
      showError(err, { context: "分析数据加载" });
      setWinMetrics(null);
    } finally {
      setWinLoading(false);
    }
  }, []);

  useEffect(() => {
    (async () => {
      const s = await loadStatus();
      await loadUsage(s, days);
    })();
  }, [loadStatus, loadUsage, days]);

  // PRD-08: recompute deep analysis whenever the window selection changes.
  useEffect(() => {
    loadWindowMetrics(winKind, winRef);
  }, [winKind, winRef, loadWindowMetrics]);

  const handleCollect = async () => {
    await startCollect();
    // The job runs in the background; store listeners will refresh status
    // when it completes. We also poll briefly for the in-progress state.
    const poll = setInterval(async () => {
      const s = await loadStatus(true);
      if (!useCollectionStore.getState().collecting) {
        clearInterval(poll);
        await loadUsage(s, days);
        await loadWindowMetrics(winKind, winRef);
      }
    }, 1000);
  };

  // PRD-08 P1: run the four-axis semantic classifier over unclassified prompts.
  const handleClassify = async () => {
    if (classifying) return;
    setClassifying(true);
    setClassifyMsg(null);
    try {
      const r = await invoke<ClassifyResult>("classify_prompts", { source: null });
      if (r.skipped_no_key) {
        setClassifyMsg(`未配置 AI，跳过分类（待分类 ${r.eligible} 条）。请在偏好设置配置 AI。`);
      } else if (r.classified === 0) {
        setClassifyMsg("没有待分类的 Prompt（已全部分类）。");
      } else {
        setClassifyMsg(`分类完成：${r.classified} / ${r.eligible} 条已标注。`);
        await loadWindowMetrics(winKind, winRef); // refresh distributions
      }
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`分类失败：${msg}`);
    } finally {
      setClassifying(false);
    }
  };

  const hasData = sources.some((s) => s.status === "ok");

  // PRD-08: dimensions used by the detail sections below.
  const td = winMetrics?.token_dimension;
  const pd = winMetrics?.prompt_dimension;
  const lv = winMetrics?.leverage;

  return (
    <div className="h-full overflow-auto p-8">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-2xl font-bold">使用洞察</h1>
        <Button
          variant="primary"
          size="sm"
          onClick={handleCollect}
          loading={collecting}
          disabled={collecting}
        >
          <RefreshCw className={`h-4 w-4 ${collecting ? "animate-spin" : ""}`} />
          {collecting ? "采集中…" : "立即采集"}
        </Button>
      </div>

      {/* Empty state */}
      {!hasData ? (
        <section className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary p-10 text-center">
          <div className="mb-2 text-lg font-medium text-primary">未发现使用数据</div>
          <p className="mb-4 text-sm text-tertiary">
            先用 Claude Code / Codex / ZCode 进行几次对话，再回来点击「立即采集」，即可看到 Token、Prompt、成本与协作画像。
          </p>
          <Button
            variant="primary"
            size="md"
            onClick={handleCollect}
            loading={collecting}
            disabled={collecting}
          >
            {collecting ? "采集中…" : "立即采集"}
          </Button>
        </section>
      ) : (
        <>
          {/* PRD-08 §3.1: window selector + 4 KPI cards with 环比/同比 */}
          <section className="mb-6 rounded-xl border border-[var(--border-subtle)] bg-secondary p-5">
            <div className="mb-4 flex flex-wrap items-center gap-2 text-sm">
              <span className="text-secondary">窗口：</span>
              {(["day", "week", "month"] as WindowKind[]).map((k) => (
                <button
                  key={k}
                  onClick={() => setWinKind(k)}
                  className={`rounded-lg px-3 py-1 transition-colors ${
                    winKind === k
                      ? "bg-accent font-medium text-primary"
                      : "bg-tertiary text-primary hover:bg-tertiary"
                  }`}
                >
                  {k === "day" ? "日" : k === "week" ? "周" : "月"}
                </button>
              ))}
              <input
                type="date"
                value={winRef}
                onChange={(e) => setWinRef(e.target.value || todayIso())}
                className="ml-2 rounded-lg border border-[var(--border-prominent)] bg-secondary px-2 py-1 text-primary"
              />
              {winMetrics && (
                <span className="ml-auto text-xs text-tertiary">
                  {winMetrics.current_window.start} ~ {winMetrics.current_window.end}
                </span>
              )}
            </div>

            {winLoading ? (
              <div className="py-8 text-center text-sm text-secondary">分析中…</div>
            ) : winMetrics ? (
              <>
                <KpiGrid metrics={winMetrics} />
                <KpiConclusion />
              </>
            ) : (
              <div className="py-6 text-center text-sm text-tertiary">
                该窗口暂无数据，试试切换窗口或重新采集。
              </div>
            )}
          </section>

          {/* PRD-08 §3.2: Token dimension (cost profile) */}
          {td && td.scale && (
            <section className="mb-6 rounded-xl border border-[var(--border-subtle)] bg-secondary p-5">
              <h2 className="mb-3 text-lg font-semibold">Token 维度 · 成本画像</h2>
              <div className="grid grid-cols-2 gap-x-8 gap-y-2 text-sm md:grid-cols-3">
                <KV label="总 Token" value={formatTokens(td.scale.total_tokens)} />
                <KV label="Fresh Token" value={formatTokens(td.scale.fresh_tokens)} />
                <KV label="输入 / 输出" value={`${formatTokens(td.scale.input_tokens)} / ${formatTokens(td.scale.output_tokens)}`} />
                <KV label="缓存读取" value={formatTokens(td.scale.cache_read_tokens)} />
                <KV label="缓存命中率" value={`${(td.diagnostics.cache_ratio * 100).toFixed(1)}%`} />
                <KV label="模型调用" value={String(td.scale.model_calls)} />
                <KV label="工具调用" value={String(td.scale.tool_calls)} />
                <KV label="估算成本" value={formatCny(td.cost.est_cost_cny)} />
                <KV label="计费构成" value={formatBillingMix(td.cost.billing_mix)} />
              </div>
              {td.scale.total_tokens > 0 && (
                <div className="mt-4">
                  <div className="mb-1 text-xs text-secondary">按平台分布</div>
                  <DistributionBars data={td.distribution.by_platform} total={td.scale.total_tokens} colors={SEGMENT_COLORS} />
                </div>
              )}
              {td.diagnostics.heavy_sessions.length > 0 && (
                <div className="mt-3 text-xs text-tertiary">
                  ⚠ {td.diagnostics.heavy_sessions.length} 个高消耗会话（Top 10%，疑似上下文膨胀）
                </div>
              )}
              {/* Leverage bridge */}
              {lv && (
                <div className="mt-4 grid grid-cols-2 gap-x-8 gap-y-2 border-t border-[var(--border-subtle)] pt-3 text-sm md:grid-cols-3">
                  <KV label="成本杠杆" value={`${lv.leverage_per_cny.toFixed(1)} 产出/元`} hint="分母仅按量成本" />
                  <KV label="变动成本 / 订阅" value={`${formatCny(lv.variable_cost_cny)} / ${formatCny(lv.subscription_cost_cny)}`} />
                  <KV label="每 Prompt 成本" value={formatCny(lv.cost_per_prompt_cny)} />
                </div>
              )}
            </section>
          )}

          {/* PRD-08 §3.3: Prompt semantic dimension (collaboration profile) */}
          {pd && (
            <section className="mb-6 rounded-xl border border-[var(--border-subtle)] bg-secondary p-5">
              <h2 className="mb-3 flex items-center gap-1.5 text-lg font-semibold">
                Prompt 语义 · 协作画像
                {/* SPEC-F6 T2: 就地解释「归因」。 */}
                <HelpTip
                  ariaLabel="什么是归因"
                  text="根据会话中的关键词与项目绑定，推断某次使用与哪个 Skill 相关。"
                />
              </h2>
              <div className="mb-3 grid grid-cols-2 gap-x-8 gap-y-2 text-sm md:grid-cols-3">
                <KV label="Prompt 总数" value={String(pd.penetration.total_prompts)} />
                <KV label="分类率" value={`${(pd.semantics.classified_ratio * 100).toFixed(0)}%`} />
                {pd.maturity && (
                  <>
                    <KV label="Agent 系数" value={`${pd.maturity.agent_coefficient_min_per_prompt} min/prompt`} />
                    <KV label="工具调用/Prompt" value={pd.maturity.avg_tool_calls_per_prompt.toFixed(1)} />
                  </>
                )}
                <KV label="质量评分" value={`${pd.quality.score} / 100`} />
              </div>

              {pd.semantics.classified_ratio > 0 ? (
                <div className="grid gap-4 md:grid-cols-2">
                  <SemAxis title="请求动作" data={pd.semantics.requested_action} />
                  <SemAxis title="目标对象" data={pd.semantics.target_object} />
                  <SemAxis title="交互状态" data={pd.semantics.interaction_state} />
                  <SemAxis title="交互模式" data={pd.semantics.interaction_mode} />
                </div>
              ) : (
                <div className="rounded-lg border border-dashed border-[var(--border-subtle)] p-4 text-sm text-tertiary">
                  四维语义需 LLM 分类（P1）。当前可看 Prompt 数量与成熟度；配置 AI 后重新采集即可填充语义分布。
                </div>
              )}

              {pd.quality.improvement_suggestions.length > 0 && (
                <div className="mt-4 border-t border-[var(--border-subtle)] pt-3">
                  <div className="mb-2 text-xs font-medium text-secondary"><Lightbulb className="inline h-3.5 w-3.5 mr-1.5" />改进建议</div>
                  <ul className="list-inside list-disc space-y-1 text-sm text-primary">
                    {pd.quality.improvement_suggestions.map((s, i) => (
                      <li key={i}>{s}</li>
                    ))}
                  </ul>
                </div>
              )}

              {/* PRD-08 P1: trigger the LLM classifier for the four axes. */}
              <div className="mt-4 flex flex-wrap items-center gap-3 border-t border-[var(--border-subtle)] pt-3">
                <button
                  onClick={handleClassify}
                  disabled={classifying}
                  className="rounded-lg bg-accent/20 px-3 py-1.5 text-xs text-accent hover:bg-accent/30 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {classifying ? "分类中…" : "语义分类（LLM）"}
                </button>
                {classifyMsg && <span className="text-xs text-secondary">{classifyMsg}</span>}
              </div>
            </section>
          )}

          {/* PRD-08 §3.4 (P1): tool role profile (heatmap + agent coefficient). */}
          {winMetrics?.entity_metrics && (
            <ToolProfileSection em={winMetrics.entity_metrics} kind={winKind} refDate={winRef} />
          )}

          {/* PRD-08 §3.6 (P1): LLM daily summary. */}
          {winKind === "day" && <DailySummarySection date={winRef} />}

          {/* Legacy: range switch + per-source breakdown (kept for continuity) */}
          <div className="mb-4 flex items-center gap-2 text-sm">
            <span className="text-secondary">总量时间范围：</span>
            {DAYS_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setDays(opt.value)}
                className={`rounded-lg px-3 py-1 transition-colors ${
                  isPreset(days) && days === opt.value
                    ? "bg-accent font-medium text-primary"
                    : "bg-tertiary text-primary hover:bg-tertiary"
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>

          <section className="mb-6 rounded-xl border border-[var(--border-subtle)] bg-secondary">
            <div className="border-b border-[var(--border-subtle)] px-6 py-4 text-lg font-semibold">
              按数据源
            </div>
            <ul className="divide-y divide-[var(--divider)]">
              {usages
                .slice()
                .sort((a, b) => b.total_tokens - a.total_tokens)
                .map((u) => (
                  <li key={u.source} className="flex items-center justify-between px-6 py-4">
                    <div>
                      <div className="font-medium">{u.source}</div>
                      <div className="mt-1 text-xs text-tertiary">
                        会话 {u.session_count} · Prompt {u.prompt_count} · 输出 {formatTokens(u.output_tokens)}
                      </div>
                    </div>
                    <div className="text-right">
                      <div className="text-lg font-semibold text-accent">
                        {formatTokens(u.total_tokens)}
                      </div>
                      <div className="text-xs text-tertiary">tokens</div>
                    </div>
                  </li>
                ))}
            </ul>
          </section>

          {/* High-value prompts → skill generation (PRD-02 FR-4) */}
          <HighValuePrompts />
        </>
      )}

      {/* Data source status */}
      <section className="mt-6">
        <DataCollectionPanel compact title="采集状态" showPath={false} />
      </section>
    </div>
  );
}

/** A KPI card with a big number + a 环比 (mom) and 同比 (yoy) line. */
function KpiCard({
  title,
  value,
  hint,
  delta,
  yoy,
  color,
}: {
  title: string;
  value: string;
  hint?: string;
  delta?: Delta | null;
  yoy?: Delta | null;
  color: "accent" | "success" | "warning" | "info";
}) {
  const colorClasses: Record<string, string> = {
    accent: "border-accent/30 bg-accent/10 text-accent",
    success: "border-green-500/30 bg-green-500/10 text-green-500",
    warning: "border-amber-500/30 bg-amber-500/10 text-amber-500",
    info: "border-sky-500/30 bg-sky-500/10 text-sky-500",
  };
  return (
    <div className={`rounded-xl border p-5 ${colorClasses[color]}`}>
      <div className="text-sm opacity-80">{title}</div>
      <div className="mt-2 text-2xl font-bold">{value}</div>
      <div className="mt-1 flex items-center gap-3 text-xs opacity-80">
        <DeltaPill delta={delta} />
        {yoy ? <span className="text-tertiary">同比 {yoy.pct === null || yoy.pct === undefined ? "—" : `${yoy.pct >= 0 ? "↑" : "↓"}${Math.abs(yoy.pct * 100).toFixed(1)}%`}</span> : null}
      </div>
      {hint && <div className="mt-1 text-xs opacity-60">{hint}</div>}
    </div>
  );
}

function KV({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <span className="text-secondary">{label}</span>
      <span className="font-medium text-primary" title={hint}>
        {value}
      </span>
    </div>
  );
}

/** SPEC-F2 T7: actionable conclusion below the KPI grid. */
function KpiConclusion() {
  const [topPrompt, setTopPrompt] = useState<HighValuePrompt | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<HighValuePrompt[]>("get_high_value_prompts", { minRepeat: 3 })
      .then((p) => {
        if (!cancelled) setTopPrompt(p[0] ?? null);
      })
      .catch(() => {
        if (!cancelled) setTopPrompt(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!topPrompt) return null;

  const snippet = topPrompt.prompt_text.slice(0, 30);
  return (
    <div className="mt-4 rounded-lg border border-warning/20 bg-warning/5 px-4 py-3 text-sm">
      <Lightbulb className="mr-1.5 inline h-4 w-4 text-warning" />
      <span className="text-secondary">
        「{snippet}…」类 Prompt 重复了 {topPrompt.repeat_count} 次，
      </span>
      <button
        onClick={() => document.getElementById("high-value-prompts")?.scrollIntoView({ behavior: "smooth" })}
        className="font-medium text-warning hover:underline"
      >
        建议沉淀为 Skill
      </button>
    </div>
  );
}

/** One of the four semantic axes, with a distribution bar. */
function SemAxis({ title, data }: { title: string; data: Record<string, number> }) {
  const total = Object.values(data).reduce((a, b) => a + b, 0);
  return (
    <div>
      <div className="mb-1.5 text-xs font-medium text-secondary">{title}</div>
      <DistributionBars data={data} total={total} colors={SEGMENT_COLORS} />
    </div>
  );
}

/** PRD-08 §3.4 (P1/P3): the tool role-profile view — a tool×action heatmap (★ on
 * the dominant action per tool), plus per-tool agent coefficient. P3: each cell
 * is clickable → a drawer lists the classified prompts backing that cell. */
function ToolProfileSection({
  em,
  kind,
  refDate,
}: {
  em: EntityMetrics;
  kind: WindowKind;
  refDate: string;
}) {
  const { role_profile, agent_coefficient_by_tool, cost_split } = em;
  const tools = Object.keys(role_profile.tools);
  // Collect the full action vocabulary across tools for column headers.
  const actions = Array.from(
    new Set(tools.flatMap((t) => Object.keys(role_profile.tools[t]))),
  ).sort();

  const hasRoleData = tools.length >= 1 && actions.length > 0;
  const hasMultiTool = tools.length >= 2;

  // P3 drill-down state: {tool, action} of the clicked cell, + loaded prompts.
  const [drill, setDrill] = useState<{ tool: string; action: string } | null>(null);
  const [drillPrompts, setDrillPrompts] = useState<CellPrompt[] | null>(null);
  const [drillLoading, setDrillLoading] = useState(false);

  const openDrill = useCallback(
    async (tool: string, action: string) => {
      setDrill({ tool, action });
      setDrillPrompts(null);
      setDrillLoading(true);
      try {
        const rows = await invoke<CellPrompt[]>("get_prompts_for_cell", {
          kind,
          refDate,
          source: tool,
          action: action || null,
        });
        setDrillPrompts(rows);
      } catch {
        setDrillPrompts([]);
      } finally {
        setDrillLoading(false);
      }
    },
    [kind, refDate],
  );

  return (
    <section className="mb-6 rounded-xl border border-[var(--border-subtle)] bg-secondary p-5">
      <h2 className="mb-3 text-lg font-semibold">工具画像 · 角色分工</h2>

      {!hasRoleData ? (
        <div className="rounded-lg border border-dashed border-[var(--border-subtle)] p-4 text-sm text-tertiary">
          角色画像需要已分类的 Prompt 数据。请先在「偏好设置」配置 AI，再回到此页点击「语义分类」。
        </div>
      ) : (
        <>
          {role_profile.degraded && (
            <div className="mb-2 text-xs text-amber-400">
              注：高置信度（≥0.6）样本不足，已回退展示全部标签，结论仅供参考。
            </div>
          )}
          {!hasMultiTool && (
            <div className="mb-2 text-xs text-tertiary">
              当前仅 {tools.length} 个 Agent（{tools.join("、")}），跨工具对比建议采集更多 Agent。
            </div>
          )}
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b border-[var(--border-subtle)] text-secondary">
                  <th className="py-2 pr-3 text-left font-medium">Agent</th>
                  {actions.map((a) => (
                    <th key={a} className="px-3 py-2 text-right font-medium">
                      {a}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {tools.map((tool) => {
                  const row = role_profile.tools[tool];
                  return (
                    <tr key={tool} className="border-b border-[var(--border-subtle)]/60">
                      <td className="py-2 pr-3 font-medium text-primary">{tool}</td>
                      {actions.map((a) => {
                        const share = row[a] ?? 0;
                        const dominant = share >= 0.5;
                        const opacity = Math.max(0.15, share);
                        return (
                          <td
                            key={a}
                            className="px-1 py-1 text-right"
                            style={{
                              backgroundColor: dominant
                                ? `rgba(56, 189, 248, ${opacity})`
                                : share > 0
                                  ? `rgba(100, 116, 139, ${opacity * 0.5})`
                                  : "transparent",
                            }}
                          >
                            {share > 0 ? (
                              <button
                                onClick={() => openDrill(tool, a)}
                                title={`查看 ${tool} · ${a} 的 Prompt`}
                                className="rounded px-2 py-1 hover:bg-white/10"
                              >
                                <span className={dominant ? "font-bold text-sky-300" : "text-primary"}>
                                  {(share * 100).toFixed(0)}%{dominant ? " ★" : ""}
                                </span>
                              </button>
                            ) : (
                              <span className="px-2 text-tertiary">—</span>
                            )}
                          </td>
                        );
                      })}
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          {hasRoleData && (
            <div className="mt-1 text-xs text-tertiary">点击任意单元格查看对应的 Prompt 明细。</div>
          )}
        </>
      )}

      {/* Agent coefficient (autonomy) — only tools with trusted duration. */}
      <div className="mt-4 border-t border-[var(--border-subtle)] pt-3">
        <div className="mb-1 text-xs font-medium text-secondary">Agent 系数（自主时长，仅可信 duration 源）</div>
        {Object.keys(agent_coefficient_by_tool.tools).length === 0 ? (
          <div className="text-xs text-tertiary">
            暂无可信 turn-duration 数据（当前仅 ZCode 落盘该信号），其它工具显示「数据不足」而非 0。
          </div>
        ) : (
          <ul className="space-y-1 text-sm">
            {Object.entries(agent_coefficient_by_tool.tools).map(([tool, min]) => (
              <li key={tool} className="flex justify-between">
                <span className="text-primary">{tool}</span>
                <span className="font-medium text-primary">{min.toFixed(1)} min/prompt <Check className="inline h-3 w-3" /></span>
              </li>
            ))}
            {tools
              .filter((t) => !(t in agent_coefficient_by_tool.tools))
              .map((t) => (
                <li key={t} className="flex justify-between text-tertiary">
                  <span>{t}</span>
                  <span>数据不足 <X className="inline h-3 w-3" /></span>
                </li>
              ))}
          </ul>
        )}
      </div>

      {/* Cost split by billing mode. */}
      <div className="mt-4 border-t border-[var(--border-subtle)] pt-3">
        <div className="mb-1 text-xs font-medium text-secondary">成本拆分（按计费模式）</div>
        <div className="flex items-center gap-4 text-sm">
          <span className="text-primary">
            按量 <span className="font-semibold text-accent">¥{cost_split.variable_cny.toFixed(2)}</span>
            <span className="ml-1 text-xs text-tertiary">({cost_split.variable_calls} calls)</span>
          </span>
          <span className="text-primary">
            订阅 <span className="font-semibold text-primary">{cost_split.subscription_calls} calls</span>
            <span className="ml-1 text-xs text-tertiary">强度 {cost_split.subscription_intensity}</span>
          </span>
        </div>
      </div>

      {/* P3 drill-down drawer */}
      {drill && (
        <DrillDrawer
          tool={drill.tool}
          action={drill.action}
          prompts={drillPrompts}
          loading={drillLoading}
          onClose={() => setDrill(null)}
        />
      )}
    </section>
  );
}

/** PRD-08 §3.4 (P3): a right-side drawer listing the prompts behind a heatmap cell. */
function DrillDrawer({
  tool,
  action,
  prompts,
  loading,
  onClose,
}: {
  tool: string;
  action: string;
  prompts: CellPrompt[] | null;
  loading: boolean;
  onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      {/* backdrop */}
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />
      {/* panel */}
      <div className="relative h-full w-full max-w-md overflow-auto border-l border-[var(--border-subtle)] bg-secondary p-5 shadow-2xl">
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-lg font-semibold">
            {tool} · {action}
          </h3>
          <button
            onClick={onClose}
            className="rounded-lg px-2 py-1 text-sm text-secondary hover:bg-tertiary hover:text-white"
          >
            ✕
          </button>
        </div>
        <div className="mb-3 text-xs text-tertiary">
          该单元格对应的已分类 Prompt（最多 200 条）。
        </div>
        {loading ? (
          <SkeletonList count={5} />
        ) : !prompts || prompts.length === 0 ? (
          <div className="text-sm text-tertiary">无匹配的 Prompt。</div>
        ) : (
          <ul className="space-y-3">
            {prompts.map((p, i) => (
              <li key={i} className="rounded-lg border border-[var(--border-subtle)]/60 p-3">
                <div className="text-sm text-primary">{p.prompt_text}</div>
                <div className="mt-1 flex flex-wrap gap-2 text-xs text-tertiary">
                  {p.target_object && <span>对象 {p.target_object}</span>}
                  {p.interaction_state && <span>状态 {p.interaction_state}</span>}
                  {p.confidence != null && <span>置信 {(p.confidence * 100).toFixed(0)}%</span>}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}


/** PRD-08 §3.6 (P1): the LLM daily-summary panel. Loads a cached summary on
 * open; offers (re)generation via the configured AI. */
function DailySummarySection({ date }: { date: string }) {
  const [summary, setSummary] = useState<DailySummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const s = await invoke<DailySummary | null>("get_daily_summary", { date });
      setSummary(s);
    } catch {
      setSummary(null);
    } finally {
      setLoading(false);
    }
  }, [date]);

  useEffect(() => {
    load();
    setMsg(null);
  }, [load]);

  const handleGenerate = async () => {
    if (generating) return;
    setGenerating(true);
    setMsg(null);
    try {
      const outcome = await invoke<SummaryOutcome>("generate_daily_summary", { date });
      if (outcome.status === "generated") {
        setSummary(outcome.summary);
        setMsg(null);
      } else if (outcome.status === "no_key") {
        setMsg("未配置 AI，请在「偏好设置」配置后再生成。");
      } else if (outcome.status === "no_sessions") {
        setMsg("当日无会话，无需生成摘要。");
      } else {
        setMsg(outcome.reason);
      }
    } catch (err) {
      const m = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      setMsg(`生成失败：${m}`);
    } finally {
      setGenerating(false);
    }
  };

  return (
    <section className="mb-6 rounded-xl border border-[var(--border-subtle)] bg-secondary p-5">
      <div className="mb-3 flex items-center justify-between">
        <h2 className="text-lg font-semibold"><FileText className="inline h-4 w-4 mr-1.5" />每日摘要（{date}）</h2>
        <button
          onClick={handleGenerate}
          disabled={generating}
          className="rounded-lg bg-accent/20 px-3 py-1.5 text-xs text-accent hover:bg-accent/30 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {generating ? "生成中…" : summary ? "重新生成" : "生成摘要"}
        </button>
      </div>

      {loading ? (
        <div className="space-y-3">
          <Skeleton className="h-4 w-1/3" />
          <Skeleton className="h-4 w-2/3" />
          <Skeleton className="h-4 w-1/2" />
        </div>
      ) : summary ? (
        <div>
          {summary.highlights.length > 0 && (
            <div className="mb-3 text-sm text-primary">
              {summary.highlights.map((h, i) => (
                <p key={i}>• {h}</p>
              ))}
            </div>
          )}
          <ul className="space-y-3">
            {summary.activities.map((a, i) => (
              <li key={i} className="rounded-lg border border-[var(--border-subtle)]/60 p-3">
                <div className="flex flex-wrap items-center gap-2 text-xs text-secondary">
                  <span className="font-medium text-primary">{a.time_range || "—"}</span>
                  <span>·</span>
                  <span>{a.project || "-"}</span>
                  <span>·</span>
                  <span>{a.category}</span>
                </div>
                <div className="mt-1 text-sm text-primary">{a.summary}</div>
                {a.details.length > 0 && (
                  <ul className="mt-1 list-inside list-disc space-y-0.5 text-xs text-secondary">
                    {a.details.map((d, j) => (
                      <li key={j}>{d}</li>
                    ))}
                  </ul>
                )}
              </li>
            ))}
          </ul>
        </div>
      ) : msg ? (
        <div className="text-sm text-secondary">{msg}</div>
      ) : (
        <div className="rounded-lg border border-dashed border-[var(--border-subtle)] p-4 text-sm text-tertiary">
          当日还没有摘要。配置 AI 后点击「生成摘要」，AI 会把当天的 Agent 会话聚合成一份中文工作日报。
        </div>
      )}
    </section>
  );
}

function HighValuePrompts() {
  const navigateToSkill = useAppStore((state) => state.navigateToSkill);
  const [prompts, setPrompts] = useState<HighValuePrompt[]>([]);
  const [loading, setLoading] = useState(false);
  const [sediment, setSediment] = useState<HighValuePrompt | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const load = async () => {
    setLoading(true);
    try {
      const p = await invoke<HighValuePrompt[]>("get_high_value_prompts", { minRepeat: 3 });
      setPrompts(p);
      setExpanded(new Set());
    } catch {
      setPrompts([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  if (prompts.length === 0) return null;

  return (
    <>
      <section id="high-value-prompts" className="mt-6 rounded-xl border border-[var(--border-subtle)] bg-secondary">
        <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-6 py-4">
          <span className="text-lg font-semibold">高价值 Prompt（可沉淀为 Skill）</span>
          <button
            onClick={load}
            disabled={loading}
            className="rounded-lg bg-tertiary px-3 py-1 text-xs text-white hover:bg-tertiary"
          >
            {loading ? "刷新中…" : "刷新"}
          </button>
        </div>
        <ul className="divide-y divide-[var(--divider)]">
          {prompts.map((p, i) => {
            const isExpanded = expanded.has(i);
            const { display, truncated } = cleanPromptText(p.prompt_text, isExpanded ? Infinity : 160);
            return (
              <li key={i} className="flex items-start justify-between px-6 py-3 text-sm">
                <div className="min-w-0 flex-1 pr-4">
                  <div className={`text-primary ${isExpanded ? "" : "truncate"}`}>{display}</div>
                  {truncated && !isExpanded && (
                    <button
                      onClick={() => setExpanded((prev) => new Set([...prev, i]))}
                      className="mt-1 text-xs text-accent hover:underline"
                    >
                      展开
                    </button>
                  )}
                  {isExpanded && (
                    <button
                      onClick={() =>
                        setExpanded((prev) => {
                          const next = new Set(prev);
                          next.delete(i);
                          return next;
                        })
                      }
                      className="mt-1 text-xs text-accent hover:underline"
                    >
                      收起
                    </button>
                  )}
                  <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-tertiary">
                    <span className="rounded bg-accent/10 px-1.5 py-0.5 text-accent">{sourceLabel(p.source)}</span>
                    <span>重复 {p.repeat_count} 次</span>
                  </div>
                </div>
                <button
                  onClick={() => setSediment(p)}
                  className="shrink-0 rounded-lg bg-accent/20 px-3 py-1 text-xs text-accent hover:bg-accent/30"
                >
                  查看并沉淀
                </button>
              </li>
            );
          })}
        </ul>
      </section>

      {sediment && (
        <SedimentDialog
          prompt={sediment}
          onClose={() => setSediment(null)}
          onCreated={(skill) => navigateToSkill(skill.id)}
        />
      )}
    </>
  );
}

function SedimentDialog({
  prompt,
  onClose,
  onCreated,
}: {
  prompt: HighValuePrompt;
  onClose: () => void;
  onCreated: (skill: Skill) => void;
}) {
  // SPEC-F5 T3: initial name/description come from the backend preview command
  // (tokenized name + structured description), not from a naive client-side
  // char-filter that produced un-tokenized blobs like "Continuefromwhereyouleftoff".
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [previewLoading, setPreviewLoading] = useState(true);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setPreviewLoading(true);
    setPreviewError(null);
    invoke<{ name: string; description: string }>("preview_skill_from_prompt", {
      promptText: prompt.prompt_text,
    })
      .then((preview) => {
        if (cancelled) return;
        setName(preview.name);
        setDescription(preview.description);
        setPreviewLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        setPreviewError(msg);
        setPreviewLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [prompt.prompt_text]);

  const handleConfirm = async () => {
    if (!name.trim()) {
      showError("名称不能为空");
      return;
    }
    setCreating(true);
    try {
      // P3 review (Q3a): the skill is created straight into the global hub with
      // the user-edited name/description — no create-then-rewrite dance.
      const skill = await invoke<Skill>("generate_skill_from_prompt", {
        promptText: prompt.prompt_text,
        name: name.trim(),
        description: description.trim(),
      });
      showSuccess(`已创建 Skill「${name.trim()}」到全局 Hub`);
      onCreated(skill);
      onClose();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`创建失败：${msg}`);
    } finally {
      setCreating(false);
    }
  };

  const confirmDisabled = creating || previewLoading || previewError !== null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div
        className="max-h-[85vh] w-[520px] overflow-auto rounded-xl bg-primary p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="mb-4 text-lg font-bold">沉淀为 Skill</h3>

        <div className="mb-4 rounded-lg bg-secondary p-3 text-sm">
          <div className="mb-1 text-xs text-secondary">来源 Prompt</div>
          <div className="text-primary">{prompt.prompt_text}</div>
          <div className="mt-2 text-xs text-tertiary">近 7 天重复 {prompt.repeat_count} 次</div>
        </div>

        {previewError ? (
          <div className="mb-4 rounded-lg border border-danger/20 bg-danger/10 p-3 text-sm text-danger">
            {previewError}
          </div>
        ) : (
          <div className="mb-4 space-y-3">
            <label>
              <span className="mb-1 block text-xs font-medium text-secondary">建议名称</span>
              {previewLoading ? (
                <div className="h-9 w-full animate-pulse rounded-lg bg-secondary" />
              ) : (
                <input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  className="w-full rounded-lg border border-[var(--border-prominent)] bg-secondary px-3 py-2 text-sm text-primary focus:border-accent focus:outline-none"
                />
              )}
            </label>
            <label>
              <span className="mb-1 block text-xs font-medium text-secondary">建议描述</span>
              {previewLoading ? (
                <div className="h-[72px] w-full animate-pulse rounded-lg bg-secondary" />
              ) : (
                <textarea
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  rows={3}
                  className="w-full resize-none rounded-lg border border-[var(--border-prominent)] bg-secondary px-3 py-2 text-sm text-primary focus:border-accent focus:outline-none"
                />
              )}
            </label>
          </div>
        )}

        <div className="mb-5 rounded-lg border border-warning/20 bg-warning/10 p-3 text-xs text-warning">
          这将创建一个新的 Skill 文件并放入 Center Repo。确认后进入内置编辑器，不会立即同步到 Agent。
        </div>

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            disabled={creating}
            className="rounded border border-[var(--border-prominent)] px-4 py-1.5 text-sm text-primary hover:bg-secondary disabled:opacity-50"
          >
            取消
          </button>
          <button
            onClick={handleConfirm}
            disabled={confirmDisabled}
            title={previewError ?? undefined}
            className="rounded bg-accent px-4 py-1.5 text-sm font-medium text-primary disabled:opacity-50"
          >
            {creating ? "创建中…" : previewLoading ? "加载建议中…" : "在编辑器中确认并创建"}
          </button>
        </div>
      </div>
    </div>
  );
}
