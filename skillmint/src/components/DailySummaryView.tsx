import { FileText, Lightbulb, Sparkles } from "lucide-react";
import type { DailySummary } from "../types";

interface DailySummaryViewProps {
  summary: DailySummary;
}

/** SPEC-F4 T7: derive a three-part narrative from the AI summary. */
function buildNarrative(summary: DailySummary) {
  const activities = summary.activities;
  const highlights = summary.highlights;

  // What happened today: a paragraph from the first activity summaries.
  const whatHappened =
    activities.length > 0
      ? activities
          .slice(0, 3)
          .map((a) => a.summary)
          .join("；") + "。"
      : highlights[0] ?? "今天没有记录到具体活动。";

  // Notable items: highlights beyond the first, or key details.
  const notable =
    highlights.length > 1
      ? highlights.slice(1)
      : highlights.length === 1 && activities.length > 0
        ? highlights
        : [];

  // Tomorrow suggestion: inferred from the last activity or generic.
  const lastProject = activities.length > 0 ? activities[activities.length - 1].project : null;
  const lastCategory = activities.length > 0 ? activities[activities.length - 1].category : null;
  let suggestion = "保持记录，明天继续与 Agent 协作。";
  if (lastProject && lastCategory) {
    suggestion = `明天可以继续推进「${lastProject}」的${lastCategory}工作，并留意是否需要沉淀为新 Skill。`;
  } else if (lastProject) {
    suggestion = `明天可以继续关注「${lastProject}」的进展。`;
  } else if (lastCategory) {
    suggestion = `明天可以继续关注${lastCategory}相关任务。`;
  }

  return { whatHappened, notable, suggestion };
}

export default function DailySummaryView({ summary }: DailySummaryViewProps) {
  const { whatHappened, notable, suggestion } = buildNarrative(summary);
  const hasContent =
    summary.highlights.length > 0 ||
    summary.activities.length > 0 ||
    whatHappened;

  return (
    <div className="space-y-5">
      {hasContent && (
        <section className="rounded-xl border border-[var(--border-subtle)] bg-secondary/50 p-4">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium text-accent">
            <FileText className="h-4 w-4" />
            今天发生了什么
          </div>
          <p className="text-sm leading-relaxed text-primary">{whatHappened}</p>
        </section>
      )}

      {notable.length > 0 && (
        <section className="rounded-xl border border-[var(--border-subtle)] bg-secondary/50 p-4">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium text-warning">
            <Sparkles className="h-4 w-4" />
            值得注意
          </div>
          <ul className="space-y-1.5 text-sm text-primary">
            {notable.map((h, i) => (
              <li key={i}>• {h}</li>
            ))}
          </ul>
        </section>
      )}

      {suggestion && (
        <section className="rounded-xl border border-[var(--border-subtle)] bg-secondary/50 p-4">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium text-success">
            <Lightbulb className="h-4 w-4" />
            明天建议
          </div>
          <p className="text-sm leading-relaxed text-primary">{suggestion}</p>
        </section>
      )}

      <section>
        <div className="mb-3 flex items-center gap-2 text-sm font-medium text-secondary">
          <FileText className="h-4 w-4" />
          活动明细
        </div>
        <ul className="space-y-3">
          {summary.activities.map((a, i) => (
            <li
              key={i}
              className="rounded-xl border border-[var(--border-subtle)] bg-secondary p-4"
            >
              <div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-secondary">
                {a.time_range ? (
                  <span className="font-medium text-primary">{a.time_range}</span>
                ) : null}
                {a.time_range && (a.project || a.category) ? (
                  <span>·</span>
                ) : null}
                {a.project ? <span>{a.project}</span> : null}
                {a.project && a.category ? <span>·</span> : null}
                {a.category ? (
                  <span className="rounded bg-accent/10 px-1.5 py-0.5 text-accent">
                    {a.category}
                  </span>
                ) : null}
              </div>
              <div className="text-sm font-medium text-primary">{a.summary}</div>
              {a.details.length > 0 && (
                <ul className="mt-2 list-inside list-disc space-y-1 text-xs text-secondary">
                  {a.details.map((d, j) => (
                    <li key={j}>{d}</li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ul>
      </section>

      {summary.highlights.length === 0 && summary.activities.length === 0 && (
        <div className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary/50 p-6 text-center text-sm text-tertiary">
          该摘要没有亮点和活动明细。
        </div>
      )}
    </div>
  );
}
