import { invoke } from "../lib/invoke";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { create } from "zustand";
import { showError, showInfo, showSuccess } from "./toastStore";
import type { CollectedSource, CollectionJob, CollectionStats } from "../types";

interface JobProgress {
  job_id: string;
  source: string;
  stats: CollectionStats;
}

interface JobCompleted {
  job_id: string;
  stats: CollectionStats[];
}

interface JobFailed {
  job_id: string;
  error: string;
}

interface CollectionState {
  sources: CollectedSource[];
  loading: boolean;
  collecting: boolean;
  currentJobId: string | null;
  lastJobStats: CollectionStats[] | null;
  lastJobError: string | null;
  lastUpdatedAt: number;
  recentJobs: CollectionJob[];
}

interface CollectionActions {
  loadStatus: (force?: boolean) => Promise<CollectedSource[]>;
  startCollect: () => Promise<string | null>;
  cancelCollect: (jobId: string) => Promise<void>;
  loadRecentJobs: (limit?: number) => Promise<void>;
  reset: () => void;
}

const initialState: CollectionState = {
  sources: [],
  loading: false,
  collecting: false,
  currentJobId: null,
  lastJobStats: null,
  lastJobError: null,
  lastUpdatedAt: 0,
  recentJobs: [],
};

let listenersReady = false;

/**
 * Centralized collection state shared across Data Collection, Usage Insight,
 * and Settings pages. Keeps a single source of truth for sources status and
 * the ongoing collect operation.
 *
 * Manual collection is now asynchronous: `startCollect` returns immediately
 * with a job id, and completion is delivered via Tauri events.
 */
export const useCollectionStore = create<CollectionState & CollectionActions>((set, get) => {
  if (!listenersReady) {
    listenersReady = true;

    let unlistenProgress: UnlistenFn | null = null;
    let unlistenCompleted: UnlistenFn | null = null;
    let unlistenFailed: UnlistenFn | null = null;

    listen<JobProgress>("collection:progress", (event) => {
      const { job_id, source, stats } = event.payload;
      // P0: only accept progress for the currently tracked job.
      if (job_id !== get().currentJobId) return;
      // Surface per-source progress by updating the source row in-place.
      set((state) => ({
        sources: state.sources.map((s) =>
          s.source === source
            ? {
                ...s,
                status: "ok",
                record_count:
                  stats.sessions >= 0
                    ? Math.max(s.record_count, stats.sessions)
                    : s.record_count,
              }
            : s
        ),
      }));
      // eslint-disable-next-line no-console
      console.log(`[collection] ${job_id} ${source}:`, stats);
    }).then((u) => {
      unlistenProgress = u;
    });

    listen<JobCompleted>("collection:completed", (event) => {
      const { job_id, stats } = event.payload;
      if (job_id !== get().currentJobId) return;
      const totalSessions = stats.reduce((a, s) => a + s.sessions, 0);
      const okSources = stats.filter((s) => s.sessions > 0 || s.prompts > 0);

      set({
        collecting: false,
        currentJobId: null,
        lastJobStats: stats,
        lastJobError: null,
      });

      if (okSources.length === 0) {
        showInfo("未发现可采集的使用数据（请先用 Agent 产生会话）。");
      } else {
        showSuccess(`采集完成，共 ${totalSessions} 个会话`);
      }
      get().loadStatus(true);
      get().loadRecentJobs();
    }).then((u) => {
      unlistenCompleted = u;
    });

    listen<JobFailed>("collection:failed", (event) => {
      const { job_id, error } = event.payload;
      if (job_id !== get().currentJobId) return;
      set({
        collecting: false,
        currentJobId: null,
        lastJobError: error,
      });
      showError(`采集失败：${error}`);
      get().loadStatus(true);
      get().loadRecentJobs();
    }).then((u) => {
      unlistenFailed = u;
    });

    // Best-effort cleanup on page unload is handled by the browser dropping the
    // JS context; Tauri listeners are per-webview and cleaned up automatically.
    void { unlistenProgress, unlistenCompleted, unlistenFailed };
  }

  return {
    ...initialState,

    loadStatus: async (force = false) => {
      if (!force && get().loading) return get().sources;
      set({ loading: true });
      try {
        const s = await invoke<CollectedSource[]>("get_collection_status");
        const sources = Array.isArray(s) ? s : [];
        set({ sources, lastUpdatedAt: Date.now() });
        return sources;
      } catch (err) {
        const msg =
          typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`读取采集状态失败：${msg}`);
        return get().sources;
      } finally {
        set({ loading: false });
      }
    },

    startCollect: async () => {
      set({ collecting: true, lastJobStats: null, lastJobError: null });
      try {
        const jobId = await invoke<string>("start_collection_job");
        set({ currentJobId: jobId });
        get().loadRecentJobs();
        return jobId;
      } catch (err) {
        const msg =
          typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`启动采集失败：${msg}`);
        set({ collecting: false });
        return null;
      }
    },

    cancelCollect: async (jobId: string) => {
      try {
        await invoke("cancel_collection_job", { id: jobId });
        showInfo("已发送取消请求");
        get().loadRecentJobs();
      } catch (err) {
        const msg =
          typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`取消失败：${msg}`);
      }
    },

    loadRecentJobs: async (limit = 20) => {
      try {
        const jobs = await invoke<CollectionJob[]>("list_recent_collection_jobs", { limit });
        set({ recentJobs: jobs });
      } catch (err) {
        const msg =
          typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        console.error("[collectionStore] loadRecentJobs failed:", msg);
      }
    },

    reset: () => {
      const state = get();
      if (state.currentJobId && state.collecting) {
        invoke("cancel_collection_job", { id: state.currentJobId }).catch(() => {});
      }
      set(initialState);
    },
  };
});
