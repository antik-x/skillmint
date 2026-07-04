import { create } from "zustand";
import { humanizeError, type HumanizedError } from "../lib/errors";

export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface Toast {
  id: string;
  message: string;
  type: "success" | "error" | "info";
  duration?: number;
  action?: ToastAction;
  /** M1: optional technical detail shown behind a collapsible toggle. */
  detail?: string;
}

interface ToastState {
  toasts: Toast[];
  addToast: (toast: Omit<Toast, "id">) => void;
  removeToast: (id: string) => void;
}

export const useToastStore = create<ToastState>((set) => ({
  toasts: [],
  addToast: (toast) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
    set((state) => ({ toasts: [...state.toasts, { ...toast, id }] }));

    const duration = toast.duration ?? 3000;
    if (duration > 0) {
      setTimeout(() => {
        set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
      }, duration);
    }
  },
  removeToast: (id) =>
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
}));

export function showSuccess(message: string, duration?: number): void;
export function showSuccess(message: string, action: ToastAction, duration?: number): void;
export function showSuccess(
  message: string,
  durationOrAction?: number | ToastAction,
  maybeDuration?: number
) {
  if (durationOrAction && typeof durationOrAction === "object") {
    useToastStore.getState().addToast({
      message,
      type: "success",
      action: durationOrAction,
      duration: maybeDuration,
    });
    return;
  }
  useToastStore.getState().addToast({ message, type: "success", duration: durationOrAction });
}

export function showError(message: string, duration?: number): void;
export function showError(err: unknown, context?: { context?: string; action?: string; duration?: number }): void;
export function showError(
  messageOrErr: string | unknown,
  durationOrContext?: number | { context?: string; action?: string; duration?: number }
) {
  if (typeof messageOrErr === "string") {
    useToastStore.getState().addToast({
      message: messageOrErr,
      type: "error",
      duration: typeof durationOrContext === "number" ? durationOrContext : 5000,
    });
    return;
  }

  // SPEC-F2 T2: already-humanized errors from the invoke wrapper carry detail.
  const alreadyHumanized =
    messageOrErr instanceof Error && "detail" in messageOrErr && typeof messageOrErr.detail === "string";

  const ctx = typeof durationOrContext === "object" ? durationOrContext : {};
  // SPEC-F6 T3: 调用方传 context.action 优先；否则沿用 invoke wrapper 携带的 action。
  const { message, detail, action } = alreadyHumanized
    ? {
        message: messageOrErr.message,
        detail: (messageOrErr as Error & { detail: string }).detail,
        action: ctx.action ?? (messageOrErr as Error & { action?: string }).action,
      }
    : humanizeError(messageOrErr, ctx);
  useToastStore.getState().addToast({
    message,
    type: "error",
    duration: ctx.duration ?? 5000,
    detail,
    action: action
      ? {
          label: action,
          onClick: () => {
            // Default recovery action is a no-op; callers can supply their own action.
          },
        }
      : undefined,
  });
}

export function showInfo(message: string, duration?: number) {
  useToastStore.getState().addToast({ message, type: "info", duration });
}

export function showInfoWithAction(
  message: string,
  action: ToastAction,
  duration?: number
) {
  useToastStore.getState().addToast({ message, type: "info", action, duration });
}

export { humanizeError };
export type { HumanizedError };
