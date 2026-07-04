import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "@testing-library/react";
import { GlobalShortcuts } from "./GlobalShortcuts";
import { useAppStore } from "../stores/appStore";
import { invoke } from "@tauri-apps/api/core";
import { resetHotkeys } from "../hooks/useHotkeys";

function isMac(): boolean {
  return navigator.platform.toLowerCase().includes("mac");
}

function dispatch(mod: string, key: string, target: Element = document.body) {
  const event = new KeyboardEvent("keydown", {
    key,
    ...(mod === "Meta" ? { metaKey: true } : { ctrlKey: true }),
    bubbles: true,
    cancelable: true,
    composed: true,
  });
  return target.dispatchEvent(event);
}

describe("GlobalShortcuts", () => {
  beforeEach(() => {
    resetHotkeys();
    useAppStore.setState({
      activeTab: "today",
      skillLibrarySubTab: "skills",
      settingsSubTab: "preferences",
      sidebarVisible: true,
      newSkillRequest: 0,
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("toggles command palette on mod+k", () => {
    const onTogglePalette = vi.fn();
    render(<GlobalShortcuts onTogglePalette={onTogglePalette} onOpenHelp={vi.fn()} />);

    const mod = isMac() ? "Meta" : "Control";
    dispatch(mod, "k");

    expect(onTogglePalette).toHaveBeenCalledTimes(1);
  });

  it("navigates to settings on mod+,", () => {
    render(<GlobalShortcuts onTogglePalette={vi.fn()} onOpenHelp={vi.fn()} />);

    const mod = isMac() ? "Meta" : "Control";
    dispatch(mod, ",");

    expect(useAppStore.getState().activeTab).toBe("settings");
    expect(useAppStore.getState().settingsSubTab).toBe("preferences");
  });

  it("requests new skill on mod+n", () => {
    render(<GlobalShortcuts onTogglePalette={vi.fn()} onOpenHelp={vi.fn()} />);

    const mod = isMac() ? "Meta" : "Control";
    dispatch(mod, "n");

    expect(useAppStore.getState().activeTab).toBe("skillLibrary");
    expect(useAppStore.getState().skillLibrarySubTab).toBe("skills");
    expect(useAppStore.getState().newSkillRequest).toBe(1);
  });

  it("toggles sidebar on mod+b", () => {
    render(<GlobalShortcuts onTogglePalette={vi.fn()} onOpenHelp={vi.fn()} />);

    const mod = isMac() ? "Meta" : "Control";
    dispatch(mod, "b");

    expect(useAppStore.getState().sidebarVisible).toBe(false);
  });

  it("syncs all on mod+shift+s", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({});
    render(<GlobalShortcuts onTogglePalette={vi.fn()} onOpenHelp={vi.fn()} />);

    const mod = isMac() ? "Meta" : "Control";
    const event = new KeyboardEvent("keydown", {
      key: "s",
      [mod === "Meta" ? "metaKey" : "ctrlKey"]: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
      composed: true,
    });
    document.body.dispatchEvent(event);

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(invoke).toHaveBeenCalledWith("sync_all_command");
  });

  it("opens help on mod+/", () => {
    const onOpenHelp = vi.fn();
    render(<GlobalShortcuts onTogglePalette={vi.fn()} onOpenHelp={onOpenHelp} />);

    const mod = isMac() ? "Meta" : "Control";
    const event = new KeyboardEvent("keydown", {
      key: "/",
      [mod === "Meta" ? "metaKey" : "ctrlKey"]: true,
      bubbles: true,
      cancelable: true,
      composed: true,
    });
    document.body.dispatchEvent(event);

    expect(onOpenHelp).toHaveBeenCalledTimes(1);
  });
});
