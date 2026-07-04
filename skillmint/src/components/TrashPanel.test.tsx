import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import TrashPanel from "./TrashPanel";
import type { TrashItem } from "../types";

// Mock the invoke wrapper so we control command responses.
vi.mock("../lib/invoke", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "../lib/invoke";

const DAY = 86400;

function makeItem(overrides: Partial<TrashItem> = {}): TrashItem {
  const now = Math.floor(Date.now() / 1000);
  return {
    id: 1,
    item_type: "skill",
    original_id: "skill-1",
    original_name: "DeletedSkill",
    snapshot_path: "/tmp/snap",
    metadata: {},
    deleted_at: now - 5 * DAY,
    expires_at: now + 25 * DAY,
    ...overrides,
  };
}

describe("TrashPanel (SPEC-C3 T4)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders empty state when trash is empty", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    render(<TrashPanel />);
    expect(await screen.findByText("回收站是空的")).toBeInTheDocument();
  });

  it("renders list items with name, deleted date, and remaining days", async () => {
    const now = Math.floor(Date.now() / 1000);
    vi.mocked(invoke).mockResolvedValue([
      makeItem({
        id: 10,
        original_name: "MySkill",
        deleted_at: now - 2 * DAY,
        expires_at: now + 28 * DAY,
      }),
    ]);
    render(<TrashPanel />);
    expect(await screen.findByText("MySkill")).toBeInTheDocument();
    // Remaining days: 28 days (ceil of (expires*1000 - now_ms)/DAY_MS).
    expect(screen.getByText(/剩余 28 天/)).toBeInTheDocument();
    // Both action buttons present.
    expect(screen.getByRole("button", { name: /恢复/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /彻底删除/ })).toBeInTheDocument();
  });

  it("restore with no clash calls restore_trash_item with null strategy and reloads", async () => {
    const item = makeItem({ id: 20, original_name: "CleanRestore" });
    // list_trash_items returns the item first, then [] after reload.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_trash_items") return Promise.resolve([item]);
      if (cmd === "restore_trash_item") {
        return Promise.resolve({ restored_id: "skill-1", skipped_bindings: [], final_name: "CleanRestore" });
      }
      return Promise.resolve(undefined);
    });
    render(<TrashPanel />);
    const restoreBtn = await screen.findByRole("button", { name: /恢复/ });
    await userEvent.click(restoreBtn);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("restore_trash_item", { id: 20, conflictStrategy: null });
    });
  });

  it("restore_conflict surfaces the three-way picker dialog", async () => {
    const item = makeItem({ id: 30, original_name: "ClashSkill" });
    const conflictErr = new Error("操作失败，请重试。") as Error & { detail?: string };
    conflictErr.detail = "restore_conflict: 名为 'ClashSkill' 的 Skill 已存在，请选择覆盖/重命名策略";
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_trash_items") return Promise.resolve([item]);
      if (cmd === "restore_trash_item") return Promise.reject(conflictErr);
      return Promise.resolve(undefined);
    });
    render(<TrashPanel />);
    const restoreBtn = await screen.findByRole("button", { name: /恢复/ });
    await userEvent.click(restoreBtn);
    // Conflict dialog should appear with the three options.
    expect(await screen.findByText("恢复时遇到重名")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /重命名恢复/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /覆盖现有/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^取消$/ })).toBeInTheDocument();
  });

  it("conflict rename strategy calls restore with 'rename'", async () => {
    const item = makeItem({ id: 40, original_name: "RenameMe" });
    const conflictErr = new Error("操作失败，请重试。") as Error & { detail?: string };
    conflictErr.detail = "restore_conflict: 已存在";
    let restoreCallCount = 0;
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_trash_items") return Promise.resolve([item]);
      if (cmd === "restore_trash_item") {
        restoreCallCount++;
        if (restoreCallCount === 1) return Promise.reject(conflictErr);
        return Promise.resolve({ restored_id: "new", skipped_bindings: [], final_name: "RenameMe-restored" });
      }
      return Promise.resolve(undefined);
    });
    render(<TrashPanel />);
    await userEvent.click(await screen.findByRole("button", { name: /恢复/ }));
    const renameBtn = await screen.findByRole("button", { name: /重命名恢复/ });
    await userEvent.click(renameBtn);
    // The second restore call must use the rename strategy. The component
    // reloads (list_trash_items) afterwards, so we check the call was made
    // rather than asserting it was the very last invoke.
    await waitFor(() => {
      expect(restoreCallCount).toBe(2);
    });
    const restoreCalls = vi.mocked(invoke).mock.calls.filter((c) => c[0] === "restore_trash_item");
    expect(restoreCalls[1]).toEqual(["restore_trash_item", { id: 40, conflictStrategy: "rename" }]);
  });

  it("conflict cancel closes the dialog without a second restore call", async () => {
    const item = makeItem({ id: 50, original_name: "CancelMe" });
    const conflictErr = new Error("操作失败，请重试。") as Error & { detail?: string };
    conflictErr.detail = "restore_conflict: 已存在";
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_trash_items") return Promise.resolve([item]);
      if (cmd === "restore_trash_item") return Promise.reject(conflictErr);
      return Promise.resolve(undefined);
    });
    render(<TrashPanel />);
    await userEvent.click(await screen.findByRole("button", { name: /恢复/ }));
    const cancelBtn = await screen.findByRole("button", { name: /^取消$/ });
    await userEvent.click(cancelBtn);
    await waitFor(() => {
      expect(screen.queryByText("恢复时遇到重名")).not.toBeInTheDocument();
    });
    // Only one restore call (the initial one); cancel must not trigger another.
    const restoreCalls = vi.mocked(invoke).mock.calls.filter((c) => c[0] === "restore_trash_item");
    expect(restoreCalls.length).toBe(1);
  });

  it("purge opens heavy-confirm dialog and requires the confirm code", async () => {
    const item = makeItem({ id: 60, original_name: "PurgeTarget" });
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_trash_items") return Promise.resolve([item]);
      if (cmd === "purge_trash_item") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    render(<TrashPanel />);
    await userEvent.click(await screen.findByRole("button", { name: /彻底删除/ }));
    expect(await screen.findByText(/此操作不可逆/)).toBeInTheDocument();
    // Confirm button exists but purge is not called until code is entered.
    const confirmBtn = screen.getByRole("button", { name: /确认彻底删除/ });
    // Clicking without the code shows an error, not a call.
    await userEvent.click(confirmBtn);
    expect(invoke).not.toHaveBeenCalledWith("purge_trash_item", expect.anything());
  });

  it("purge with correct confirm code calls purge_trash_item", async () => {
    const item = makeItem({ id: 70, original_name: "PurgeOk" });
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_trash_items") return Promise.resolve([item]);
      if (cmd === "purge_trash_item") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    render(<TrashPanel />);
    await userEvent.click(await screen.findByRole("button", { name: /彻底删除/ }));
    const input = await screen.findByPlaceholderText("DELETE");
    await userEvent.type(input, "DELETE");
    await userEvent.click(screen.getByRole("button", { name: /确认彻底删除/ }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("purge_trash_item", { id: 70, confirm: "DELETE" });
    });
  });
});
