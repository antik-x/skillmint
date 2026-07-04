import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import GrowthAssets, { eliminationPercent } from "./GrowthAssets";
import { resetHotkeys } from "../hooks/useHotkeys";
import { useAppStore } from "../stores/appStore";
import type { GrowthMetrics } from "../types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function makeMetrics(overrides: Partial<GrowthMetrics> = {}): GrowthMetrics {
  return {
    skill_count_by_source: { from_discovery: 3, handwritten: 5, remote: 2 },
    compounding_curves: [
      {
        skill_id: "s1",
        skill_name: "重复模式 A",
        weeks: ["2026-W20", "2026-W21", "2026-W22", "2026-W23", "2026-W24", "2026-W25"],
        counts: [10, 12, 8, 2, 1, 0],
      },
      {
        skill_id: "s2",
        skill_name: "重复模式 B",
        weeks: ["2026-W20", "2026-W21", "2026-W22", "2026-W23", "2026-W24", "2026-W25"],
        counts: [5, 5, 5, 5, 5, 5],
      },
    ],
    active_skills: [
      { skill_id: "s1", skill_name: "tauri-v2", usage_count: 34, dormant: false },
      { skill_id: "s2", skill_name: "中文注释规范", usage_count: 27, dormant: false },
      { skill_id: "s3", skill_name: "react-legacy", usage_count: 0, dormant: true },
    ],
    capability_map: [
      { category: "前端", has_skill: true, prompt_count_7d: 6 },
      { category: "数据库", has_skill: false, prompt_count_7d: 0 },
      { category: "测试", has_skill: true, prompt_count_7d: 0 },
    ],
    ...overrides,
  };
}

describe("eliminationPercent", () => {
  it("computes reduction percent", () => {
    const curve = makeMetrics().compounding_curves[0];
    expect(eliminationPercent(curve)).toBe(90);
  });
});

function resetStore() {
  useAppStore.setState({ activeTab: "growthAssets", discoverSearchTerm: null });
}

describe("GrowthAssets page", () => {
  beforeEach(() => {
    resetHotkeys();
    resetStore();
    vi.mocked(invoke).mockReset();
  });

  it("renders asset total and source breakdown", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_growth_metrics") return Promise.resolve(makeMetrics());
      return Promise.resolve(undefined);
    });
    render(<GrowthAssets />);
    expect(await screen.findByText("10")).toBeInTheDocument();
    expect(screen.getByText(/其中 3 个由发现沉淀而来/)).toBeInTheDocument();
    expect(screen.getByText("从发现沉淀 3")).toBeInTheDocument();
  });

  it("shows empty compounding state when no curves", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_growth_metrics") return Promise.resolve(makeMetrics({ compounding_curves: [] }));
      return Promise.resolve(undefined);
    });
    render(<GrowthAssets />);
    expect(
      await screen.findByText("第一个从发现沉淀的 Skill 会在这里长出复利曲线。")
    ).toBeInTheDocument();
  });

  it("renders compounding chart and percent", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_growth_metrics") return Promise.resolve(makeMetrics());
      return Promise.resolve(undefined);
    });
    render(<GrowthAssets />);
    expect(await screen.findByText(/重复模式 A/)).toBeInTheDocument();
    expect(screen.getByTestId("compounding-percent")).toHaveTextContent("90%");
  });

  it("switches curve with arrow keys", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_growth_metrics") return Promise.resolve(makeMetrics());
      return Promise.resolve(undefined);
    });
    render(<GrowthAssets />);
    await screen.findByText(/重复模式 A/);
    fireEvent.keyDown(document, { key: "ArrowRight", code: "ArrowRight" });
    expect(await screen.findByText(/重复模式 B/)).toBeInTheDocument();
  });

  it("renders activity and dormant skills", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_growth_metrics") return Promise.resolve(makeMetrics());
      return Promise.resolve(undefined);
    });
    render(<GrowthAssets />);
    expect(await screen.findByText("tauri-v2")).toBeInTheDocument();
    expect(screen.getByText(/react-legacy/)).toBeInTheDocument();
    expect(screen.getByText(/睡眠资产/)).toBeInTheDocument();
  });

  it("navigates to discover from blank capability tile", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_growth_metrics") return Promise.resolve(makeMetrics());
      return Promise.resolve(undefined);
    });
    render(<GrowthAssets />);
    await screen.findByText("数据库");
    const blankTile = screen.getByText("空白 →");
    await userEvent.click(blankTile);
    await waitFor(() => {
      expect(useAppStore.getState().activeTab).toBe("discover");
      expect(useAppStore.getState().discoverSearchTerm).toBe("数据库");
    });
  });
});
