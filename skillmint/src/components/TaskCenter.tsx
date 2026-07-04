import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Activity, X, RotateCcw, CheckCircle2, AlertCircle, Loader2, Clock } from "lucide-react";
import { useCollectionStore } from "../stores/collectionStore";
import { showError, showSuccess } from "../stores/toastStore";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";
import { cn } from "./ui/utils";
import type { CollectionJob } from "../types";

export default function TaskCenter() {
  const [open, setOpen] = useState(false);
  const { recentJobs, collecting, currentJobId, loadRecentJobs, cancelCollect } =
    useCollectionStore();

  const refresh = async () => {
    await loadRecentJobs();
  };

  const handleCancel = async (job: CollectionJob) => {
    await cancelCollect(job.id);
  };

  const handleRetry = async (_job: CollectionJob) => {
    try {
      await invoke("start_collection_job");
      showSuccess("已重新启动采集任务");
      await loadRecentJobs();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`启动失败：${msg}`);
    }
  };

  const running = recentJobs.find((j) => j.status === "running");

  return (
    <div className="relative">
      <button
        onClick={() => {
          setOpen((v) => !v);
          void loadRecentJobs();
        }}
        className={cn(
          "group relative flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm font-medium transition-all duration-200",
          open
            ? "bg-accent/10 text-accent"
            : "text-secondary hover:bg-tertiary/60 hover:text-primary"
        )}
      >
        {collecting && (
          <Loader2 className="h-[18px] w-[18px] shrink-0 animate-spin text-accent" />
        )}
        {!collecting && (
          <Activity
            className={cn(
              "h-[18px] w-[18px] shrink-0 transition-colors",
              open ? "text-accent" : "text-tertiary group-hover:text-secondary"
            )}
          />
        )}
        <span className="flex-1 truncate">任务中心</span>
        {running && (
          <span className="ml-auto flex h-2 w-2 rounded-full bg-accent animate-pulse" />
        )}
      </button>

      {open && (
        <Card className="absolute bottom-full left-3 right-3 mb-2 max-h-96 overflow-auto p-0 shadow-lg">
          <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-4 py-3">
            <div className="flex items-center gap-2 text-sm font-semibold text-primary">
              <Activity className="h-4 w-4 text-accent" />
              后台任务
            </div>
            <div className="flex items-center gap-1">
              <button
                onClick={refresh}
                className="rounded p-1.5 text-tertiary hover:bg-tertiary hover:text-primary"
                title="刷新"
              >
                <RotateCcw className="h-4 w-4" />
              </button>
              <button
                onClick={() => setOpen(false)}
                className="rounded p-1.5 text-tertiary hover:bg-tertiary hover:text-primary"
                title="关闭"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          </div>

          {recentJobs.length === 0 ? (
            <div className="px-4 py-6 text-center text-sm text-secondary">暂无后台任务</div>
          ) : (
            <div className="divide-y divide-[var(--border-subtle)]">
              {recentJobs.map((job) => (
                <div key={job.id} className="px-4 py-3">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2 overflow-hidden">
                      <JobStatusIcon status={job.status} />
                      <span className="truncate text-sm text-primary">
                        {formatTime(job.started_at)}
                      </span>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      {job.status === "running" && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => handleCancel(job)}
                          disabled={!collecting || currentJobId !== job.id}
                        >
                          取消
                        </Button>
                      )}
                      {(job.status === "failed" || job.status === "cancelled") && (
                        <Button variant="ghost" size="sm" onClick={() => handleRetry(job)}>
                          重试
                        </Button>
                      )}
                    </div>
                  </div>
                  <div className="mt-1 flex items-center gap-2 text-xs text-secondary">
                    <span className={statusBadgeClass(job.status)}>{statusLabel(job.status)}</span>
                    {job.error && <span className="truncate text-danger">{job.error}</span>}
                  </div>
                </div>
              ))}
            </div>
          )}
        </Card>
      )}
    </div>
  );
}

function JobStatusIcon({ status }: { status: CollectionJob["status"] }) {
  switch (status) {
    case "running":
      return <Loader2 className="h-4 w-4 animate-spin text-accent" />;
    case "completed":
      return <CheckCircle2 className="h-4 w-4 text-success" />;
    case "failed":
      return <AlertCircle className="h-4 w-4 text-danger" />;
    case "cancelled":
      return <X className="h-4 w-4 text-warning" />;
    default:
      return <Clock className="h-4 w-4 text-tertiary" />;
  }
}

function statusLabel(status: CollectionJob["status"]) {
  switch (status) {
    case "pending":
      return "等待中";
    case "running":
      return "运行中";
    case "completed":
      return "已完成";
    case "failed":
      return "失败";
    case "cancelled":
      return "已取消";
    default:
      return status;
  }
}

function statusBadgeClass(status: CollectionJob["status"]) {
  switch (status) {
    case "running":
      return "rounded bg-accent/10 px-1.5 py-0.5 text-accent";
    case "completed":
      return "rounded bg-success/10 px-1.5 py-0.5 text-success";
    case "failed":
      return "rounded bg-danger/10 px-1.5 py-0.5 text-danger";
    case "cancelled":
      return "rounded bg-warning/10 px-1.5 py-0.5 text-warning";
    default:
      return "rounded bg-tertiary/50 px-1.5 py-0.5 text-secondary";
  }
}

function formatTime(ts: number) {
  return new Date(ts * 1000).toLocaleString();
}
