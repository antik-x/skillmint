import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { useAppStore } from "../stores/appStore";
import { useCollectionStore } from "../stores/collectionStore";
import Today from "./Today";
import { invoke } from "@tauri-apps/api/core";
import type { DailySummary } from "../types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function yesterdayIso(): string {
  const d = new Date();
  d.setDate(d.getDate() - 1);
  return d.toISOString().slice(0, 10);
}

function resetStore() {
  useAppStore.setState({
    initialized: true,
    activeTab: "today",
    agents: [{ id: "a1", name: "claude-code", skill_directory: "/x", is_enabled: true, source: "claude-code" }],
    syncTargets: [],
    settings: {
      device_id: "",
      center_repo: "",
      default_sync_mode: "symlink",
      auto_sync_interval_minutes: 0,
      launch_at_login: false,
      show_dock_icon: true,
      onboarding_completed: true,
      skill_scope_mode: "global",
      auto_commit_after_import: false,
      project_skill_dir_name: ".skillmint/skills",
      remote_enabled: false,
      theme: "system",
      ai: { models: [], acp_connections: [], prefer_acp: false, strict_local_mode: false },
    },
  });
  useCollectionStore.setState({
    sources: [{ source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 }],
  });
}

describe("Today page", () => {
  beforeEach(() => {
    resetStore();
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation(() => Promise.resolve(undefined));
  });

  it("renders onboarding when no agents exist", async () => {
    useAppStore.setState({ agents: [] });
    vi.mocked(invoke).mockResolvedValue([]);
    render(<Today />);
    expect(await screen.findByText("扫描本机 Agent")).toBeInTheDocument();
  });

  it("renders onboarding when no collected sessions exist", async () => {
    useCollectionStore.setState({ sources: [{ source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 0 }] });
    vi.mocked(invoke).mockResolvedValue([]);
    render(<Today />);
    expect(await screen.findByText("采集使用数据")).toBeInTheDocument();
  });

  it("renders story card with daily summary", async () => {
    const summary: DailySummary = {
      date: yesterdayIso(),
      highlights: ["完成了 api-v2 分页修复"],
      activities: [],
    };
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_daily_summary") return Promise.resolve(summary);
      if (cmd === "get_collection_status") {
        return Promise.resolve([{ source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 }]);
      }
      if (cmd === "list_discoveries") return Promise.resolve([]);
      if (cmd === "get_skill_usage") return Promise.resolve([]);
      if (cmd === "list_daily_summaries") return Promise.resolve([]);
      if (cmd === "get_window_metrics") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    render(<Today />);
    expect(await screen.findByText(/完成了 api-v2 分页修复/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("深挖 → 洞察档案")).toBeInTheDocument());
  });

  it("renders rule-based narrative when no summary and no AI key", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_daily_summary") return Promise.resolve(null);
      if (cmd === "get_collection_status") {
        return Promise.resolve([{ source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 }]);
      }
      if (cmd === "list_discoveries") return Promise.resolve([]);
      if (cmd === "get_skill_usage") return Promise.resolve([]);
      if (cmd === "list_daily_summaries") return Promise.resolve([]);
      if (cmd === "get_window_metrics") {
        return Promise.resolve({
          kind: "day",
          ref_date: yesterdayIso(),
          current_window: { start: yesterdayIso(), end: yesterdayIso() },
          previous_window: { start: yesterdayIso(), end: yesterdayIso() },
          yoy_window: { start: yesterdayIso(), end: yesterdayIso() },
          has_previous_baseline: false,
          has_yoy_baseline: false,
          comparison: {},
          token_dimension: {
            scale: {
              total_tokens: 12000,
              fresh_tokens: 8000,
              input_tokens: 6000,
              output_tokens: 4000,
              reasoning_tokens: 0,
              cache_read_tokens: 0,
              cache_creation_tokens: 0,
              model_calls: 8,
              tool_calls: 0,
              duration_hours: 0,
            },
            distribution: { by_platform: {}, by_project: { "api-v2": 8000, "skillmint": 4000 }, by_model: {} },
            cost: { est_cost_cny: 0, by_platform_cny: {}, billing_mix: {} },
            diagnostics: { cache_ratio: 0, heavy_sessions: [] },
          },
          prompt_dimension: {
            penetration: { total_prompts: 0, by_platform: {}, by_project: {} },
            semantics: { classified_ratio: 0, requested_action: {}, target_object: {}, interaction_state: {}, interaction_mode: {} },
            quality: { score: 0, clarification_correction_rate: 0, planning_ratio: 0, test_object_ratio: 0, improvement_suggestions: [] },
          },
          leverage: { est_cost_cny: 0, variable_cost_cny: 0, subscription_cost_cny: 0, fresh_tokens: 0, output_proxy: 0, leverage_per_cny: 0, cost_per_prompt_cny: 0 },
        });
      }
      return Promise.resolve(undefined);
    });
    render(<Today />);
    await waitFor(() => expect(screen.getByText(/与 1 个 Agent 协作 8 次会话/)).toBeInTheDocument());
    expect(screen.getByText(/最活跃的项目是「api-v2」/)).toBeInTheDocument();
  });

  // Regression for issue #3: backend fills by_project with full absolute paths;
  // the narrative must show only the project name, not the whole path.
  it("shows only the project name when by_project key is an absolute path", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_daily_summary") return Promise.resolve(null);
      if (cmd === "get_collection_status") {
        return Promise.resolve([{ source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 }]);
      }
      if (cmd === "list_discoveries") return Promise.resolve([]);
      if (cmd === "get_skill_usage") return Promise.resolve([]);
      if (cmd === "list_daily_summaries") return Promise.resolve([]);
      if (cmd === "get_window_metrics") {
        return Promise.resolve({
          kind: "day",
          ref_date: yesterdayIso(),
          current_window: { start: yesterdayIso(), end: yesterdayIso() },
          previous_window: { start: yesterdayIso(), end: yesterdayIso() },
          yoy_window: { start: yesterdayIso(), end: yesterdayIso() },
          has_previous_baseline: false,
          has_yoy_baseline: false,
          comparison: {},
          token_dimension: {
            scale: {
              total_tokens: 9000,
              fresh_tokens: 6000,
              input_tokens: 5000,
              output_tokens: 4000,
              reasoning_tokens: 0,
              cache_read_tokens: 0,
              cache_creation_tokens: 0,
              model_calls: 5,
              tool_calls: 0,
              duration_hours: 0,
            },
            distribution: {
              by_platform: {},
              by_project: { "/Users/jiangjianyong/projects/03-OPC/tools/skill-hub": 6000, "/Users/jiangjianyong/projects/api-v2": 3000 },
              by_model: {},
            },
            cost: { est_cost_cny: 0, by_platform_cny: {}, billing_mix: {} },
            diagnostics: { cache_ratio: 0, heavy_sessions: [] },
          },
          prompt_dimension: {
            penetration: { total_prompts: 0, by_platform: {}, by_project: {} },
            semantics: { classified_ratio: 0, requested_action: {}, target_object: {}, interaction_state: {}, interaction_mode: {} },
            quality: { score: 0, clarification_correction_rate: 0, planning_ratio: 0, test_object_ratio: 0, improvement_suggestions: [] },
          },
          leverage: { est_cost_cny: 0, variable_cost_cny: 0, subscription_cost_cny: 0, fresh_tokens: 0, output_proxy: 0, leverage_per_cny: 0, cost_per_prompt_cny: 0 },
        });
      }
      return Promise.resolve(undefined);
    });
    render(<Today />);
    await waitFor(() => expect(screen.getByText(/与 1 个 Agent 协作 5 次会话/)).toBeInTheDocument());
    // Only the basename "skill-hub" should appear, not the absolute path.
    expect(screen.getByText(/最活跃的项目是「skill-hub」/)).toBeInTheDocument();
    expect(screen.queryByText(/最活跃的项目是「\/Users/)).not.toBeInTheDocument();
  });

  it("hides discovery strip when list_discoveries fails", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_daily_summary") return Promise.resolve(null);
      if (cmd === "list_discoveries") return Promise.reject(new Error("404: unknown command"));
      if (cmd === "get_skill_usage") return Promise.resolve([]);
      if (cmd === "list_daily_summaries") return Promise.resolve([]);
      if (cmd === "get_window_metrics") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    render(<Today />);
    await waitFor(() => expect(screen.queryByText("待你裁决")).not.toBeInTheDocument());
  });
});
