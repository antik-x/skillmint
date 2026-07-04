import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ErrorBoundary } from "./ErrorBoundary";

function Bomb({ shouldThrow }: { shouldThrow: boolean }) {
  if (shouldThrow) {
    throw new Error("boom");
  }
  return <div>ok</div>;
}

describe("ErrorBoundary", () => {
  it("renders children when there is no error", () => {
    render(
      <ErrorBoundary>
        <div>content</div>
      </ErrorBoundary>
    );
    expect(screen.getByText("content")).toBeInTheDocument();
  });

  it("renders fallback UI when child throws", () => {
    // Suppress expected React error log noise.
    vi.spyOn(console, "error").mockImplementation(() => {});
    render(
      <ErrorBoundary>
        <Bomb shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(screen.getByText("页面出错了")).toBeInTheDocument();
    expect(screen.getByText("重新加载")).toBeInTheDocument();
  });
});
