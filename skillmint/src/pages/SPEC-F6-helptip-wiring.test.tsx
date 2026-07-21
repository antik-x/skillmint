import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../stores/appStore";
import { useCollectionStore } from "../stores/collectionStore";

// SPEC-F6 T2: 四处 HelpTip 接线的存在性断言。
// 分别在各自页/组件里查找 testid=help-tip-trigger，并验证其父级文案上下文。

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function resetAppStore(overrides: Partial<ReturnType<typeof useAppStore.getState>> = {}) {
  useAppStore.setState({
    initialized: true,
    activeTab: "today",
    agents: [],
    skills: [],
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
    ...overrides,
  } as never);
}

describe("SPEC-F6 T2: HelpTip 接线存在性", () => {
  beforeEach(() => {
    resetAppStore();
    useCollectionStore.setState({
      sources: [],
      collecting: false,
      currentJobId: null,
      lastJobStats: null,
      lastJobError: null,
    });
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_collection_status") return Promise.resolve([]);
      if (cmd === "get_window_metrics") return Promise.resolve(null);
      if (cmd === "get_high_value_prompts") return Promise.resolve([]);
      if (cmd === "get_agent_usage") return Promise.resolve(null);
      if (cmd === "detect_local_agents") return Promise.resolve([]);
      if (cmd === "save_settings") return Promise.resolve(useAppStore.getState().settings);
      return Promise.resolve(undefined);
    });
  });

  it("Skill 详情页「同步到 Agent」按钮旁有 HelpTip（ariaLabel=什么是同步到 Agent）", async () => {
    const { SkillDetailView } = await import("./Skills");
    const skill = {
      id: "s1",
      name: "weekly-report",
      repo_path: "/repo/weekly-report",
      created_at: 0,
      updated_at: 0,
      status: "draft" as const,
    };
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_skill_usage") return Promise.resolve([]);
      if (cmd === "get_related_skills") return Promise.resolve([]);
      if (cmd === "get_sync_targets") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<SkillDetailView skill={skill} onBack={() => {}} />);

    const trigger = screen.getByLabelText("什么是同步到 Agent");
    expect(trigger).toBeInTheDocument();
    expect(trigger.closest("div")!).toHaveTextContent("同步到 Agent");
  });

  it("设置 → 数据采集页标题旁有 HelpTip（ariaLabel=什么是数据采集）", async () => {
    const DataCollectionPanel = (await import("../components/DataCollectionPanel")).default;
    const { container } = render(<DataCollectionPanel compact={false} showPath title="使用数据采集" />);
    const trigger = screen.getByLabelText("什么是数据采集");
    expect(trigger).toBeInTheDocument();
    expect(container).toHaveTextContent("使用数据采集");
  });

  it("Usage 页 Prompt 语义区有 HelpTip（ariaLabel=什么是归因）", async () => {
    // Usage 需要 window metrics 带 prompt_dimension 才会渲染语义区。
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_window_metrics") {
        return Promise.resolve({
          kind: "week",
          ref_date: "2026-07-04",
          current_window: { start: "2026-06-28", end: "2026-07-04" },
          previous_window: { start: "2026-06-21", end: "2026-06-27" },
          yoy_window: { start: "2025-06-28", end: "2025-07-04" },
          has_previous_baseline: true,
          has_yoy_baseline: false,
          comparison: {},
          token_dimension: null,
          prompt_dimension: {
            penetration: { total_prompts: 0, by_platform: {}, by_project: {} },
            semantics: {
              classified_ratio: 0,
              requested_action: {},
              target_object: {},
              interaction_state: {},
              interaction_mode: {},
            },
            quality: {
              score: 0,
              clarification_correction_rate: 0,
              planning_ratio: 0,
              test_object_ratio: 0,
              improvement_suggestions: [],
            },
          },
          leverage: null,
        });
      }
      if (cmd === "get_collection_status") {
        return Promise.resolve([
          { source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 1 },
        ]);
      }
      if (cmd === "get_high_value_prompts") return Promise.resolve([]);
      if (cmd === "get_agent_usage") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });

    const Usage = (await import("./Usage")).default;
    render(<Usage />);
    const trigger = await screen.findByLabelText("什么是归因");
    expect(trigger).toBeInTheDocument();
  });

  it("设置 → AI 分析（ACP）子页标题旁有 HelpTip（ariaLabel=什么是 ACP）", async () => {
    const AiSettingsPanel = (await import("../components/AiSettingsPanel")).default;
    render(<AiSettingsPanel />);
    const trigger = screen.getByLabelText("什么是 ACP");
    expect(trigger).toBeInTheDocument();
    expect(trigger.closest("h3")!).toHaveTextContent("本地 Agent 分析（ACP）");
  });
});
