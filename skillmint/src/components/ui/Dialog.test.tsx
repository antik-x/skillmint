import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Dialog } from "./Dialog";
import { resetHotkeys } from "../../hooks/useHotkeys";

describe("Dialog", () => {
  beforeEach(() => {
    resetHotkeys();
  });

  it("closes on Escape", async () => {
    const onClose = vi.fn();
    render(
      <Dialog open onClose={onClose} title="Test">
        <button>Action</button>
      </Dialog>
    );

    const dialog = screen.getByRole("dialog");
    dialog.focus();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("traps Tab focus", async () => {
    render(
      <Dialog open onClose={vi.fn()} title="Test">
        <button data-testid="first">First</button>
        <button data-testid="second">Second</button>
      </Dialog>
    );

    // Wait for the auto-focus rAF to complete.
    await new Promise((resolve) => requestAnimationFrame(resolve));

    const first = screen.getByTestId("first");
    const second = screen.getByTestId("second");

    expect(document.activeElement).toBe(first);
    await userEvent.tab();
    expect(document.activeElement).toBe(second);
    await userEvent.tab();
    expect(document.activeElement).toBe(first);
  });

  it("focuses the first focusable element on open", async () => {
    render(
      <Dialog open onClose={vi.fn()} title="Test">
        <input data-testid="input" />
        <button>Submit</button>
      </Dialog>
    );

    // Wait for requestAnimationFrame.
    await new Promise((resolve) => requestAnimationFrame(resolve));
    expect(document.activeElement).toBe(screen.getByTestId("input"));
  });

  it("restores focus on close", async () => {
    const button = document.createElement("button");
    button.textContent = "Trigger";
    document.body.appendChild(button);
    button.focus();

    const { rerender } = render(
      <Dialog open onClose={vi.fn()} title="Test">
        <button>Inside</button>
      </Dialog>
    );

    rerender(
      <Dialog open={false} onClose={vi.fn()} title="Test">
        <button>Inside</button>
      </Dialog>
    );

    expect(document.activeElement).toBe(button);
    document.body.removeChild(button);
  });

  it("calls onSubmit on Enter when enabled", async () => {
    const onSubmit = vi.fn();
    render(
      <Dialog open onClose={vi.fn()} onSubmit={onSubmit} title="Test">
        <input data-testid="input" />
      </Dialog>
    );

    await userEvent.type(screen.getByTestId("input"), "{Enter}");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("has dialog role and aria-modal", () => {
    render(
      <Dialog open onClose={vi.fn()} title="Test">
        content
      </Dialog>
    );

    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveAttribute("aria-labelledby");
  });
});
