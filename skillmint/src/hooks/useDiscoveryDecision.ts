import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DecideDiscoveryResult, DismissReason, Discovery } from "../types";

export type DecisionAction = "accept" | "accept_edited" | "dismiss";

export interface UseDiscoveryDecisionResult {
  decide: (
    discovery: Discovery,
    action: DecisionAction,
    reason?: DismissReason,
  ) => Promise<DecideDiscoveryResult>;
  loading: boolean;
}

/**
 * Shared decision logic for discovery cards.
 *
 * Callers are responsible for optimistic UI updates because the Inbox page and
 * the Today strip maintain their own lists.
 */
export function useDiscoveryDecision(): UseDiscoveryDecisionResult {
  const [loading, setLoading] = useState(false);

  const decide = useCallback(async (
    discovery: Discovery,
    action: DecisionAction,
    reason?: DismissReason,
  ): Promise<DecideDiscoveryResult> => {
    setLoading(true);
    try {
      const result = await invoke<DecideDiscoveryResult>("decide_discovery", {
        id: discovery.id,
        action,
        reason: reason ?? null,
      });
      return result;
    } finally {
      setLoading(false);
    }
  }, []);

  return { decide, loading };
}
