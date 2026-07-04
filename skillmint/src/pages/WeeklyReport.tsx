import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CalendarDays, ChevronLeft, ChevronRight, RefreshCw, Download } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import { showError, showInfo, showSuccess } from "../stores/toastStore";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { EmptyState } from "../components/ui/EmptyState";
import type { WeeklyReport } from "../types";

function mondayOf(iso: string): string {
  const d = new Date(iso + "T00:00:00Z");
  const day = d.getUTCDay();
  const offset = (day + 6) % 7;
  d.setUTCDate(d.getUTCDate() - offset);
  return d.toISOString().slice(0, 10);
}

function addDays(iso: string, days: number): string {
  const d = new Date(iso + "T00:00:00Z");
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

function formatRange(monday: string): string {
  const start = new Date(monday + "T00:00:00Z");
  const end = new Date(monday + "T00:00:00Z");
  end.setUTCDate(start.getUTCDate() + 6);
  const s = `${start.getUTCMonth() + 1}/${start.getUTCDate()}`;
  const e = `${end.getUTCMonth() + 1}/${end.getUTCDate()}`;
  return `${s} - ${e}`;
}

function reportToMarkdown(report: WeeklyReport): string {
  const { week_start, content } = report;
  const lines: string[] = [`# SkillMint 周报（${week_start}）`, ""];

  lines.push("## 项目进展");
  if (content.projects.length === 0) {
    lines.push("本周暂无项目数据。", "");
  } else {
    content.projects.forEach((p) => {
      lines.push(`### ${p.name}`, p.summary, "");
    });
  }

  lines.push("## 踩坑与浪费复盘");
  if (content.pitfalls.length === 0) {
    lines.push("本周暂无复盘记录。", "");
  } else {
    content.pitfalls.forEach((p) => {
      lines.push(`### ${p.title}`, p.lesson, "");
    });
  }

  lines.push("## 本周成长小结");
  const growth = content.growth ?? {};
  lines.push(`- 新增资产：${(growth.new_skills ?? []).length} 个`);
  lines.push(`- 消除的重复模式：${(growth.eliminated_patterns ?? []).length} 个`);
  lines.push(`- 裁决：沉淀 ${growth.accepted_count ?? 0} / 划掉 ${growth.dismissed_count ?? 0}`);
  lines.push("");

  return lines.join("\n");
}

export default function WeeklyReport() {
  const [week, setWeek] = useState<string>(() => mondayOf(new Date().toISOString().slice(0, 10)));
  const [report, setReport] = useState<WeeklyReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [generating, setGenerating] = useState(false);

  const loadReport = useCallback(async () => {
    setLoading(true);
    try {
      const r = await invoke<WeeklyReport | null>("get_weekly_report", { week });
      setReport(r);
    } catch (err) {
      showError(err, { context: "加载周报" });
      setReport(null);
    } finally {
      setLoading(false);
    }
  }, [week]);

  useEffect(() => {
    loadReport();
  }, [loadReport]);

  const handlePrevWeek = () => setWeek((w) => addDays(w, -7));
  const handleNextWeek = () => setWeek((w) => addDays(w, 7));
  const handleThisWeek = () => setWeek(mondayOf(new Date().toISOString().slice(0, 10)));

  const handleGenerate = async () => {
    if (generating) return;
    setGenerating(true);
    try {
      const r = await invoke<WeeklyReport>("generate_weekly_report", { week });
      setReport(r);
      showSuccess("周报已生成");
    } catch (err) {
      const raw = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      if (/已在运行|running|busy/i.test(raw)) {
        showInfo("周报正在生成中，请稍后再试");
      } else {
        showError(err, { context: "生成周报" });
      }
    } finally {
      setGenerating(false);
    }
  };

  const handleExport = async () => {
    if (!report) return;
    try {
      const path = await save({
        defaultPath: `SkillMint-周报-${report.week_start}.md`,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!path) return;
      await writeTextFile(path, reportToMarkdown(report));
      showSuccess("周报已导出");
    } catch (err) {
      showError(err, { context: "导出周报" });
    }
  };

  return (
    <div className="h-full overflow-auto p-8">
      <div className="mb-6 flex items-center justify-between">
        <div className="flex items-baseline gap-3">
          <h1 className="text-xl font-bold text-primary">周报</h1>
          <span className="text-xs text-tertiary font-mono" data-testid="weekly-date-range">
            {formatRange(week)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={handlePrevWeek}
            title="上一周"
          >
            <ChevronLeft className="h-4 w-4" />
          </Button>
          <Button variant="secondary" size="sm" onClick={handleThisWeek}>
            本周
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={handleNextWeek}
            title="下一周"
          >
            <ChevronRight className="h-4 w-4" />
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={handleGenerate}
            loading={generating}
            disabled={generating}
          >
            <RefreshCw className="h-4 w-4" />
            重新生成
          </Button>
          {report && (
            <Button variant="primary" size="sm" onClick={handleExport}>
              <Download className="h-4 w-4" />
              导出 Markdown
            </Button>
          )}
        </div>
      </div>

      {loading ? (
        <div className="flex h-64 items-center justify-center">
          <div className="h-5 w-5 animate-spin rounded-full border-2 border-accent border-t-transparent" />
        </div>
      ) : report ? (
        <div className="space-y-4">
          <Section title="项目进展叙事">
            {report.content.projects.length === 0 ? (
              <p className="text-sm text-secondary">本周暂无项目数据。</p>
            ) : (
              <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
                {report.content.projects.map((p) => (
                  <Card key={p.project_id} className="p-4">
                    <h3 className="mb-2 text-sm font-semibold text-primary">{p.name}</h3>
                    <p className="text-base leading-relaxed text-secondary">{p.summary}</p>
                  </Card>
                ))}
              </div>
            )}
          </Section>

          <Section title="踩坑与浪费复盘">
            {report.content.pitfalls.length === 0 ? (
              <p className="text-sm text-secondary">本周暂无复盘记录。</p>
            ) : (
              <div className="space-y-3">
                {report.content.pitfalls.map((p) => (
                  <Card key={p.session_id} className="p-4">
                    <h3 className="mb-1 text-sm font-semibold text-primary">{p.title}</h3>
                    <p className="text-sm leading-relaxed text-secondary">{p.lesson}</p>
                  </Card>
                ))}
              </div>
            )}
          </Section>

          <Section title="本周成长小结">
            <Card className="p-4">
              <GrowthSummary growth={report.content.growth} />
            </Card>
          </Section>
        </div>
      ) : (
        <div className="flex h-full flex-col items-center justify-center p-8">
          <EmptyState
            icon={CalendarDays}
            illustration="insights"
            title="本周报告将于周一早晨生成"
            description="第一期周报会在你完成首次数据收集后的周一出现。你也可以立即生成一份基于当前数据的预览。"
            action={
              <Button variant="primary" size="sm" onClick={handleGenerate} loading={generating}>
                <RefreshCw className="mr-1 h-4 w-4" />
                立即生成
              </Button>
            }
          />
        </div>
      )}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <h2 className="mb-3 text-sm font-semibold text-secondary">{title}</h2>
      {children}
    </section>
  );
}

function GrowthSummary({ growth }: { growth: WeeklyReport["content"]["growth"] }) {
  const g = growth ?? {};
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      <Metric label="新增资产" value={String((g.new_skills ?? []).length)} />
      <Metric label="消除的重复模式" value={String((g.eliminated_patterns ?? []).length)} />
      <Metric
        label="裁决"
        value={`${g.accepted_count ?? 0} / ${g.dismissed_count ?? 0}`}
        hint="沉淀 / 划掉"
      />
    </div>
  );
}

function Metric({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary p-4">
      <div className="text-xs text-secondary">{label}</div>
      <div className="mt-1 font-mono text-2xl font-semibold text-accent tabular-nums">{value}</div>
      {hint && <div className="mt-0.5 text-2xs text-tertiary">{hint}</div>}
    </div>
  );
}
