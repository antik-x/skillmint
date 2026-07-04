import { describe, it, expect, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useListNavigation } from "./useListNavigation";

describe("useListNavigation", () => {
  it("initializes with the provided index", () => {
    const { result } = renderHook(() =>
      useListNavigation({ items: ["a", "b", "c"], initialIndex: 1 })
    );
    expect(result.current.activeIndex).toBe(1);
  });

  it("moves active index with arrow keys", () => {
    const onSelect = vi.fn();
    const { result } = renderHook(() =>
      useListNavigation({ items: ["a", "b", "c"], onSelect })
    );

    act(() => {
      result.current.handleKeyDown({ key: "ArrowDown", preventDefault: vi.fn() } as unknown as React.KeyboardEvent);
    });
    expect(result.current.activeIndex).toBe(0);

    act(() => {
      result.current.handleKeyDown({ key: "ArrowDown", preventDefault: vi.fn() } as unknown as React.KeyboardEvent);
    });
    expect(result.current.activeIndex).toBe(1);

    act(() => {
      result.current.handleKeyDown({ key: "ArrowUp", preventDefault: vi.fn() } as unknown as React.KeyboardEvent);
    });
    expect(result.current.activeIndex).toBe(0);
  });

  it("does not move past boundaries by default", () => {
    const { result } = renderHook(() =>
      useListNavigation({ items: ["a", "b"] })
    );

    act(() => {
      result.current.handleKeyDown({ key: "ArrowUp", preventDefault: vi.fn() } as unknown as React.KeyboardEvent);
    });
    expect(result.current.activeIndex).toBe(0);

    act(() => {
      result.current.setActiveIndex(1);
    });
    act(() => {
      result.current.handleKeyDown({ key: "ArrowDown", preventDefault: vi.fn() } as unknown as React.KeyboardEvent);
    });
    expect(result.current.activeIndex).toBe(1);
  });

  it("cycles when enabled", () => {
    const { result } = renderHook(() =>
      useListNavigation({ items: ["a", "b"], cycle: true })
    );

    act(() => {
      result.current.setActiveIndex(1);
    });
    act(() => {
      result.current.handleKeyDown({ key: "ArrowDown", preventDefault: vi.fn() } as unknown as React.KeyboardEvent);
    });
    expect(result.current.activeIndex).toBe(0);
  });

  it("calls onSelect on Enter", () => {
    const onSelect = vi.fn();
    const { result } = renderHook(() =>
      useListNavigation({ items: ["a", "b", "c"], onSelect, initialIndex: 1 })
    );

    act(() => {
      result.current.handleKeyDown({ key: "Enter", preventDefault: vi.fn() } as unknown as React.KeyboardEvent);
    });

    expect(onSelect).toHaveBeenCalledWith("b", 1);
  });

  it("exposes listbox aria attributes", () => {
    const { result } = renderHook(() =>
      useListNavigation({ items: ["a", "b"], initialIndex: 0 })
    );

    expect(result.current.listProps.role).toBe("listbox");
    expect(result.current.listProps["aria-activedescendant"]).toMatch(/-item-0$/);
    expect(result.current.getItemProps(0).role).toBe("option");
    expect(result.current.getItemProps(0)["aria-selected"]).toBe(true);
    expect(result.current.getItemProps(1)["aria-selected"]).toBe(false);
  });
});
