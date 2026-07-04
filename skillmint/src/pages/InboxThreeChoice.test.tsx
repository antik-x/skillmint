import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import Inbox from "./Inbox";
import { useAppStore } from "../stores/appStore";
import { useToastStore } from "../stores/toastStore";
import { resetHotkeys } from "../hooks/useHotkeys";
import type { Discovery } from "../types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function makeDiscovery(overrides: Partial<Discovery> = {}): Discovery {
  return {
    id: `d-${overrides.id ?? Math.random().toString(36).slice(2)}`,
    kind: "repeat_pattern",
    title: "三选测试发现",
    payload: {
      evidence: [
        { session_id: "s1", prompt_text: "证据一", started_at: 1750000000, source: "cursor", project: "hub" },
      ],
      draft_skill: { name: "测试Skill", body: "草稿" },
    },
    confidence: 0.85,
    status: "pending",
    created_at: 1750000000,
    ...overrides,
  };
}

function makeSkillFeedback(): Discovery {
  return {
    id: "sf-1",
    kind: "skill_feedback",
    title: "Skill 效果反馈",
    payload: {
      evidence: [],
      stats: { 效率: 120, 基线: 100 },
    },
    confidence: 0.7,
    status: "pending",
    created_at: 1750000000,
  };
}

function resetStore() {
  useAppStore.setState({
    activeTab: "inbox",
    selectedSkillId: null,
    skillEditMode: false,
    discoverSearchTerm: null,
    inboxRefreshKey: 0,
  });
  useToastStore.setState({ toasts: [] });
}

describe("SPEC-C2: three-choice inbox", () => {
  beforeEach(() => {
    resetHotkeys();
    resetStore();
    vi.mocked(invoke).mockReset();
  });

  // T1: three buttons for repeat_pattern / high_value_prompt
  it("T1: renders three-choice action buttons for repeat_pattern", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    expect(await screen.findByTestId("three-choice-actions")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /采纳并同步/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /修改后采纳/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /拒绝/ })).toBeInTheDocument();
  });

  it("T1: skill_feedback does not show three-choice buttons", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeSkillFeedback()]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    await screen.findByText("Skill 效果反馈");
    expect(screen.queryByTestId("three-choice-actions")).not.toBeInTheDocument();
    expect(screen.queryByTestId("dismiss-reasons")).not.toBeInTheDocument();
  });

  it("T1: accept calls decide with action='accept'", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, _args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery") {
        return Promise.resolve({
          created_skill_id: "skill-x",
          sync_summary: { success: 2, failed: 0, failures: [] },
        });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    await userEvent.click(screen.getByRole("button", { name: /采纳并同步/ }));
    await waitFor(() => {
      const calls = vi.mocked(invoke).mock.calls.filter(
        (c) => c[0] === "decide_discovery" && (c[1] as { action: string }).action === "accept"
      );
      expect(calls.length).toBe(1);
    });
  });

  it("T1: accept_edited calls decide with action='accept_edited' and navigates to editor", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "accept_edited") {
        return Promise.resolve({ created_skill_id: "skill-edit" });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    await userEvent.click(screen.getByRole("button", { name: /修改后采纳/ }));
    await waitFor(() => expect(useAppStore.getState().selectedSkillId).toBe("skill-edit"));
  });

  it("T1: reject opens inline reason picker with three options", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    await userEvent.click(screen.getByRole("button", { name: /拒绝/ }));
    expect(await screen.findByTestId("dismiss-reasons")).toBeInTheDocument();
    expect(screen.getByText("内容不对")).toBeInTheDocument();
    expect(screen.getByText("太琐碎")).toBeInTheDocument();
    expect(screen.getByText("重复了已有 Skill")).toBeInTheDocument();
  });

  it("T1: selecting a reason calls decide with action='dismiss' and the reason", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, _args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery") return Promise.resolve({ created_skill_id: null });
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    await userEvent.click(screen.getByRole("button", { name: /拒绝/ }));
    await screen.findByTestId("dismiss-reasons");
    await userEvent.click(screen.getByText("太琐碎"));
    await waitFor(() => {
      const calls = vi.mocked(invoke).mock.calls.filter(
        (c) =>
          c[0] === "decide_discovery" &&
          (c[1] as { action: string }).action === "dismiss" &&
          (c[1] as { reason: string }).reason === "trivial"
      );
      expect(calls.length).toBe(1);
    });
  });

  // T2: partial_synced banner
  it("T2: accept with sync_summary.failed > 0 shows partial banner and keeps card", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "accept") {
        return Promise.resolve({
          created_skill_id: "skill-pf",
          sync_summary: {
            success: 1,
            failed: 1,
            failures: [{
              target_id: "t1", skill_id: "skill-pf", skill_name: "Test",
              agent_id: "a1", agent_name: "Cursor", error: "Permission denied",
              recovery_hint: "请检查 Agent 目录的写权限",
            }],
          },
        });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    await userEvent.click(screen.getByRole("button", { name: /采纳并同步/ }));
    const banner = await screen.findByTestId("partial-sync-banner");
    expect(banner).toBeInTheDocument();
    expect(banner.textContent).toContain("1 个 Agent 同步失败");
    expect(banner.textContent).toContain("请检查 Agent 目录的写权限");
    // Card still visible.
    expect(screen.getByText("三选测试发现")).toBeInTheDocument();
  });

  it("T2: retry sync calls sync_single_skill_command", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "accept") {
        return Promise.resolve({
          created_skill_id: "skill-rt",
          sync_summary: { success: 0, failed: 1, failures: [{ target_id: "t1", skill_id: "skill-rt", agent_id: "a1", error: "denied" }] },
        });
      }
      if (cmd === "sync_single_skill_command") {
        return Promise.resolve({ success_count: 1, failure_count: 0, failures: [], targets: [], imported_skills: 0, import_conflicts: 0 });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    await userEvent.click(screen.getByRole("button", { name: /采纳并同步/ }));
    await screen.findByTestId("partial-sync-banner");
    await userEvent.click(screen.getByRole("button", { name: /重试同步/ }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("sync_single_skill_command", { skillId: "skill-rt" });
    });
  });

  it("T2: ignore removes the card leaving the skill created", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "accept") {
        return Promise.resolve({
          created_skill_id: "skill-ig",
          sync_summary: { success: 0, failed: 1, failures: [{ target_id: "t1", skill_id: "skill-ig", agent_id: "a1", error: "x" }] },
        });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    await userEvent.click(screen.getByRole("button", { name: /采纳并同步/ }));
    await screen.findByTestId("partial-sync-banner");
    await userEvent.click(screen.getByRole("button", { name: /忽略/ }));
    await waitFor(() => {
      expect(screen.queryByText("三选测试发现")).not.toBeInTheDocument();
    });
  });

  // T3: keyboard A/E/R + reason numbers + Esc layering
  it("T3: A key triggers accept", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "accept") {
        return Promise.resolve({ created_skill_id: "s-a", sync_summary: { success: 0, failed: 0, failures: [] } });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    fireEvent.keyDown(document, { key: "a", code: "KeyA" });
    await waitFor(() => {
      const calls = vi.mocked(invoke).mock.calls.filter(
        (c) => c[0] === "decide_discovery" && (c[1] as { action: string }).action === "accept"
      );
      expect(calls.length).toBe(1);
    });
  });

  it("T3: E key triggers accept_edited", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "accept_edited") {
        return Promise.resolve({ created_skill_id: "s-e" });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    fireEvent.keyDown(document, { key: "e", code: "KeyE" });
    await waitFor(() => expect(useAppStore.getState().selectedSkillId).toBe("s-e"));
  });

  it("T3: R key opens reason picker, 3 selects duplicate, card leaves", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, _args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery") return Promise.resolve({ created_skill_id: null });
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    fireEvent.keyDown(document, { key: "r", code: "KeyR" });
    await screen.findByTestId("dismiss-reasons");
    fireEvent.keyDown(document, { key: "3", code: "Digit3" });
    await waitFor(() => {
      const calls = vi.mocked(invoke).mock.calls.filter(
        (c) =>
          c[0] === "decide_discovery" &&
          (c[1] as { reason: string }).reason === "duplicate"
      );
      expect(calls.length).toBe(1);
    });
  });

  // T4: score-source badge + low-confidence tab
  it("T4: shows 规则评分 badge for rule-based discovery", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    expect(await screen.findByText("规则评分")).toBeInTheDocument();
  });

  it("T4: shows AI 评分 badge for LLM-enhanced discovery", async () => {
    const d = makeDiscovery({ id: "1" });
    (d.payload as Record<string, unknown>).enhanced_by = "llm";
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([d]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    expect(await screen.findByText("AI 评分")).toBeInTheDocument();
  });

  it("T4: low-confidence tab loads gate rejections and is read-only", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([]);
      if (cmd === "list_gate_rejections") {
        return Promise.resolve([{
          id: 1, reason: "below_threshold", kind: "high_value_prompt",
          confidence: 0.45, payload: { title: "低置信候选" }, created_at: 1750000000,
        }]);
      }
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    // Click the low-confidence tab.
    await screen.findByText(/待确认/);
    const lowConfTab = screen.getByText("低置信");
    await userEvent.click(lowConfTab);
    expect(await screen.findByText("低置信候选")).toBeInTheDocument();
    // No three-choice actions in low-confidence view.
    expect(screen.queryByTestId("three-choice-actions")).not.toBeInTheDocument();
  });

  it("T4: low-confidence tab empty state", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([]);
      if (cmd === "list_gate_rejections") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    await screen.findByText(/待确认/);
    await userEvent.click(screen.getByText("低置信"));
    expect(await screen.findByText("没有被闸门保留的低置信候选")).toBeInTheDocument();
  });

  // T5: completion stats include 修改采纳
  it("T5: completion stats show 沉淀 / 修改采纳 / 拒绝", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "accept_edited") {
        return Promise.resolve({ created_skill_id: "skill-t5" });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");
    await userEvent.click(screen.getByRole("button", { name: /修改后采纳/ }));
    await waitFor(() => expect(useAppStore.getState().selectedSkillId).toBe("skill-t5"));
    const stats = await screen.findByTestId("inbox-completion-stats");
    expect(stats.textContent).toContain("修改采纳 1");
  });

  // T3: hotkeys must not fire when an input is focused (typing guard).
  it("T3: A/E/R keys do not trigger decide when an input is focused", async () => {
    const mockFn = vi.mocked(invoke).mockImplementation(((cmd: string, _args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery") return Promise.resolve({ created_skill_id: "should-not-happen" });
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("三选测试发现");

    // Render an input inside the document and focus it (simulates the user
    // typing into a search/filter field while a discovery card is visible).
    const input = document.createElement("input");
    input.type = "text";
    input.dataset.testid = "test-input";
    document.body.appendChild(input);
    input.focus();
    expect(document.activeElement).toBe(input);

    // Pressing A/E/R while the input has focus must NOT trigger any decision.
    fireEvent.keyDown(input, { key: "a", code: "KeyA" });
    fireEvent.keyDown(input, { key: "e", code: "KeyE" });
    fireEvent.keyDown(input, { key: "r", code: "KeyR" });

    // Allow any pending microtasks to settle.
    await new Promise((r) => setTimeout(r, 50));

    // No decide_discovery call should have been made.
    const decideCalls = mockFn.mock.calls.filter((c) => c[0] === "decide_discovery");
    expect(decideCalls.length).toBe(0);

    // The discovery card is still visible (not dismissed or accepted).
    expect(screen.getByText("三选测试发现")).toBeInTheDocument();

    // Cleanup.
    document.body.removeChild(input);
  });
});
