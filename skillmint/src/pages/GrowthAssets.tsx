import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TrendingUp, ArrowLeft, ArrowRight, Archive } from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { showError } from "../stores/toastStore";
import { Card } from "../components/ui/Card";
import { EmptyState } from "../components/ui/EmptyState";
import { useHotkey, useHotkeyScope } from "../hooks/useHotkeys";
import type { ActiveSkill, CapabilityMapItem, CompoundingCurve, GrowthMetrics } from "../types";

function weekLabelToDate(label: string): Date | null {
  const m = label.match(/^(\d{4})-W(\d{2})$/);
  if (!m) return null;
  const year = parseInt(m[1], 10);
  const week = parseInt(m[2], 10);
  // ISO week: Monday of week W01 is the Monday of the week containing Jan 4.
  const jan4 = new Date(Date.UTC(year, 0, 4));
  const jan4Day = jan4.getUTCDay() || 7;
  const monday = new Date(jan4);
  monday.setUTCDate(jan4.getUTCDate() - jan4Day + 1 + (week - 1) * 7);
  return monday;
}

function formatMmDd(date: Date | null): string {
  if (!date) return "--";
  const mm = String(date.getMonth() + 1).padStart(2, "0");
  const dd = String(date.getDate()).padStart(2, "0");
  return `${mm}-${dd}`;
}

export function eliminationPercent(curve: CompoundingCurve): number {
  const precipIdx = Math.floor(curve.weeks.length / 2) - 1;
  const pre = curve.counts.slice(0, precipIdx + 1);
  const post = curve.counts.slice(precipIdx + 1);
  const preAvg = pre.reduce((a, b) => a + b, 0) / (pre.length || 1);
  const postAvg = post.reduce((a, b) => a + b, 0) / (post.length || 1);
  if (preAvg <= 0) return 0;
  return Math.max(0, Math.min(100, Math.round(((preAvg - postAvg) / preAvg) * 100)));
}

export default function GrowthAssets() {
  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const setDiscoverSearchTerm = useAppStore((state) => state.setDiscoverSearchTerm);

  const [metrics, setMetrics] = useState<GrowthMetrics | null>(null);
  const [loading, setLoading] = useState(true);
  const [curveIndex, setCurveIndex] = useState(0);

  useHotkeyScope("growth");

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    invoke<GrowthMetrics>("get_growth_metrics")
      .then((m) => {
        if (!cancelled) setMetrics(m);
      })
      .catch((err) => {
        if (!cancelled) {
          showError(err, { context: "加载成长资产" });
          setMetrics(null);
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useHotkey("ArrowLeft", () => {
    setCurveIndex((i) => Math.max(0, i - 1));
  }, { scope: "growth" });

  useHotkey("ArrowRight", () => {
    setCurveIndex((i) => Math.min((metrics?.compounding_curves.length ?? 1) - 1, i + 1));
  }, { scope: "growth", description: "下一条复利曲线" }, [metrics]);

  const goDiscover = useCallback(
    (term: string) => {
      setDiscoverSearchTerm(term);
      setActiveTab("discover");
    },
    [setActiveTab, setDiscoverSearchTerm]
  );

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center p-8">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-accent border-t-transparent" />
      </div>
    );
  }

  if (!metrics) {
    return (
      <div className="flex h-full flex-col items-center justify-center p-8">
        <EmptyState
          icon={TrendingUp}
          illustration="insights"
          title="成长资产"
          description="汇总你从发现沉淀的 Skill、手写资产和远程安装，用复利证据证明成长。"
          action={
            <ButtonLike onClick={() => setActiveTab("today")}>回今天页</ButtonLike>
          }
        />
      </div>
    );
  }

  const sourceCounts = metrics.skill_count_by_source || {};
  const fromDiscovery = sourceCounts.from_discovery ?? 0;
  const handwritten = sourceCounts.handwritten ?? 0;
  const remote = sourceCounts.remote ?? 0;
  const totalAssets = fromDiscovery + handwritten + remote || 1;

  return (
    <div className="h-full overflow-auto p-8">
      <div className="mb-6 flex items-baseline justify-between">
        <div>
          <h1 className="text-xl font-bold text-primary">成长资产</h1>
          <span className="text-xs text-tertiary font-mono">
            统计至 {new Date().toISOString().slice(0, 10)}
          </span>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {/* Asset total & sources */}
        <Card className="p-5">
          <div className="mb-4 flex items-baseline justify-between">
            <h3 className="text-sm font-semibold text-secondary">资产总量与来源</h3>
            <span className="text-2xs text-tertiary font-mono">{totalAssets} 个 Skill</span>
          </div>
          <div className="mb-3 font-mono text-2xl font-semibold text-primary tabular-nums">
            {totalAssets}{" "}
            <small className="ml-1 text-xs font-normal text-secondary">
              其中 {fromDiscovery} 个由发现沉淀而来
            </small>
          </div>
          <div className="mb-3 flex h-2.5 overflow-hidden rounded-full">
            <div
              className="bg-amber"
              style={{ width: `${(fromDiscovery / totalAssets) * 100}%` }}
            />
            <div
              className="bg-accent"
              style={{ width: `${(handwritten / totalAssets) * 100}%` }}
            />
            <div
              className="bg-tertiary"
              style={{ width: `${(remote / totalAssets) * 100}%` }}
            />
          </div>
          <div className="flex flex-wrap gap-4 text-xs text-secondary">
            <LegendDot color="bg-amber" label={`从发现沉淀 ${fromDiscovery}`} />
            <LegendDot color="bg-accent" label={`手写 ${handwritten}`} />
            <LegendDot color="bg-tertiary" label={`远程安装 ${remote}`} />
          </div>
        </Card>

        {/* Compounding evidence */}
        <Card className="p-5">
          <div className="mb-4 flex items-baseline justify-between">
            <h3 className="text-sm font-semibold text-secondary">
              {metrics.compounding_curves.length > 0
                ? `复利证据 · ${metrics.compounding_curves[curveIndex]?.skill_name}`
                : "复利证据"}
            </h3>
            {metrics.compounding_curves.length > 1 && (
              <span className="text-2xs text-tertiary font-mono">
                <ArrowLeft className="inline h-3 w-3" />{" "}
                {curveIndex + 1} / {metrics.compounding_curves.length}{" "}
                <ArrowRight className="inline h-3 w-3" />
              </span>
            )}
          </div>

          {metrics.compounding_curves.length === 0 ? (
            <div className="rounded-lg border border-dashed border-[var(--border-prominent)] bg-secondary/50 p-6 text-center text-sm text-secondary">
              第一个从发现沉淀的 Skill 会在这里长出复利曲线。
            </div>
          ) : (
            <CompoundingChart curve={metrics.compounding_curves[curveIndex]} />
          )}
        </Card>

        {/* Activity */}
        <Card className="p-5">
          <div className="mb-4 flex items-baseline justify-between">
            <h3 className="text-sm font-semibold text-secondary">资产活跃度</h3>
            <span className="text-2xs text-tertiary font-mono">近 30 天被引用</span>
          </div>
          <ActivityList skills={metrics.active_skills} />
          <DormantSkills skills={metrics.active_skills} />
        </Card>

        {/* Capability map */}
        <Card className="p-5">
          <div className="mb-4 flex items-baseline justify-between">
            <h3 className="text-sm font-semibold text-secondary">能力地图</h3>
            <span className="text-2xs text-tertiary font-mono">按领域覆盖</span>
          </div>
          <CapabilityGrid items={metrics.capability_map} onBlankClick={goDiscover} />
        </Card>
      </div>
    </div>
  );
}

function LegendDot({ color, label }: { color: string; label: string }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className={`h-2 w-2 rounded-sm ${color}`} />
      {label}
    </span>
  );
}

function CompoundingChart({ curve }: { curve: CompoundingCurve }) {
  const max = Math.max(1, ...curve.counts);
  const precipIdx = Math.floor(curve.weeks.length / 2) - 1;
  const preCounts = curve.counts.slice(0, precipIdx + 1);
  const postCounts = curve.counts.slice(precipIdx + 1);
  const precipLabel = formatMmDd(weekLabelToDate(curve.weeks[precipIdx] ?? ""));
  const percent = eliminationPercent(curve);

  return (
    <div>
      <div className="flex h-20 items-end gap-1">
        <div className="flex flex-1 items-end gap-1">
          {preCounts.map((c, i) => (
            <Bar key={`pre-${i}`} value={c} max={max} className="bg-[var(--border-prominent)]" />
          ))}
        </div>
        <div className="relative flex h-full flex-none flex-col items-center justify-end">
          <div className="h-full w-px bg-amber-glow" />
        </div>
        <div className="flex flex-1 items-end gap-1">
          {postCounts.map((c, i) => (
            <Bar key={`post-${i}`} value={c} max={max} className="bg-amber" />
          ))}
        </div>
      </div>
      <div className="mt-2 flex items-center justify-between font-mono text-2xs text-tertiary">
        <span>沉淀前 {preCounts.length} 周</span>
        <span className="text-amber">▲ {precipLabel} 沉淀</span>
        <span>之后 {postCounts.length} 周</span>
      </div>
      <p className="mt-3 text-sm leading-relaxed text-secondary">
        这条 Skill 替你消除了{" "}
        <b className="text-amber" data-testid="compounding-percent">
          {percent}%
        </b>{" "}
        的重复指令。
      </p>
    </div>
  );
}

function Bar({ value, max, className }: { value: number; max: number; className: string }) {
  const height = Math.max(4, Math.round((value / max) * 100));
  return (
    <div
      className={`flex-1 rounded-t ${className}`}
      style={{ height: `${height}%` }}
      title={String(value)}
    />
  );
}

function ActivityList({ skills }: { skills: ActiveSkill[] }) {
  const active = useMemo(
    () => skills.filter((s) => !s.dormant).sort((a, b) => b.usage_count - a.usage_count).slice(0, 8),
    [skills]
  );
  const max = Math.max(1, ...active.map((s) => s.usage_count));

  if (active.length === 0) {
    return <p className="text-sm text-secondary">近 30 天暂无活跃 Skill。</p>;
  }

  return (
    <ul className="space-y-2">
      {active.map((s, idx) => (
        <li key={s.skill_name} className="flex items-center gap-3 text-sm">
          <span className="w-28 truncate text-primary" title={s.skill_name}>
            {s.skill_name}
          </span>
          <div className="flex-1">
            <div
              className={`h-1.5 rounded ${idx === 0 ? "bg-amber" : "bg-accent"}`}
              style={{ width: `${(s.usage_count / max) * 100}%`, opacity: idx === 0 ? 1 : 0.75 }}
            />
          </div>
          <span className="w-8 text-right font-mono text-xs text-tertiary tabular-nums">
            {s.usage_count}
          </span>
        </li>
      ))}
    </ul>
  );
}

function DormantSkills({ skills }: { skills: ActiveSkill[] }) {
  const dormant = useMemo(() => skills.filter((s) => s.dormant).slice(0, 5), [skills]);
  const setActiveTab = useAppStore((state) => state.setActiveTab);

  if (dormant.length === 0) return null;

  return (
    <div className="mt-4 flex items-start gap-2 border-t border-[var(--border-subtle)] pt-3 text-xs text-tertiary">
      <Archive className="mt-0.5 h-3.5 w-3.5" />
      <span>
        睡眠资产：{dormant.map((s) => s.skill_name).join("、")} 等已 90 天未被引用 →{" "}
        <button
          onClick={() => setActiveTab("skillLibrary")}
          className="text-accent hover:underline"
        >
          归档或改进
        </button>
      </span>
    </div>
  );
}

function CapabilityGrid({
  items,
  onBlankClick,
}: {
  items: CapabilityMapItem[];
  onBlankClick: (term: string) => void;
}) {
  return (
    <div className="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-5">
      {items.map((item) => {
        const isActive = item.active ?? (item.has_skill && (item.prompt_count_7d ?? 0) > 0);
        const hasSkill = item.has_skill ?? false;
        return (
          <button
            key={item.category}
            onClick={() => !hasSkill && onBlankClick(item.category)}
            disabled={hasSkill}
            className={`rounded-lg border p-3 text-left transition-colors ${
              isActive
                ? "border-amber-glow bg-amber-dim"
                : hasSkill
                  ? "border-transparent bg-accent-dim"
                  : "border-[var(--border-subtle)] bg-primary hover:border-[var(--border-prominent)]"
            }`}
          >
            <div className={`text-xs ${isActive ? "text-primary" : "text-secondary"}`}>
              {item.category}
            </div>
            <div
              className={`mt-1 font-mono text-2xs ${
                isActive ? "text-amber" : hasSkill ? "text-accent" : "text-tertiary"
              }`}
            >
              {isActive
                ? `${item.prompt_count_7d ?? 0} · 活跃`
                : hasSkill
                  ? `${item.prompt_count_7d ?? 0}`
                  : "空白 →"}
            </div>
          </button>
        );
      })}
    </div>
  );
}

function ButtonLike({ children, onClick }: { children: React.ReactNode; onClick?: () => void }) {
  return (
    <button
      onClick={onClick}
      className="rounded-lg border border-[var(--border-subtle)] bg-tertiary px-3 py-1.5 text-xs font-medium text-primary transition-colors hover:bg-[var(--border-subtle)]"
    >
      {children}
    </button>
  );
}
