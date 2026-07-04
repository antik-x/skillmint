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

function getAgentSelect(): HTMLSelectElement {
  // 下拉区由 label「选择 Agent」标注；label 元素后紧跟 select。
  const container = screen.getByText("选择 Agent").closest("div")!;
  return container.querySelector("select") as HTMLSelectElement;
}

describe("SPEC-F6 T1: ImportSkillModal 按目录去重", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation(() => Promise.resolve(undefined));
  });

  it("27 个同目录 Agent → 下拉仅出现 1 个选项（去重后）", async () => {
    const dir = "/home/u/.skillmint/skills";
    const agents = buildSameDirAgents(27, dir);
    render(<ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />);

    const select = getAgentSelect();
    // 1 个占位「请选择…」+ 1 个去重后的代表项 = 共 2 个 option。
    expect(select.options.length).toBe(2);
    // 代表项的 value 指向第一个 Agent（去重保留首个）。
    expect(select.options[1].value).toBe("agent-0");
  });

  it("不同目录的 Agent 各自保留为独立选项", async () => {
    const agents: Agent[] = [
      { id: "a1", name: "claude", skill_directory: "/x/.claude/skills", is_enabled: true, source: "claude-code" },
      { id: "a2", name: "cursor", skill_directory: "/y/.cursor/rules", is_enabled: true, source: "cursor" },
    ];
    render(<ImportSkillModal agents={agents} onClose={() => {}} onImported={() => {}} />);

    const select = getAgentSelect();
    // 占位 + 2 个独立 Agent。
    expect(select.options.length).toBe(3);
    expect(select.options[1].value).toBe("a1");
    expect(select.options[2].value).toBe("a2");
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

    const select = getAgentSelect();
    await userEvent.selectOptions(select, "a1");
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

    const select = getAgentSelect();
    await userEvent.selectOptions(select, "a1");
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
