import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  registerHotkey,
  pushScope,
  popScope,
  resetHotkeys,
  getActiveScope,
  getRegisteredShortcuts,
} from "./useHotkeys";

function createKeydown(options: Partial<KeyboardEventInit>): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    composed: true,
    ...options,
  });
}

function dispatch(event: KeyboardEvent, target: Element = document.body): boolean {
  return target.dispatchEvent(event);
}

function isMac(): boolean {
  return navigator.platform.toLowerCase().includes("mac");
}

function createModKeydown(key: string): KeyboardEvent {
  const modKey = isMac() ? "Meta" : "Control";
  return createKeydown({
    key,
    ...(modKey === "Meta" ? { metaKey: true } : { ctrlKey: true }),
  });
}

describe("useHotkeys registry", () => {
  beforeEach(() => {
    resetHotkeys();
  });

  afterEach(() => {
    resetHotkeys();
  });

  it("registers and triggers mod+k", () => {
    const handler = vi.fn();
    registerHotkey("mod+k", handler);

    const e = createModKeydown("k");
    dispatch(e);

    expect(handler).toHaveBeenCalledTimes(1);
    expect(e.defaultPrevented).toBe(true);
  });

  it("does not trigger mod+n when an input is focused", () => {
    const handler = vi.fn();
    registerHotkey("mod+n", handler);

    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    const e = createModKeydown("n");
    dispatch(e, input);

    expect(handler).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false);

    document.body.removeChild(input);
  });

  it("allows mod+k even when an input is focused", () => {
    const handler = vi.fn();
    registerHotkey("mod+k", handler);

    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    const e = createModKeydown("k");
    dispatch(e, input);

    expect(handler).toHaveBeenCalledTimes(1);
    document.body.removeChild(input);
  });

  it("allows Esc even when an input is focused", () => {
    const handler = vi.fn();
    registerHotkey("esc", handler);

    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    const e = createKeydown({ key: "Escape" });
    dispatch(e, input);

    expect(handler).toHaveBeenCalledTimes(1);
    document.body.removeChild(input);
  });

  it("respects scope stack isolation", () => {
    const globalHandler = vi.fn();
    const modalHandler = vi.fn();
    registerHotkey("esc", globalHandler, { scope: "global" });
    registerHotkey("esc", modalHandler, { scope: "modal" });

    pushScope("modal");
    const e = createKeydown({ key: "Escape" });
    dispatch(e);

    expect(modalHandler).toHaveBeenCalledTimes(1);
    expect(globalHandler).not.toHaveBeenCalled();

    popScope("modal");
    const e2 = createKeydown({ key: "Escape" });
    dispatch(e2);

    expect(globalHandler).toHaveBeenCalledTimes(1);
  });

  it("lets global:true hotkeys fire inside any scope", () => {
    const handler = vi.fn();
    registerHotkey("mod+k", handler, { global: true });

    pushScope("editor");
    const e = createModKeydown("k");
    dispatch(e);

    expect(handler).toHaveBeenCalledTimes(1);
    popScope("editor");
  });

  it("returns the active scope", () => {
    expect(getActiveScope()).toBe("global");
    pushScope("modal");
    expect(getActiveScope()).toBe("modal");
    popScope();
    expect(getActiveScope()).toBe("global");
  });

  it("unregisters via returned cleanup", () => {
    const handler = vi.fn();
    const cleanup = registerHotkey("mod+r", handler);
    cleanup();

    const e = createModKeydown("r");
    dispatch(e);

    expect(handler).not.toHaveBeenCalled();
    expect(getRegisteredShortcuts()).toHaveLength(0);
  });

  it("normalizes mod to Meta on Mac and Ctrl elsewhere", () => {
    const handler = vi.fn();
    registerHotkey("mod+k", handler);

    if (isMac()) {
      const ctrl = createKeydown({ key: "k", ctrlKey: true });
      dispatch(ctrl);
      expect(handler).not.toHaveBeenCalled();

      dispatch(createModKeydown("k"));
      expect(handler).toHaveBeenCalledTimes(1);
    } else {
      const meta = createKeydown({ key: "k", metaKey: true });
      dispatch(meta);
      expect(handler).not.toHaveBeenCalled();

      dispatch(createModKeydown("k"));
      expect(handler).toHaveBeenCalledTimes(1);
    }
  });
});
