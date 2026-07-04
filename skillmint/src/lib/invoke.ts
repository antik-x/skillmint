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
