import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useAppStore } from "../stores/appStore";
import { useCollectionStore } from "../stores/collectionStore";
import Onboarding from "./Onboarding";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function resetStore() {
  useAppStore.setState({
    initialized: true,
    activeTab: "today",
    agents: [{ id: "a1", name: "claude-code", skill_directory: "/x", is_enabled: true, source: "claude-code" }],
    skills: [],
    syncTargets: [],
    settings: {
      device_id: "",
      auto_sync_interval_minutes: 0,
      launch_at_login: false,
      show_dock_icon: true,
      onboarding_completed: false,
      remote_enabled: false,
      theme: "system",
      ai: { models: [], acp_connections: [], prefer_acp: false, strict_local_mode: false },
    },
  });
  useCollectionStore.setState({
    sources: [],
    collecting: false,
    currentJobId: null,
    lastJobStats: null,
    lastJobError: null,
  });
}

describe("Onboarding", () => {
  beforeEach(() => {
    resetStore();
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation(() => Promise.resolve(undefined));
  });

  it("unlocks next step after collection is cancelled", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agents") return Promise.resolve([{ id: "a1", name: "claude-code", skill_directory: "/x", is_enabled: true, source: "claude-code" }]);
      if (cmd === "get_collection_status") return Promise.resolve([]);
      if (cmd === "start_collection_job") return Promise.resolve("job-1");
      if (cmd === "cancel_collection_job") return Promise.resolve(undefined);
      if (cmd === "save_settings") return Promise.resolve(useAppStore.getState().settings);
      return Promise.resolve(undefined);
    });

    render(<Onboarding />);
    // advance to the collection step
    await userEvent.click(screen.getByText("开始设置"));
    await userEvent.click(screen.getByText("下一步：预览范围"));
    await userEvent.click(screen.getByText("下一步：开始采集"));
    await userEvent.click(screen.getByText("开始第一次采集"));

    await waitFor(() => expect(screen.getByText("采集中…")).toBeInTheDocument());
    const nextButton = screen.getByText("下一步：查看 Skill").closest("button");
    expect(nextButton).toBeDisabled();

    await userEvent.click(screen.getByText("取消"));
    await waitFor(() => expect(screen.getByText("跳过并查看 Skill")).toBeInTheDocument());
    expect(screen.getByText("跳过并查看 Skill").closest("button")).not.toBeDisabled();
  });

  it("shows skip button after long timeout", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agents") return Promise.resolve([{ id: "a1", name: "claude-code", skill_directory: "/x", is_enabled: true, source: "claude-code" }]);
      if (cmd === "get_collection_status") return Promise.resolve([]);
      if (cmd === "start_collection_job") return Promise.resolve("job-1");
      if (cmd === "save_settings") return Promise.resolve(useAppStore.getState().settings);
      return Promise.resolve(undefined);
    });

    render(<Onboarding />);
    await user.click(screen.getByText("开始设置"));
    await user.click(screen.getByText("下一步：预览范围"));
    await user.click(screen.getByText("下一步：开始采集"));
    await user.click(screen.getByText("开始第一次采集"));
    await waitFor(() => expect(screen.getByText("采集中…")).toBeInTheDocument());

    await act(async () => {
      vi.advanceTimersByTime(61000);
    });
    expect(screen.getAllByRole("button", { name: "跳过" }).length).toBeGreaterThanOrEqual(1);

    vi.useRealTimers();
  });

  // SPEC-F5 T1: 60s 长超时与进度解耦。即使 sources 有 ok 进度但 job 一直未完成，
  // 跳过按钮也必须出现，且可点击进入下一步（不被其他条件二次锁死）。
  it("shows clickable skip after 60s even when sources have progress but job never completes", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agents") return Promise.resolve([{ id: "a1", name: "claude-code", skill_directory: "/x", is_enabled: true, source: "claude-code" }]);
      if (cmd === "get_collection_status") return Promise.resolve([
        { source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 3 },
      ]);
      if (cmd === "get_skills") return Promise.resolve([]);
      if (cmd === "get_agents") return Promise.resolve([]);
      if (cmd === "get_sync_targets") return Promise.resolve([]);
      if (cmd === "start_collection_job") return Promise.resolve("job-1");
      if (cmd === "save_settings") return Promise.resolve(useAppStore.getState().settings);
      return Promise.resolve(undefined);
    });

    render(<Onboarding />);
    await user.click(screen.getByText("开始设置"));
    await user.click(screen.getByText("下一步：预览范围"));
    await user.click(screen.getByText("下一步：开始采集"));
    await user.click(screen.getByText("开始第一次采集"));
    await waitFor(() => expect(screen.getByText("采集中…")).toBeInTheDocument());

    // Simulate a source reporting progress but the job itself never completing.
    await act(async () => {
      useCollectionStore.setState({
        sources: [{ source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 3 }],
      });
    });

    // <60s: no skip yet.
    await act(async () => { vi.advanceTimersByTime(30000); });
    expect(screen.queryByText("跳过并查看 Skill")).toBeNull();

    // >=60s: skip appears and is enabled.
    await act(async () => { vi.advanceTimersByTime(35000); });
    const skipButton = await waitFor(() =>
      screen.getByText("跳过并查看 Skill").closest("button")
    );
    expect(skipButton).not.toBeDisabled();

    // Clicking it advances to the skills step.
    await user.click(skipButton!);
    await waitFor(() => expect(screen.getByText("你的第一批 Skill")).toBeInTheDocument());

    vi.useRealTimers();
  });

  // SPEC-F6 T1: Onboarding 第 2 步用 agentDisplayName 消歧，不再直接渲染 agent.name。
  // 评委原话「Onboarding 第 2 步 36 行重复『Agent』」——同名 Agent 必须被消歧后展示。
  it("renders disambiguated agent names on step 2 when agents share a name", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agents") return Promise.resolve([
        { id: "a1", name: "skills", skill_directory: "/x/.cursor/skills", is_enabled: true, source: "cursor" },
        { id: "a2", name: "skills", skill_directory: "/y/.claude/rules", is_enabled: true, source: "claude-code" },
      ]);
      return Promise.resolve(undefined);
    });

    render(<Onboarding />);
    // 第一步的「开始设置」触发 scan_agents 并进入第 2 步。
    await userEvent.click(screen.getByText("开始设置"));
    await waitFor(() => expect(screen.getByText("扫描本机 Agent")).toBeInTheDocument());

    // 两个同名 Agent 必须出现不同的显示名（至少有一项带「·」消歧符）。
    const a1Label = screen.getByText("/x/.cursor/skills", { exact: false });
    // 父容器里应能找到带消歧符的显示名。
    const row1 = a1Label.closest("label")!.textContent;
    const row2 = screen.getByText("/y/.claude/rules", { exact: false }).closest("label")!.textContent;
    expect(row1).not.toBe(row2);
    // 至少其中一行带消歧点号。
    expect(row1!.includes("·") || row2!.includes("·")).toBe(true);
  });
});
