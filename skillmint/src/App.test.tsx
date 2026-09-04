import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { useAppStore } from "./stores/appStore";
import App from "./App";
import type { AppSettings } from "./types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

// 捕获 listen 注册的事件回调，测试里手动触发。vi.hoisted 保证它在 mock
// 工厂与依赖模块（collectionStore 在创建时即调用 listen）之前初始化。
type EventHandler = (event: { payload: unknown }) => void;
const handlers = vi.hoisted(() => new Map<string, EventHandler>());

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: EventHandler) => {
    handlers.set(name, cb);
    return Promise.resolve(() => {
      handlers.delete(name);
    });
  }),
  emit: vi.fn(() => Promise.resolve()),
}));

const settingsObj: AppSettings = {
  device_id: "test-device",
  auto_sync_interval_minutes: 0,
  launch_at_login: false,
  show_dock_icon: true,
  onboarding_completed: true,
  remote_enabled: false,
  theme: "system",
  ai: { models: [], acp_connections: [], prefer_acp: false, strict_local_mode: false },
};

describe("App deep-link / tray 路由", () => {
  beforeEach(() => {
    handlers.clear();
    vi.mocked(invoke).mockClear();
    vi.mocked(listen).mockClear();
    vi.mocked(emit).mockClear();
    useAppStore.setState({
      initialized: false,
      activeTab: "today",
      skills: [],
      agents: [],
      syncTargets: [],
      settings: settingsObj,
    });
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "init_app") return Promise.resolve(null);
      if (cmd === "get_settings") return Promise.resolve(settingsObj);
      if (cmd === "check_repo_integrity") return Promise.resolve("healthy");
      if (
        [
          "get_skills",
          "get_agents",
          "get_sync_targets",
          "list_discoveries",
          "get_skill_usage",
          "list_daily_summaries",
          "get_collection_status",
        ].includes(cmd)
      ) {
        return Promise.resolve([]);
      }
      return Promise.resolve(null);
    });
  });

  it("挂载后注册监听并发出 app-ready（P1-1）", async () => {
    render(<App />);
    await waitFor(() => expect(useAppStore.getState().initialized).toBe(true));
    await waitFor(() => expect(handlers.has("tray-navigate")).toBe(true));
    expect(handlers.has("deep-link-sync")).toBe(true);
    expect(handlers.has("deep-link-open-skill")).toBe(true);
    expect(vi.mocked(emit)).toHaveBeenCalledWith("app-ready");
  });

  it("tray-navigate: Skill 库 → skillLibrary；同步健康 → today（P1-2）", async () => {
    render(<App />);
    await waitFor(() => expect(handlers.has("tray-navigate")).toBe(true));

    await act(async () => {
      handlers.get("tray-navigate")!({ payload: "skillLibrary" });
    });
    expect(useAppStore.getState().activeTab).toBe("skillLibrary");

    await act(async () => {
      handlers.get("tray-navigate")!({ payload: "today" });
    });
    expect(useAppStore.getState().activeTab).toBe("today");
  });

  it("tray-navigate: 未知 payload 不改路由", async () => {
    render(<App />);
    await waitFor(() => expect(handlers.has("tray-navigate")).toBe(true));

    await act(async () => {
      handlers.get("tray-navigate")!({ payload: "nonsense" });
    });
    expect(useAppStore.getState().activeTab).toBe("today");
  });
});
