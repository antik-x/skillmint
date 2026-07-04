import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import PreferencesPanel from "./PreferencesPanel";
import { useAppStore } from "../stores/appStore";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  save: vi.fn(),
}));

function resetStore() {
  useAppStore.setState({
    settings: {
      device_id: "dev-1",
      center_repo: "",
      default_sync_mode: "symlink",
      auto_sync_interval_minutes: 0,
      launch_at_login: false,
      show_dock_icon: true,
      onboarding_completed: true,
      skill_scope_mode: "global",
      project_skill_dir_name: ".skillmint/skills",
      remote_enabled: false,
      theme: "system",
      ai: { models: [], acp_connections: [], prefer_acp: false, strict_local_mode: false },
    },
  });
}

describe("PreferencesPanel theme", () => {
  beforeEach(() => {
    resetStore();
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_snapshots" || cmd === "git_versions" || cmd === "git_status_inited") {
        return Promise.resolve([]);
      }
      return Promise.resolve(undefined);
    });
    delete document.documentElement.dataset.theme;
  });

  it("switches theme to dark and persists via save_settings", async () => {
    vi.mocked(invoke).mockResolvedValue({
      ...useAppStore.getState().settings,
      theme: "dark",
    });
    render(<PreferencesPanel />);
    const select = screen.getByRole("combobox", { name: /外观/ });
    await userEvent.selectOptions(select, "dark");
    expect(document.documentElement.dataset.theme).toBe("dark");

    await userEvent.click(screen.getByRole("button", { name: /保存设置/ }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("save_settings", expect.any(Object)));
  });

  it("removes data-theme when system is selected", async () => {
    document.documentElement.dataset.theme = "dark";
    render(<PreferencesPanel />);
    const select = screen.getByRole("combobox", { name: /外观/ });
    await userEvent.selectOptions(select, "system");
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});


