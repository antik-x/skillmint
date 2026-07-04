import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import DailySummaryView from "./DailySummaryView";
import type { DailySummary } from "../types";

function buildSummary(overrides: Partial<DailySummary> = {}): DailySummary {
  return {
    date: "2026-07-02",
    highlights: ["完成了 api-v2 分页修复", "优化了提示词缓存策略"],
    activities: [
      {
        time_range: "09:30-11:00",
        project: "api-v2",
        category: "重构",
        summary: "修复了分页参数越界问题",
        details: ["补全了边界单元测试"],
      },
      {
        time_range: "14:00-15:30",
        project: "skillmint",
        category: "优化",
        summary: "优化了提示词缓存命中率",
        details: [],
      },
    ],
    ...overrides,
  };
}

describe("DailySummaryView", () => {
  it("renders the three-part narrative for an AI summary", () => {
    render(<DailySummaryView summary={buildSummary()} />);

    expect(screen.getByText("今天发生了什么")).toBeInTheDocument();
    expect(screen.getByText(/修复了分页参数越界问题；优化了提示词缓存命中率/)).toBeInTheDocument();

    expect(screen.getByText("值得注意")).toBeInTheDocument();
    expect(screen.getByText(/• 优化了提示词缓存策略/)).toBeInTheDocument();

    expect(screen.getByText("明天建议")).toBeInTheDocument();
    expect(screen.getByText(/明天可以继续推进「skillmint」的优化工作/)).toBeInTheDocument();
  });

  it("falls back to highlights when there are no activities", () => {
    const summary = buildSummary({ activities: [] });
    render(<DailySummaryView summary={summary} />);
    expect(screen.getByText(/完成了 api-v2 分页修复/)).toBeInTheDocument();
  });

  it("shows an empty note when there is no content", () => {
    const summary = buildSummary({ highlights: [], activities: [] });
    render(<DailySummaryView summary={summary} />);
    expect(screen.getByText("该摘要没有亮点和活动明细。")).toBeInTheDocument();
  });
});
