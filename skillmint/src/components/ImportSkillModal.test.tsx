import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../stores/appStore";
import { useToastStore } from "../stores/toastStore";
import ToastContainer from "./ToastContainer";
import ImportSkillModal from "./ImportSkillModal";
import type { Agent, Skill } from "../types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

// SPEC-F6 T1: 指向同一 skill_directory 的多个 Agent 在下拉里只展示一个代表项。
function buildSameDirAgents(n: number, dir: string): Agent[] {
  return Array.from({ length: n }, (_, i) => ({
    id: `agent-${i}`,
    name: "skills",
    skill_directory: dir,
    is_enabled: true,
    source: "claude-code",
  }));
}

// P1-3: 自绘 combobox——label「选择 Agent」经 aria-labelledby 标注触发按钮。
function getAgentCombobox(): HTMLElement {
  return screen.getByRole("button", { name: "选择 Agent" });
}

async function chooseAgentOption(name: string | RegExp) {
  await userEvent.click(getAgentCombobox());
  await userEvent.click(await screen.findByRole("option", { name }));
}

describe("SPEC-F6 T1: ImportSkillModal 按目录去重", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation(() => Promise.resolve(undefined));
  });

  it("27 个同目录 Agent → 下拉仅出现 1 个选项（去重后）", async () => {
    const dir = "/home/u/.skillmint/skills";
    const agents = buildSameDirAgents(27, dir);
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agent_skills") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    render(<ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />);

    await userEvent.click(getAgentCombobox());
    const options = await screen.findAllByRole("option");
    expect(options.length).toBe(1);

    // 代表项指向第一个 Agent（去重保留首个）。
    await userEvent.click(options[0]);
    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith("scan_agent_skills", { agentId: "agent-0" }),
    );
  });

  it("不同目录的 Agent 各自保留为独立选项", async () => {
    const agents: Agent[] = [
      { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
      { id: "a2", name: "cursor", skill_directory: "/y/.cursor/rules", is_enabled: true, source: "cursor" },
    ];
    render(<ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />);

    await userEvent.click(getAgentCombobox());
    const options = await screen.findAllByRole("option");
    expect(options.length).toBe(2);
    expect(options[0]).toHaveTextContent("claude");
    expect(options[1]).toHaveTextContent("cursor");
  });

  it("选择 Agent 后加载技能并导入成功", async () => {
    const agents: Agent[] = [
      { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
    ];
    const onImported = vi.fn();
    const onClose = vi.fn();
    const importedSkill: Skill = {
      id: "s-imported-1",
      name: "weekly-report",
      repo_path: "/repo/weekly-report",
      created_at: 0,
      updated_at: 0,
      status: "draft",
    };
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agent_skills") {
        return Promise.resolve([{ name: "weekly-report", exists_in_center: false }]);
      }
      if (cmd === "import_skill") return Promise.resolve(importedSkill);
      return Promise.resolve(undefined);
    });

    render(<ImportSkillModal agents={agents} onClose={onClose} onImported={onImported} />);

    await chooseAgentOption("claude");
    await waitFor(() => expect(screen.getByText("weekly-report")).toBeInTheDocument());

    await userEvent.click(screen.getByText("weekly-report"));
    const importBtn = screen.getByRole("button", { name: /导入 1 个/ });
    await userEvent.click(importBtn);

    await waitFor(() => expect(onImported).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  // SPEC-F6 T4: 导入成功 toast 带「查看导入的 Skill」动作，点击触发 navigateToSkill。
  it("导入成功后 toast 带「查看导入的 Skill」动作，点击调用 navigateToSkill", async () => {
    useAppStore.setState({
      navigateToSkill: vi.fn(),
    } as never);

    const agents: Agent[] = [
      { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
    ];
    const importedSkill: Skill = {
      id: "s-imported-2",
      name: "weekly-report",
      repo_path: "/repo/weekly-report",
      created_at: 0,
      updated_at: 0,
      status: "draft",
    };
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agent_skills") {
        return Promise.resolve([{ name: "weekly-report", exists_in_center: false }]);
      }
      if (cmd === "import_skill") return Promise.resolve(importedSkill);
      return Promise.resolve(undefined);
    });

    useToastStore.setState({ toasts: [] });
    render(
      <>
        <ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />
        <ToastContainer />
      </>,
    );

    await chooseAgentOption("claude");
    await waitFor(() => expect(screen.getByText("weekly-report")).toBeInTheDocument());
    await userEvent.click(screen.getByText("weekly-report"));
    await userEvent.click(screen.getByRole("button", { name: /导入 1 个/ }));

    const actionBtn = await screen.findByText("查看导入的 Skill");
    await act(async () => {
      await userEvent.click(actionBtn);
    });
    expect((useAppStore.getState() as unknown as { navigateToSkill: ReturnType<typeof vi.fn> }).navigateToSkill)
      .toHaveBeenCalledWith("s-imported-2");
  });
});

// P0-2: agent 侧合法指向 center 的软链 → content_match=true，UI 标注
// 「与中心一致」，默认不勾选，且永远不会出现「保留本地/保留中心」（防止
// 软链被实体目录覆盖）。只有实体目录内容分叉才显示「内容冲突」。
describe("P0-2: 软链条目标注「与中心一致」而非「内容冲突」", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockClear();
  });

  it("与中心一致条目默认不勾选、无冲突解决按钮；实体分叉仍显示内容冲突", async () => {
    const agents: Agent[] = [
      { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
    ];
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agent_skills") {
        return Promise.resolve([
          // 后端 P0-2 短路：合法指向 center 的软链 → content_match=true
          { name: "whyactions-seo", exists_in_center: true, content_match: true },
          // 实体目录内容分叉 → 仍是冲突
          { name: "forked-skill", exists_in_center: true, content_match: false },
        ]);
      }
      return Promise.resolve(undefined);
    });

    render(<ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />);
    await chooseAgentOption("claude");
    await waitFor(() => expect(screen.getByText("whyactions-seo")).toBeInTheDocument());

    expect(screen.getByText("与中心一致")).toBeInTheDocument();
    expect(screen.getByText("内容冲突")).toBeInTheDocument();

    // 默认不勾选，且没有渲染任何冲突解决按钮。
    const syncedCheckbox = screen.getByRole("checkbox", { name: /whyactions-seo/ });
    expect(syncedCheckbox).not.toBeChecked();
    expect(screen.queryByText("保留本地")).not.toBeInTheDocument();
    expect(screen.queryByText("保留中心")).not.toBeInTheDocument();

    // 勾选「与中心一致」条目后也不会出现 保留本地/保留中心。
    await userEvent.click(syncedCheckbox);
    expect(screen.queryByText("保留本地")).not.toBeInTheDocument();
    expect(screen.queryByText("保留中心")).not.toBeInTheDocument();
  });

  it("勾选实体冲突条目才会出现「保留本地」", async () => {
    const agents: Agent[] = [
      { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
    ];
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agent_skills") {
        return Promise.resolve([
          { name: "forked-skill", exists_in_center: true, content_match: false },
        ]);
      }
      return Promise.resolve(undefined);
    });

    render(<ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />);
    await chooseAgentOption("claude");
    await waitFor(() => expect(screen.getByText("forked-skill")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("checkbox", { name: /forked-skill/ }));
    expect(screen.getByText("保留本地")).toBeInTheDocument();
    expect(screen.getByText("保留中心")).toBeInTheDocument();
  });
});

// P0-3: 批量操作——全选 / 仅选"新 Skill" / 清空（痛点：手动勾 25 个框）。
describe("P0-3: 批量选择按钮", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockClear();
  });

  it("全选选中全部；仅选新 Skill 只留未入库项；清空归零", async () => {
    const agents: Agent[] = [
      { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
    ];
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agent_skills") {
        return Promise.resolve([
          { name: "new-skill", exists_in_center: false },
          { name: "synced-skill", exists_in_center: true, content_match: true },
          { name: "conflict-skill", exists_in_center: true, content_match: false },
        ]);
      }
      return Promise.resolve(undefined);
    });

    render(<ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />);
    await chooseAgentOption("claude");
    await waitFor(() => expect(screen.getByText("new-skill")).toBeInTheDocument());

    const newBox = screen.getByRole("checkbox", { name: /new-skill/ });
    const syncedBox = screen.getByRole("checkbox", { name: /synced-skill/ });
    const conflictBox = screen.getByRole("checkbox", { name: /conflict-skill/ });

    // 全选
    await userEvent.click(screen.getByText("全选"));
    expect(newBox).toBeChecked();
    expect(syncedBox).toBeChecked();
    expect(conflictBox).toBeChecked();
    expect(screen.getByText("已选 3 / 3")).toBeInTheDocument();

    // 仅选"新 Skill"
    await userEvent.click(screen.getByText(/仅选/));
    expect(newBox).toBeChecked();
    expect(syncedBox).not.toBeChecked();
    expect(conflictBox).not.toBeChecked();

    // 清空
    await userEvent.click(screen.getByText("清空"));
    expect(newBox).not.toBeChecked();
    expect(screen.getByText("已选 0 / 3")).toBeInTheDocument();
  });
});

// P1-3: 自绘 combobox 的键盘与合成事件可达性——原生 <select> 的
// AXPopUpButton 不响应合成输入，这里全部走普通 button + listbox。
describe("P1-3: Agent combobox 键盘与合成事件", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agent_skills") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
  });

  const twoAgents: Agent[] = [
    { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
    { id: "a2", name: "cursor", skill_directory: "/y/.cursor/rules", is_enabled: true, source: "cursor" },
  ];

  it("Esc 关闭；Enter 展开；上下键导航；Enter 选定", async () => {
    render(<ImportSkillModal agents={twoAgents} onClose={() => {}} onImported={() => {}} />);

    // 合成点击展开（AXPress 等价路径）。
    await userEvent.click(getAgentCombobox());
    expect(await screen.findByRole("listbox")).toBeInTheDocument();

    // Esc 关闭。
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();

    // Enter 展开（焦点仍在按钮上），默认高亮第一项；ArrowDown 移到第二项；Enter 选定。
    getAgentCombobox().focus();
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("listbox")).toBeInTheDocument();
    await userEvent.keyboard("{ArrowDown}");
    await userEvent.keyboard("{Enter}");

    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(getAgentCombobox()).toHaveTextContent("cursor");
    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith("scan_agent_skills", { agentId: "a2" }),
    );
  });

  it("全链路：选 agent → 渲染扫描结果 → 勾选 → 导入计数变化", async () => {
    const agents: Agent[] = [
      { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
    ];
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_agent_skills") {
        return Promise.resolve([
          { name: "alpha", exists_in_center: false },
          { name: "beta", exists_in_center: false },
        ]);
      }
      if (cmd === "import_skill") {
        return Promise.resolve({ id: "s1", name: "alpha", repo_path: "/r/alpha", created_at: 0, updated_at: 0, status: "draft" });
      }
      return Promise.resolve(undefined);
    });

    render(<ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />);

    // 初始：无勾选，导入按钮计数为 0。
    expect(screen.getByRole("button", { name: /导入 0 个/ })).toBeDisabled();

    await chooseAgentOption("claude");
    await waitFor(() => expect(screen.getByText("alpha")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("checkbox", { name: /alpha/ }));
    expect(screen.getByRole("button", { name: /导入 1 个/ })).toBeEnabled();
    await userEvent.click(screen.getByRole("checkbox", { name: /beta/ }));
    expect(screen.getByRole("button", { name: /导入 2 个/ })).toBeEnabled();
  });
});
