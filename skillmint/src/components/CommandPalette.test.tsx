import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useAppStore } from "../stores/appStore";
import CommandPalette from "./CommandPalette";
import { invoke } from "@tauri-apps/api/core";
import { resetHotkeys } from "../hooks/useHotkeys";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function resetStore() {
  useAppStore.setState({
    initialized: true,
    activeTab: "today",
    skillLibrarySubTab: "skills",
    settingsSubTab: "preferences",
    skills: [],
    selectedSkillId: null,
    newSkillRequest: 0,
  });
}

function createStoreState() {
  resetStore();
  useAppStore.setState({
    skills: [
      { id: "s1", name: "password-hash", repo_path: "/repo/password-hash", created_at: 0, updated_at: 0, status: "approved" },
      { id: "s2", name: "weekly-report", repo_path: "/repo/weekly-report", created_at: 0, updated_at: 0, status: "draft" },
    ],
  });
}

describe("CommandPalette", () => {
  beforeEach(() => {
    resetHotkeys();
    createStoreState();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("filters results by query", async () => {
    render(<CommandPalette open onClose={vi.fn()} />);

    const input = screen.getByPlaceholderText("搜索页面、Skill 或动作…");
    await userEvent.type(input, "password");

    expect(screen.getByText("password-hash")).toBeInTheDocument();
    expect(screen.queryByText("weekly-report")).not.toBeInTheDocument();
  });

  it("moves active item with arrow keys and executes on Enter", async () => {
    const onClose = vi.fn();
    render(<CommandPalette open onClose={onClose} />);

    const input = screen.getByPlaceholderText("搜索页面、Skill 或动作…");
    // P3: the sync-all action became "刷新技能索引" (index rebuild).
    await userEvent.type(input, "索引");

    await userEvent.keyboard("{ArrowDown}");
    await userEvent.keyboard("{Enter}");

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("rebuild_skill_index", { projectRoot: null });
    });
  });

  it("closes on Escape", async () => {
    const onClose = vi.fn();
    render(<CommandPalette open onClose={onClose} />);

    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("debounces remote skill search", async () => {
    vi.mocked(invoke).mockResolvedValueOnce([
      {
        skill_name: "remote-skill",
        origin: "remote",
        source_id: "src1",
        source_name: "github",
        installed_locally: false,
        usage_count: 0,
        correction_count: 0,
        skill_path: "remote-skill",
      },
    ]);

    render(<CommandPalette open onClose={vi.fn()} />);
    const input = screen.getByPlaceholderText("搜索页面、Skill 或动作…");
    await userEvent.type(input, "remote");

    expect(invoke).not.toHaveBeenCalled();
    vi.advanceTimersByTime(250);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("search_all", { query: "remote" });
    });
  });

  it("navigates to a page when selected", async () => {
    const onClose = vi.fn();
    render(<CommandPalette open onClose={onClose} />);

    const input = screen.getByPlaceholderText("搜索页面、Skill 或动作…");
    await userEvent.type(input, "设置");
    await userEvent.keyboard("{Enter}");

    expect(useAppStore.getState().activeTab).toBe("settings");
    expect(onClose).toHaveBeenCalled();
  });

  it("opens the shortcuts help dialog", async () => {
    render(<CommandPalette open onClose={vi.fn()} />);

    const input = screen.getByPlaceholderText("搜索页面、Skill 或动作…");
    await userEvent.type(input, "键盘快捷键");
    await userEvent.keyboard("{Enter}");

    await waitFor(() => {
      expect(screen.getByRole("dialog", { name: /键盘快捷键/i })).toBeInTheDocument();
    });
  });
});
