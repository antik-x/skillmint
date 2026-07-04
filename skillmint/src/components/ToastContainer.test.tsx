import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, cleanup } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ToastContainer from "./ToastContainer";
import { useToastStore, showError, showSuccess } from "../stores/toastStore";

describe("SPEC-F6 T3: Toast 渲染 action（恢复路径）", () => {
  beforeEach(() => {
    useToastStore.setState({ toasts: [] });
  });
  afterEach(() => cleanup());

  it("showError(unknown) 渲染 RULES 推导出的 action 文案按钮", async () => {
    render(<ToastContainer />);
    act(() => {
      showError(new Error("Skill 'x' already exists"), { context: "创建 Skill" });
    });
    // action 文案应作为按钮 label 出现。
    const actionBtn = await screen.findByText(/换一个名称|删除/);
    expect(actionBtn).toBeInTheDocument();
  });

  it("showSuccess 带 action 时渲染动作按钮且可点击", async () => {
    const onClick = vi.fn();
    render(<ToastContainer />);
    act(() => {
      showSuccess("成功导入 1 个 Skill", { label: "查看导入的 Skill", onClick });
    });
    const btn = await screen.findByText("查看导入的 Skill");
    await userEvent.click(btn);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("invoke wrapper 携带的 action 不被 showError 吞掉", async () => {
    // 模拟 invoke wrapper 抛出的 already-humanized 错误（带 detail + action）。
    const wrapped = new Error("存在同名 Skill。");
    (wrapped as Error & { detail: string }).detail = "Skill 'x' already exists";
    (wrapped as Error & { action?: string }).action = "换一个名称，或先删除/恢复同名 Skill";

    render(<ToastContainer />);
    act(() => {
      showError(wrapped);
    });
    const actionBtn = await screen.findByText("换一个名称，或先删除/恢复同名 Skill");
    expect(actionBtn).toBeInTheDocument();
  });
});
