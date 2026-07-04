import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { ShortcutsHelp } from "./ShortcutsHelp";
import { registerHotkey, resetHotkeys } from "../hooks/useHotkeys";

describe("ShortcutsHelp", () => {
  beforeEach(() => {
    resetHotkeys();
  });

  it("lists registered shortcuts with descriptions", () => {
    registerHotkey("mod+k", vi.fn(), { description: "命令面板" });
    registerHotkey("mod+n", vi.fn(), { description: "新建 Skill" });

    render(<ShortcutsHelp open onClose={vi.fn()} />);

    expect(screen.getByText("命令面板")).toBeInTheDocument();
    expect(screen.getByText("新建 Skill")).toBeInTheDocument();
  });

  it("renders empty state when no shortcuts are registered", () => {
    render(<ShortcutsHelp open onClose={vi.fn()} />);
    expect(screen.getByText("暂无已注册快捷键。")).toBeInTheDocument();
  });
});
