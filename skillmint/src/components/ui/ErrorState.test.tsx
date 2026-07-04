import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ErrorState } from "./ErrorState";

describe("ErrorState", () => {
  it("renders title and message", () => {
    render(<ErrorState title="出错了" message="无法加载数据" />);
    expect(screen.getByRole("alert")).toHaveTextContent("出错了");
    expect(screen.getByText("无法加载数据")).toBeInTheDocument();
  });

  it("hides detail by default and reveals on click", () => {
    render(<ErrorState message="失败" detail="raw error text" />);
    expect(screen.queryByText("raw error text")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("技术详情"));
    expect(screen.getByText("raw error text")).toBeInTheDocument();
  });

  it("calls onRetry when retry button clicked", () => {
    const onRetry = vi.fn();
    render(<ErrorState message="失败" onRetry={onRetry} />);
    fireEvent.click(screen.getByText("重试"));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});
