import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import WeeklyReport from "./WeeklyReport";
import type { WeeklyReport as WeeklyReportType } from "../types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-fs", () => ({
  writeTextFile: vi.fn(),
}));

const OriginalDate = globalThis.Date;
const FIXED_DATE = new OriginalDate("2026-06-30T00:00:00Z");

function mockFixedDate() {
  vi.spyOn(globalThis as any, "Date").mockImplementation(function (this: unknown, ...args: unknown[]) {
    if (args.length === 0) {
      return new OriginalDate(FIXED_DATE);
    }
    return new (OriginalDate as unknown as new (...args: unknown[]) => Date)(...args);
  } as any);
}

function makeReport(weekStart = "2026-06-29", overrides: Partial<WeeklyReportType> = {}): WeeklyReportType {
  return {
    week_start: weekStart,
    content: {
      projects: [
        { project_id: "p1", name: "skill-hub", summary: "完成了 M2 收件箱裁决流。" },
      ],
      pitfalls: [
        { session_id: "s1", project_id: "p1", title: "重复请求中文注释", lesson: "下次直接引用 Skill。" },
      ],
      growth: {
        new_skills: ["tauri-v2", "中文注释规范"],
        eliminated_patterns: ["重复注释请求"],
        accepted_count: 2,
        dismissed_count: 1,
      },
    },
    generated_at: 1750000000,
    model: null,
    provider: "skillmint",
    ...overrides,
  };
}

describe("WeeklyReport page", () => {
  beforeEach(() => {
    mockFixedDate();
    vi.mocked(invoke).mockReset();
    vi.mocked(save).mockReset();
    vi.mocked(writeTextFile).mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders report sections", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_weekly_report") return Promise.resolve(makeReport());
      return Promise.resolve(undefined);
    });
    render(<WeeklyReport />);
    expect(await screen.findByText("skill-hub")).toBeInTheDocument();
    expect(screen.getByText("完成了 M2 收件箱裁决流。")).toBeInTheDocument();
    expect(screen.getByText("重复请求中文注释")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
  });

  it("shows empty state and generates report", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_weekly_report") return Promise.resolve(null);
      if (cmd === "generate_weekly_report") return Promise.resolve(makeReport());
      return Promise.resolve(undefined);
    });
    render(<WeeklyReport />);
    const btn = await screen.findByText("立即生成");
    await userEvent.click(btn);
    expect(await screen.findByText("skill-hub")).toBeInTheDocument();
  });

  it("navigates weeks", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "get_weekly_report") {
        const week = (args as { week: string }).week;
        return Promise.resolve(makeReport(week, { content: { projects: [{ project_id: "p", name: week, summary: "" }], pitfalls: [], growth: {} } }));
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<WeeklyReport />);
    expect(await screen.findByTestId("weekly-date-range")).toHaveTextContent("6/29 - 7/5");
    const prev = screen.getByTitle("上一周");
    await userEvent.click(prev);
    await waitFor(() => expect(screen.getByTestId("weekly-date-range")).toHaveTextContent("6/22 - 6/28"));
  });

  it("exports markdown", async () => {
    const report = makeReport();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_weekly_report") return Promise.resolve(report);
      return Promise.resolve(undefined);
    });
    vi.mocked(save).mockResolvedValue("/tmp/report.md");
    vi.mocked(writeTextFile).mockResolvedValue(undefined);

    render(<WeeklyReport />);
    await screen.findByText("skill-hub");
    const exportBtn = screen.getByText("导出 Markdown");
    await userEvent.click(exportBtn);

    await waitFor(() => {
      expect(save).toHaveBeenCalled();
      expect(writeTextFile).toHaveBeenCalledWith("/tmp/report.md", expect.stringContaining("# SkillMint 周报"));
    });
  });
});
