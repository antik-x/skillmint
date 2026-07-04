import { describe, it, expect, vi } from "vitest";
import { invoke } from "./invoke";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke as rawInvoke } from "@tauri-apps/api/core";

describe("invoke wrapper", () => {
  it("returns successful results unchanged", async () => {
    vi.mocked(rawInvoke).mockResolvedValueOnce({ ok: true });
    const result = await invoke("test_cmd");
    expect(result).toEqual({ ok: true });
  });

  it("throws a humanized error with detail attached", async () => {
    vi.mocked(rawInvoke).mockRejectedValueOnce(
      new Error("invoke(foo) → 404: unknown command")
    );
    await expect(invoke("foo")).rejects.toThrow("此功能在当前环境不可用");
  });
});
