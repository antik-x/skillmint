import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SkillDetailView } from "./Skills";
import { useAppStore } from "../stores/appStore";
import { resetHotkeys } from "../hooks/useHotkeys";
import { useToastStore } from "../stores/toastStore";
import ToastContainer from "../components/ToastContainer";
import type { Skill } from "../types";

// Mock the invoke wrapper.
vi.mock("../lib/invoke", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "../lib/invoke";

const sampleSkill: Skill = {
  id: "s1",
  name: "trashable-skill",
  repo_path: "/repo/trashable-skill",
  created_at: 0,
  updated_at: 0,
  status: "draft",
};

function resetStore() {
  useAppStore.setState({
    skills: [sampleSkill],
    agents: [],
    syncTargets: [],
    activeTab: "skillLibrary",
    selectedSkillId: null,
    skillEditMode: false,
    loadData: vi.fn(async () => {}),
  });
  // Clear any leftover toasts from previous tests.
  useToastStore.setState({ toasts: [] });
}

// List-returning commands must resolve to arrays, otherwise the detail view's
// `u.find(...)` throws an unhandled rejection outside the test assertions.
function defaultInvoke(cmd: string): Promise<never> {
  if (cmd === "get_skill_usage" || cmd === "get_related_skills") {
    return Promise.resolve([] as never);
  }
  return Promise.resolve(undefined as never);
}

describe("SkillDetailView trash dialog (SPEC-C3 T3)", () => {
  beforeEach(() => {
    resetHotkeys();
    resetStore();
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation(defaultInvoke);
  });

  it("renders the 移入回收站 button in the detail header", async () => {
    render(<SkillDetailView skill={sampleSkill} onBack={vi.fn()} />);
    expect(await screen.findByRole("button", { name: /移入回收站/ })).toBeInTheDocument();
  });

  it("clicking the button opens the confirmation dialog", async () => {
    render(<SkillDetailView skill={sampleSkill} onBack={vi.fn()} />);
    const btn = await screen.findByRole("button", { name: /移入回收站/ });
    await userEvent.click(btn);
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();
    expect(dialog.textContent).toContain("移入回收站？");
    // The dialog body must mention the 30-day recovery window.
    expect(dialog.textContent).toContain("30 天内");
  });

  it("cancel button closes the dialog without calling remove_skill", async () => {
    render(<SkillDetailView skill={sampleSkill} onBack={vi.fn()} />);
    await userEvent.click(await screen.findByRole("button", { name: /移入回收站/ }));
    const cancelBtn = await screen.findByRole("button", { name: /^取消$/ });
    await userEvent.click(cancelBtn);
    await waitFor(() => {
      expect(screen.queryByText("移入回收站？")).not.toBeInTheDocument();
    });
    expect(invoke).not.toHaveBeenCalledWith("remove_skill", expect.anything());
  });

  it("confirm calls remove_skill and navigates back", async () => {
    const onBack = vi.fn();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "remove_skill") return Promise.resolve(123 as never);
      return defaultInvoke(cmd);
    });
    render(<SkillDetailView skill={sampleSkill} onBack={onBack} />);
    await userEvent.click(await screen.findByRole("button", { name: /移入回收站/ }));
    // The dialog confirm button is the second one matching "移入回收站".
    const trashButtons = await screen.findAllByRole("button", { name: /移入回收站/ });
    expect(trashButtons.length).toBeGreaterThanOrEqual(2);
    await userEvent.click(trashButtons[trashButtons.length - 1]);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("remove_skill", { skillId: "s1" });
    });
    // The detail view navigates back to the list after a successful trash.
    await waitFor(() => {
      expect(onBack).toHaveBeenCalled();
    });
  });

  it("undo action calls restore_trash_item with the returned trash id", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "remove_skill") return Promise.resolve(42 as never);
      if (cmd === "restore_trash_item") return Promise.resolve({} as never);
      return defaultInvoke(cmd);
    });

    // Render both the detail view and the toast container so the undo button
    // is actually mounted in the DOM.
    render(
      <>
        <SkillDetailView skill={sampleSkill} onBack={vi.fn()} />
        <ToastContainer />
      </>,
    );
    await userEvent.click(await screen.findByRole("button", { name: /移入回收站/ }));
    const trashButtons = await screen.findAllByRole("button", { name: /移入回收站/ });
    await userEvent.click(trashButtons[trashButtons.length - 1]);

    // Wait for the undo button to appear in the rendered toast, then click it.
    const undoBtn = await screen.findByRole("button", { name: "撤销" });
    await userEvent.click(undoBtn);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("restore_trash_item", { id: 42, conflictStrategy: null });
    });
  });
});
