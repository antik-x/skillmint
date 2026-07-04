import { useEffect, useRef, useCallback, useSyncExternalStore } from "react";

export type HotkeyHandler = (e: KeyboardEvent) => void | boolean | Promise<void | boolean>;

export interface HotkeyOptions {
  scope?: string;
  description?: string;
  /** When true, the handler fires even if an input/textarea is focused. */
  allowInInput?: boolean;
  /** When true, the handler fires regardless of the current scope stack. */
  global?: boolean;
}

export interface RegisteredHotkey {
  id: string;
  combo: string;
  handler: HotkeyHandler;
  scope: string;
  description?: string;
  allowInInput?: boolean;
  global?: boolean;
}

const registry: RegisteredHotkey[] = [];
let scopeStack: string[] = [];
const listeners = new Set<() => void>();
let shortcutsSnapshot: RegisteredHotkey[] = [];

let globalListenerAttached = false;

updateSnapshot();

function updateSnapshot() {
  shortcutsSnapshot = registry.slice();
}

function emit() {
  updateSnapshot();
  listeners.forEach((l) => l());
}

function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return navigator.platform.toLowerCase().includes("mac");
}

function normalizeKey(key: string): string {
  const k = key.trim().toLowerCase();
  const aliases: Record<string, string> = {
    esc: "escape",
    escape: "escape",
    up: "arrowup",
    down: "arrowdown",
    left: "arrowleft",
    right: "arrowright",
    space: " ",
    spacebar: " ",
    enter: "enter",
    return: "enter",
    tab: "tab",
    del: "delete",
    "delete": "delete",
    backspace: "backspace",
    cmd: "meta",
    command: "meta",
    win: "meta",
    meta: "meta",
    ctrl: "control",
    control: "control",
    alt: "alt",
    option: "alt",
    shift: "shift",
    "/": "/",
    ",": ",",
    ".": ".",
    ";": ";",
    "'": "'",
    "[": "[",
    "]": "]",
    "\\": "\\",
    "=": "=",
    "-": "-",
    "`": "`",
  };
  if (k.length === 1) {
    return k;
  }
  return aliases[k] ?? k;
}

function parseCombo(combo: string): {
  key: string;
  mod: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
} {
  const parts = combo.toLowerCase().split(/[+\-]/).map((p) => p.trim()).filter(Boolean);
  let key = "";
  const mods = { mod: false, ctrl: false, alt: false, shift: false, meta: false };
  for (const part of parts) {
    const normalized = normalizeKey(part);
    if (normalized === "mod") mods.mod = true;
    else if (normalized === "control" || normalized === "ctrl") mods.ctrl = true;
    else if (normalized === "alt" || normalized === "option") mods.alt = true;
    else if (normalized === "shift") mods.shift = true;
    else if (normalized === "meta") mods.meta = true;
    else key = normalized;
  }
  return { key, ...mods };
}

function eventMatches(e: KeyboardEvent, parsed: ReturnType<typeof parseCombo>): boolean {
  const key = normalizeKey(e.key);
  if (key !== parsed.key) return false;

  const mac = isMac();
  const modPressed = mac ? e.metaKey : e.ctrlKey;
  const expectedMod = parsed.mod;
  if (expectedMod !== modPressed) return false;

  if (parsed.ctrl && !e.ctrlKey) return false;
  if (parsed.alt && !e.altKey) return false;
  if (parsed.shift && !e.shiftKey) return false;
  if (parsed.meta && !e.metaKey) return false;

  // Ensure no extra modifiers are pressed (except those explicitly allowed).
  const expectedCtrl = parsed.ctrl || (parsed.mod && !mac);
  const expectedMeta = parsed.meta || (parsed.mod && mac);
  const expectedAlt = parsed.alt;
  const expectedShift = parsed.shift;

  if (e.ctrlKey !== expectedCtrl) return false;
  if (e.metaKey !== expectedMeta) return false;
  if (e.altKey !== expectedAlt) return false;
  if (e.shiftKey !== expectedShift) return false;

  return true;
}

function isTypingTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  const tag = el.tagName.toLowerCase();
  const editable = el.getAttribute("contenteditable") === "true";
  return tag === "input" || tag === "textarea" || tag === "select" || editable;
}

function activeScope(): string {
  return scopeStack.length > 0 ? scopeStack[scopeStack.length - 1] : "global";
}

function handleKeyDown(e: KeyboardEvent) {
  const scope = activeScope();
  const typing = isTypingTarget(e.target);

  // Find the most recently registered matching hotkey in the active scope,
  // or a global hotkey that bypasses the scope stack.
  for (let i = registry.length - 1; i >= 0; i--) {
    const entry = registry[i];
    if (entry.scope !== scope && !entry.global) continue;
    if (!eventMatches(e, parseCombo(entry.combo))) continue;

    // When typing in an input, ignore shortcuts unless explicitly allowed
    // or the shortcut is Esc / command-palette (mod+k).
    const isEsc = normalizeKey(e.key) === "escape";
    const isCommandPalette = entry.combo.toLowerCase() === "mod+k";
    if (typing && !entry.allowInInput && !isEsc && !isCommandPalette) continue;

    const result = entry.handler(e);
    if (result !== false) {
      e.preventDefault();
      e.stopPropagation();
    }
    return;
  }
}

function ensureGlobalListener() {
  if (globalListenerAttached || typeof document === "undefined") return;
  document.addEventListener("keydown", handleKeyDown, true);
  globalListenerAttached = true;
}

function removeGlobalListener() {
  if (!globalListenerAttached || typeof document === "undefined") return;
  document.removeEventListener("keydown", handleKeyDown, true);
  globalListenerAttached = false;
}

export function registerHotkey(
  combo: string,
  handler: HotkeyHandler,
  options: HotkeyOptions = {}
): () => void {
  ensureGlobalListener();
  const id = `${combo}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const entry: RegisteredHotkey = {
    id,
    combo,
    handler,
    scope: options.scope ?? "global",
    description: options.description,
    allowInInput: options.allowInInput,
    global: options.global,
  };
  registry.push(entry);
  emit();

  return () => {
    const idx = registry.findIndex((r) => r.id === id);
    if (idx >= 0) {
      registry.splice(idx, 1);
      emit();
    }
    if (registry.length === 0) {
      removeGlobalListener();
    }
  };
}

export function pushScope(scope: string) {
  scopeStack.push(scope);
  emit();
}

export function popScope(scope?: string) {
  if (scope) {
    const idx = scopeStack.lastIndexOf(scope);
    if (idx >= 0) scopeStack.splice(idx, 1);
  } else {
    scopeStack.pop();
  }
  emit();
}

export function getActiveScope(): string {
  return activeScope();
}

export function getRegisteredShortcuts(): RegisteredHotkey[] {
  return shortcutsSnapshot;
}

export function resetHotkeys() {
  registry.length = 0;
  scopeStack = [];
  updateSnapshot();
  removeGlobalListener();
  emit();
}

// React hook: subscribe to registry changes.
export function useHotkeysRegistry(): RegisteredHotkey[] {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    getRegisteredShortcuts,
    getRegisteredShortcuts
  );
}

// React hook: register a hotkey for the lifetime of the component.
export function useHotkey(
  combo: string,
  handler: HotkeyHandler,
  options?: HotkeyOptions,
  deps: React.DependencyList = []
) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  const wrapped = useCallback(
    (e: KeyboardEvent) => handlerRef.current(e),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    deps
  );

  useEffect(() => {
    return registerHotkey(combo, wrapped, options);
  }, [combo, wrapped, options?.scope, options?.description, options?.allowInInput, options?.global]);
}

// React hook: push a scope for the lifetime of the component.
export function useHotkeyScope(scope: string | undefined) {
  useEffect(() => {
    if (!scope) return;
    pushScope(scope);
    return () => popScope(scope);
  }, [scope]);
}
