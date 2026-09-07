import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { humanizeError } from "./errors";

/**
 * SPEC-F2 T2: unified invoke wrapper.
 * All frontend invocations should go through this function so raw technical
 * errors are never surfaced to users without humanization.
 *
 * SPEC-F6 T3: carry the recovery `action` through so callers that only pass
 * the thrown error to showError retain the suggested recovery hint.
 */
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return args ? await tauriInvoke<T>(cmd, args) : await tauriInvoke<T>(cmd);
  } catch (err) {
    const humanized = humanizeError(err, { context: undefined });
    const wrapped = new Error(humanized.message);
    (wrapped as Error & { detail?: string }).detail = humanized.detail;
    (wrapped as Error & { action?: string }).action = humanized.action;
    throw wrapped;
  }
}

/**
 * Q5 丝滑度兜底：带超时的 invoke。超时后 reject（本地命令可能仍在后台执行，
 * 但 UI 不再无限转圈）。用于所有"点完一直加载中"风险点位。
 */
export async function invokeWithTimeout<T>(
  cmd: string,
  args: Record<string, unknown> | undefined,
  ms: number,
  timeoutMsg: string
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(timeoutMsg)), ms);
  });
  try {
    return await Promise.race([invoke<T>(cmd, args), timeout]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}
