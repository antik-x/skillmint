import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import Inbox from "./Inbox";
import { useAppStore } from "../stores/appStore";
import { resetHotkeys } from "../hooks/useHotkeys";
import type { Discovery } from "../types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function makeDiscovery(overrides: Partial<Discovery> = {}): Discovery {
  return {
    id: `d-${overrides.id ?? Math.random().toString(36).slice(2)}`,
    kind: "repeat_pattern",
    title: "测试发现",
    payload: {
      evidence: [
        { session_id: "s1", prompt_text: "第一条证据", started_at: 1750000000, source: "cursor", project: "hub" },
        { session_id: "s2", prompt_text: "第二条证据", started_at: 1750000100, source: "claude-code", project: "api" },
        { session_id: "s3", prompt_text: "第三条证据", started_at: 1750000200, source: "zcode", project: "app" },
      ],
      draft_skill: { name: "测试 Skill", body: "草稿正文" },
    },
    confidence: 0.92,
    status: "pending",
    created_at: 1750000000,
    ...overrides,
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
}

describe("Inbox page", () => {
  beforeEach(() => {
    resetHotkeys();
    resetStore();
    vi.mocked(invoke).mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders pending discovery card with meta", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    expect(await screen.findByText("收件箱 · 1 / 1")).toBeInTheDocument();
    expect(await screen.findByText("测试发现")).toBeInTheDocument();
    expect(screen.getByText("AI 已备好草稿")).toBeInTheDocument();
  });

  it("expands and collapses evidence with button and space", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    await screen.findByText("测试发现");
    expect(screen.queryByText("第三条证据")).not.toBeInTheDocument();

    await userEvent.click(screen.getByText(/展开其余 1 条证据/));
    expect(await screen.findByText("第三条证据")).toBeInTheDocument();

    fireEvent.keyDown(document, { key: " ", code: "Space" });
    await waitFor(() => expect(screen.queryByText("第三条证据")).not.toBeInTheDocument());
  });

  it("navigates cards with arrow keys", async () => {
    const d1 = makeDiscovery({ id: "1", title: "第一条" });
    const d2 = makeDiscovery({ id: "2", title: "第二条" });
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([d1, d2]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    await screen.findByText("第一条");
    fireEvent.keyDown(document, { key: "ArrowRight", code: "ArrowRight" });
    expect(await screen.findByText("第二条")).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "ArrowLeft", code: "ArrowLeft" });
    expect(await screen.findByText("第一条")).toBeInTheDocument();
  });

  it("accepts a repeat_pattern (adopt-and-sync) and card leaves on success", async () => {
    const discovery = makeDiscovery({ id: "1", kind: "repeat_pattern" });
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([discovery]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "accept") {
        return Promise.resolve({ created_skill_id: "skill-123", sync_summary: { success: 0, failed: 0, failures: [] } });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("测试发现");

    fireEvent.keyDown(document, { key: "Enter", code: "Enter" });
    // Card leaves (accept-and-sync does NOT navigate to editor).
    await waitFor(() => expect(screen.queryByText("测试发现")).not.toBeInTheDocument(), { timeout: 2000 });
    // Editor is NOT opened (that's accept_edited's job now).
    expect(useAppStore.getState().selectedSkillId).toBeNull();
  });

  it("dismisses with reason picker + number key and shows completion when empty", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "dismiss") {
        return Promise.resolve({ created_skill_id: null });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("测试发现");

    // SPEC-C2 T1/T3: Backspace opens the reason picker for repeat_pattern.
    fireEvent.keyDown(document, { key: "Backspace", code: "Backspace" });
    expect(await screen.findByTestId("dismiss-reasons")).toBeInTheDocument();
    // Press "2" = 太琐碎 (trivial).
    fireEvent.keyDown(document, { key: "2", code: "Digit2" });
    expect(await screen.findByText("今日裁决完成")).toBeInTheDocument();
    const stats = screen.getByTestId("inbox-completion-stats");
    expect(stats.textContent).toContain("沉淀 0");
    expect(stats.textContent).toContain("拒绝 1");
  });

  it("returns to today on escape (after closing reason picker first)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    await screen.findByText("测试发现");
    // Open the reason picker.
    fireEvent.keyDown(document, { key: "Backspace", code: "Backspace" });
    await screen.findByTestId("dismiss-reasons");
    // First Esc closes the picker, not the page.
    fireEvent.keyDown(document, { key: "Escape", code: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("dismiss-reasons")).not.toBeInTheDocument());
    expect(useAppStore.getState().activeTab).not.toBe("today");
    // Second Esc leaves the page.
    fireEvent.keyDown(document, { key: "Escape", code: "Escape" });
    await waitFor(() => expect(useAppStore.getState().activeTab).toBe("today"));
  });

  it("rolls back dismiss on error", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: InvokeArgs) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery" && (args as { action: string }).action === "dismiss") {
        return Promise.reject(new Error("network error"));
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    render(<Inbox />);
    await screen.findByText("测试发现");
    // Open reason picker, pick a reason.
    fireEvent.keyDown(document, { key: "Backspace", code: "Backspace" });
    await screen.findByTestId("dismiss-reasons");
    fireEvent.keyDown(document, { key: "1", code: "Digit1" });
    // On error the card stays (no completion screen).
    await waitFor(() => expect(screen.queryByText("今日裁决完成")).not.toBeInTheDocument());
    expect(screen.getByText("测试发现")).toBeInTheDocument();
  });

  it("degrades mock accept to dismiss", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_discoveries") return Promise.resolve([makeDiscovery({ id: "1" })]);
      if (cmd === "decide_discovery") return Promise.resolve({ created_skill_id: null, mock: true });
      return Promise.resolve(undefined);
    });
    render(<Inbox />);
    await screen.findByText("测试发现");
    fireEvent.keyDown(document, { key: "Enter", code: "Enter" });
    expect(await screen.findByText("今日裁决完成")).toBeInTheDocument();
    expect(screen.getByTestId("inbox-completion-stats").textContent).toContain("拒绝 1");
  });
});
