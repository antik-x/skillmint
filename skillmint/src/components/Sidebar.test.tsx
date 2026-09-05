import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import Sidebar from "./Sidebar";
import { useAppStore } from "../stores/appStore";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function resetStore() {
  useAppStore.setState({
    initialized: true,
    activeTab: "today",
    sidebarVisible: true,
    agents: [],
    skills: [],
  });
}

describe("Sidebar", () => {
  beforeEach(() => {
    resetStore();
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation(() => Promise.resolve(undefined));
  });

  it("renders new M1 navigation groups", () => {
    render(<Sidebar />);
    expect(screen.getByText("今天")).toBeInTheDocument();
    expect(screen.getByText("收件箱")).toBeInTheDocument();
    expect(screen.getByText("成长资产")).toBeInTheDocument();
    expect(screen.getByText("Skill 库")).toBeInTheDocument();
    expect(screen.getByText("知识图谱")).toBeInTheDocument();
    expect(screen.getByText("发现")).toBeInTheDocument();
    expect(screen.getByText("项目")).toBeInTheDocument();
    expect(screen.getByText("Agent")).toBeInTheDocument();
    expect(screen.getByText("洞察档案")).toBeInTheDocument();
    expect(screen.getByText("周报")).toBeInTheDocument();
  });

  it("switches active tab on click", async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByText("收件箱"));
    expect(useAppStore.getState().activeTab).toBe("inbox");
  });

  it("selects daily summaries without redirecting to today", async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByText("每日摘要"));
    expect(useAppStore.getState().activeTab).toBe("dailySummaries");
  });

  it("shows amber badge for pending discoveries", async () => {
    vi.mocked(invoke).mockResolvedValue([
      { id: "d1", status: "pending" },
      { id: "d2", status: "pending" },
    ]);
    render(<Sidebar />);
    await waitFor(() => expect(screen.getByText("2")).toBeInTheDocument());
  });

  it("hides badge when list_discoveries fails", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("404: unknown command"));
    render(<Sidebar />);
    await waitFor(() => expect(screen.queryByText(/^[0-9]+$/)).not.toBeInTheDocument());
  });
});
