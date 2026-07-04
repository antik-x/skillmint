import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HelpTip } from "./HelpTip";

describe("SPEC-F6 T2: HelpTip", () => {
  it("默认收起，气泡不显示", () => {
    render(<HelpTip text="解释文案" />);
    expect(screen.queryByTestId("help-tip-bubble")).toBeNull();
  });

  it("hover trigger 后显示气泡，文案为 text", async () => {
    render(<HelpTip text="把中心仓库里的这份 Skill 写入各 Agent 的规则目录" />);
    const trigger = screen.getByTestId("help-tip-trigger");
    await userEvent.hover(trigger);
    const bubble = await screen.findByTestId("help-tip-bubble");
    expect(bubble).toHaveTextContent("把中心仓库里的这份 Skill 写入各 Agent 的规则目录");
  });

  it("键盘聚焦 trigger 显示气泡，Esc 关闭", async () => {
    render(<HelpTip text="键盘可访问" />);
    const trigger = screen.getByTestId("help-tip-trigger");
    trigger.focus();
    const bubble = await screen.findByTestId("help-tip-bubble");
    expect(bubble).toBeInTheDocument();

    await userEvent.keyboard("{Escape}");
    expect(screen.queryByTestId("help-tip-bubble")).toBeNull();
  });

  it("trigger 带 aria-label 与 aria-expanded 状态", () => {
    render(<HelpTip text="x" ariaLabel="什么是同步" />);
    const trigger = screen.getByTestId("help-tip-trigger");
    expect(trigger).toHaveAttribute("aria-label", "什么是同步");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });
});
