import { invoke } from "../lib/invoke";
import { create } from "zustand";
import { showError, showSuccess } from "./toastStore";
import type { ScheduledTask, TaskRun } from "../types";

interface ScheduledTaskState {
  tasks: ScheduledTask[];
  runs: Record<string, TaskRun[]>;
  loadingTasks: boolean;
  runningIds: Set<string>;
}

interface ScheduledTaskActions {
  loadTasks: () => Promise<ScheduledTask[]>;
  saveTask: (task: ScheduledTask) => Promise<ScheduledTask | null>;
  deleteTask: (id: string) => Promise<void>;
  runNow: (id: string) => Promise<void>;
  loadRuns: (taskId: string, limit?: number) => Promise<TaskRun[]>;
  toggleEnabled: (task: ScheduledTask) => Promise<ScheduledTask | null>;
  setRunningId: (id: string, running: boolean) => void;
}

const initialState: ScheduledTaskState = {
  tasks: [],
  runs: {},
  loadingTasks: false,
  runningIds: new Set(),
};

export const useScheduledTaskStore = create<ScheduledTaskState & ScheduledTaskActions>((set, get) => ({
  ...initialState,

  loadTasks: async () => {
    if (get().loadingTasks) return get().tasks;
    set({ loadingTasks: true });
    try {
      const tasks = await invoke<ScheduledTask[]>("get_scheduled_tasks");
      set({ tasks, loadingTasks: false });
      return tasks;
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`读取定时任务失败：${msg}`);
      set({ loadingTasks: false });
      return get().tasks;
    }
  },

  saveTask: async (task) => {
    try {
      const saved = await invoke<ScheduledTask>("save_scheduled_task", { task });
      await get().loadTasks();
      showSuccess("任务已保存");
      return saved;
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`保存任务失败：${msg}`);
      return null;
    }
  },

  deleteTask: async (id) => {
    try {
      await invoke("delete_scheduled_task", { id });
      await get().loadTasks();
      set((state) => {
        const next = { ...state.runs };
        delete next[id];
        return { runs: next };
      });
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`删除任务失败：${msg}`);
    }
  },

  runNow: async (id) => {
    if (get().runningIds.has(id)) return;
    get().setRunningId(id, true);
    try {
      await invoke<TaskRun>("run_scheduled_task_now", { id });
      showSuccess("任务已开始执行");
      // Poll briefly for completion.
      let attempts = 0;
      const poll = async () => {
        if (attempts > 30) return;
        attempts += 1;
        await get().loadRuns(id, 1);
        const latest = get().runs[id]?.[0];
        if (latest && latest.status !== "pending" && latest.status !== "running") {
          get().setRunningId(id, false);
          return;
        }
        setTimeout(poll, 1000);
      };
      setTimeout(poll, 500);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`执行任务失败：${msg}`);
      get().setRunningId(id, false);
    }
  },

  loadRuns: async (taskId, limit = 20) => {
    try {
      const runs = await invoke<TaskRun[]>("get_task_runs", { taskId, limit });
      set((state) => ({ runs: { ...state.runs, [taskId]: runs } }));
      return runs;
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`读取执行历史失败：${msg}`);
      return get().runs[taskId] ?? [];
    }
  },

  toggleEnabled: async (task) => {
    try {
      const command = task.enabled ? "pause_scheduled_task" : "resume_scheduled_task";
      const updated = await invoke<ScheduledTask>(command, { id: task.id });
      await get().loadTasks();
      return updated;
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`切换任务状态失败：${msg}`);
      return null;
    }
  },

  setRunningId: (id, running) => {
    set((state) => {
      const next = new Set(state.runningIds);
      if (running) next.add(id);
      else next.delete(id);
      return { runningIds: next };
    });
  },
}));
