import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { SkillListItem, SkillDetailView } from "./Skills";
import { useToastStore } from "../stores/toastStore";
import ToastContainer from "../components/ToastContainer";
import type { Skill, SyncStatus } from "../types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const sampleSkill: Skill = {
  id: "s1",
  name: "weekly-report",
  repo_path: "/repo/weekly-report",
  created_at: 0,
  updated_at: 0,
  status: "draft",
};

const itemProps = {
  id: "skill-0",
  role: "listitem" as const,
  "aria-selected": false as const,
  onClick: vi.fn(),
  onMouseEnter: vi.fn(),
  onKeyDown: vi.fn(),
  tabIndex: -1 as const,
};

function renderItem(overrides: { syncStatus?: SyncStatus; label?: string } = {}) {
  const onOpen = vi.fn();
  const onEdit = vi.fn();
  render(
    <SkillListItem
      skill={sampleSkill}
      index={0}
      active={false}
      itemProps={itemProps}
      syncStatus={{ status: overrides.syncStatus ?? "synced", label: overrides.label ?? "已同步" }}
      onOpen={onOpen}
      onEdit={onEdit}
    />,
  );
  return { onOpen, onEdit };
}

describe("Skills page", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("opens skill detail on Enter when list item has focus", async () => {
    const { onOpen, onEdit } = renderItem();
    const row = screen.getByRole("listitem");
    row.focus();
    await userEvent.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onEdit).not.toHaveBeenCalled();
  });

  it("triggers edit mode on Enter from the edit button without propagating to the row", async () => {
    const { onOpen, onEdit } = renderItem();
    const button = screen.getByRole("button", { name: /编辑 weekly-report/i });
    button.focus();
    await userEvent.keyboard("{Enter}");
    expect(onEdit).toHaveBeenCalledTimes(1);
    expect(onOpen).not.toHaveBeenCalled();
    expect(itemProps.onKeyDown).not.toHaveBeenCalled();
  });

  it("triggers edit mode on Space from the edit button without propagating to the row", async () => {
    const { onOpen, onEdit } = renderItem();
    const button = screen.getByRole("button", { name: /编辑 weekly-report/i });
    button.focus();
    await userEvent.keyboard(" ");
    expect(onEdit).toHaveBeenCalledTimes(1);
    expect(onOpen).not.toHaveBeenCalled();
    expect(itemProps.onKeyDown).not.toHaveBeenCalled();
  });

  // SPEC-F5 T4: the per-skill sync status must render as a colored dot + label
  // on each list row, with the dot class matching the status semantics.
  it.each([
    ["synced", "bg-success", "已同步"],
    ["center_changed", "bg-accent", "中心已更新"],
    ["local_changed", "bg-accent", "有本地变更"],
    ["conflict", "bg-warning", "部分失败"],
    ["broken", "bg-danger", "目标失效"],
  ] as const)("renders sync dot (%s) and label", (status, dotClass, label) => {
    renderItem({ syncStatus: status, label });
    const indicator = document.querySelector(`[data-sync-status="${status}"]`);
    expect(indicator).not.toBeNull();
    const dot = indicator!.querySelector("span.inline-block");
    expect(dot?.className).toContain(dotClass);
    expect(indicator).toHaveTextContent(label);
  });
});

// SPEC-F6 T4: 编辑保存成功后的 toast 带「同步到 Agent」动作，点击触发 sync_all_command。
describe("SPEC-F6 T4: SkillDetailView 保存 toast 的同步动作", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useToastStore.setState({ toasts: [] });
  });

  it("header 的「同步到 Agent」按钮点击调用 sync_all_command（handleSync 可用）", async () => {
    const skill: Skill = {
      id: "s1",
      name: "weekly-report",
      repo_path: "/repo/weekly-report",
      created_at: 0,
      updated_at: 0,
      status: "draft",
    };
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "sync_all_command") {
        return Promise.resolve({ success_count: 1, failure_count: 0, failures: [] });
      }
      if (cmd === "get_skill_usage") return Promise.resolve([]);
      if (cmd === "get_related_skills") return Promise.resolve([]);
      if (cmd === "get_sync_targets") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<SkillDetailView skill={skill} onBack={() => {}} />);

    const headerBtn = screen.getAllByRole("button", { name: /同步到 Agent/ })[0];
    await act(async () => {
      await userEvent.click(headerBtn);
    });
    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith("sync_all_command"));
  });

  it("SkillEditor 保存成功后触发带「同步到 Agent」动作的 toast，点击调用 sync_all_command", async () => {
    const skill: Skill = {
      id: "s1",
      name: "weekly-report",
      repo_path: "/repo/weekly-report",
      created_at: 0,
      updated_at: 0,
      status: "draft",
    };
    const savedSkill: Skill = { ...skill };
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "sync_all_command") {
        return Promise.resolve({ success_count: 1, failure_count: 0, failures: [] });
      }
      if (cmd === "read_skill_content") {
        return Promise.resolve({ frontmatter: { name: "weekly-report" }, body: "# x" });
      }
      if (cmd === "save_skill_content") return Promise.resolve(savedSkill);
      if (cmd === "list_skill_versions_command") return Promise.resolve([]);
      if (cmd === "get_skill_usage") return Promise.resolve([]);
      if (cmd === "get_related_skills") return Promise.resolve([]);
      if (cmd === "get_sync_targets") return Promise.resolve([]);
      if (cmd === "check_skill_external_change") return Promise.resolve(false);
      return Promise.resolve(undefined);
    });

    render(
      <>
        <SkillDetailView skill={skill} onBack={() => {}} />
        <ToastContainer />
      </>,
    );

    // 进入编辑页签。
    await userEvent.click(screen.getByText("编辑"));
    // 等编辑器加载完成（出现「双栏」模式切换）。
    await waitFor(() => expect(screen.getByText("双栏")).toBeInTheDocument());

    // 点击工具栏「保存」。
    const saveBtn = screen.getByRole("button", { name: "保存" });
    await act(async () => {
      await userEvent.click(saveBtn);
    });

    // toast 应出现「同步到 Agent」动作（header 按钮也有此文案，取最后一个——toast 渲染在 body 末尾）。
    const syncActions = await screen.findAllByText("同步到 Agent");
    // 至少有 header + toast 两处；点击 toast 动作（最后一个）。
    const toastSyncAction = syncActions[syncActions.length - 1];
    await act(async () => {
      await userEvent.click(toastSyncAction);
    });
    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith("sync_all_command"));
  });
});
