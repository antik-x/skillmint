import { X, Check, AlertTriangle, Loader2 } from "lucide-react";
import { useEffect } from "react";
import { useScheduledTaskStore } from "../stores/scheduledTaskStore";
import type { TaskRun } from "../types";

interface ScheduledTaskHistoryProps {
  taskId: string;
  taskName: string;
  onClose: () => void;
}

function statusIcon(status: TaskRun["status"]) {
  switch (status) {
    case "success":
      return <Check className="h-4 w-4 text-success" />;
    case "failed":
      return <AlertTriangle className="h-4 w-4 text-danger" />;
    case "running":
      return <Loader2 className="h-4 w-4 animate-spin text-accent" />;
    default:
      return <Loader2 className="h-4 w-4 animate-spin text-tertiary" />;
  }
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  return d.toLocaleString("zh-CN", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export default function ScheduledTaskHistory({ taskId, taskName, onClose }: ScheduledTaskHistoryProps) {
  const runs = useScheduledTaskStore((state) => state.runs[taskId] ?? []);
  const loadRuns = useScheduledTaskStore((state) => state.loadRuns);

  useEffect(() => {
    loadRuns(taskId, 50);
  }, [taskId, loadRuns]);

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      {/* Backdrop */}
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />

      {/* Panel */}
      <div className="relative h-full w-full max-w-md overflow-auto border-l border-[var(--border-subtle)] bg-secondary p-5 shadow-2xl">
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-lg font-semibold text-primary">{taskName} · 执行历史</h3>
          <button
            onClick={onClose}
            className="rounded-lg px-2 py-1 text-sm text-secondary hover:bg-tertiary hover:text-primary"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {runs.length === 0 ? (
          <div className="text-sm text-tertiary">暂无执行记录</div>
        ) : (
          <ul className="space-y-3">
            {runs.map((run) => (
              <li
                key={run.id}
                className="rounded-lg border border-[var(--border-subtle)]/60 bg-primary p-3"
              >
                <div className="flex items-center gap-2">
                  {statusIcon(run.status)}
                  <span className="text-sm font-medium text-primary">{formatTime(run.started_at)}</span>
                  {run.duration_ms !== undefined && run.duration_ms > 0 && (
                    <span className="ml-auto text-xs text-tertiary">
                      {(run.duration_ms / 1000).toFixed(1)}s
                    </span>
                  )}
                </div>
                {run.result_summary && (
                  <p className="mt-1.5 text-sm text-secondary">{run.result_summary}</p>
                )}
                {run.error_message && (
                  <p className="mt-1.5 text-xs text-danger">{run.error_message}</p>
                )}
                <div className="mt-1 text-xs text-tertiary">
                  触发：
                  {run.triggered_by === "manual" ? "手动" : run.triggered_by === "startup" ? "启动" : "定时"}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
