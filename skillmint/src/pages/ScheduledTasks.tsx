import { useEffect, useState } from "react";
import {
  CalendarClock,
  CheckCircle2,
  Clock,
  Database,
  Download,
  Edit3,
  GitBranch,
  History,
  LayoutGrid,
  MoreHorizontal,
  Pause,
  Play,
  Plus,
  RefreshCw,
  ScanLine,
  Search,
  Share2,
  Trash2,
} from "lucide-react";
import { useScheduledTaskStore } from "../stores/scheduledTaskStore";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import ScheduledTaskForm from "../components/ScheduledTaskForm";
import ScheduledTaskHistory from "../components/ScheduledTaskHistory";
import type { ScheduledTask, ScheduleStrategy, TaskKind } from "../types";

const TASK_ICON: Record<TaskKind, React.ElementType> = {
  collect_usage_data: Database,
  scan_agents: ScanLine,
  sync_all_skills: RefreshCw,
  import_agent_skills: Download,
  scan_agent_directories: Search,
  scan_projects: LayoutGrid,
  generate_knowledge_graph: Share2,
  generate_daily_summary: CalendarClock,
  sync_remote_sources: GitBranch,
  backup_center_repo: CheckCircle2,
};

function formatStrategy(strategy: ScheduleStrategy): string {
  if (strategy.kind === "manual") return "手动执行";
  if (strategy.kind === "interval") {
    const unit = strategy.unit === "hours" ? "小时" : strategy.unit === "days" ? "天" : "分钟";
    return `每 ${strategy.value} ${unit}`;
  }
  return `Cron: ${strategy.expression}`;
}

function timeAgo(ts?: number): string {
  if (!ts) return "从未";
  const diff = Math.max(0, Date.now() / 1000 - ts);
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

function nextRunText(ts?: number): string {
  if (!ts) return "下次：—";
  const d = new Date(ts * 1000);
  return `下次 ${d.toLocaleString("zh-CN", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}`;
}

function StatusBadge({ status, errorCount }: { status?: ScheduledTask["last_status"]; errorCount: number }) {
  if (!status) return <Badge variant="default">未执行</Badge>;
  switch (status) {
    case "success":
      return <Badge variant="success">成功</Badge>;
    case "failed":
      return <Badge variant="danger">失败 {errorCount > 0 ? `(${errorCount})` : ""}</Badge>;
    case "running":
      return <Badge variant="accent">运行中</Badge>;
    case "skipped":
      return <Badge variant="warning">跳过</Badge>;
    default:
      return <Badge variant="default">待执行</Badge>;
  }
}

interface ScheduledTasksProps {
  embedded?: boolean;
}

const FILTERS: { key: "all" | "enabled" | "disabled" | "failed"; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "enabled", label: "已启用" },
  { key: "disabled", label: "已暂停" },
  { key: "failed", label: "失败" },
];

export default function ScheduledTasks({ embedded = false }: ScheduledTasksProps) {
  const tasks = useScheduledTaskStore((state) => state.tasks);
  const loading = useScheduledTaskStore((state) => state.loadingTasks);
  const runningIds = useScheduledTaskStore((state) => state.runningIds);
  const loadTasks = useScheduledTaskStore((state) => state.loadTasks);
  const saveTask = useScheduledTaskStore((state) => state.saveTask);
  const deleteTask = useScheduledTaskStore((state) => state.deleteTask);
  const runNow = useScheduledTaskStore((state) => state.runNow);
  const toggleEnabled = useScheduledTaskStore((state) => state.toggleEnabled);

  const [formTask, setFormTask] = useState<ScheduledTask | null>(null);
  const [historyTask, setHistoryTask] = useState<ScheduledTask | null>(null);
  const [filter, setFilter] = useState<"all" | "enabled" | "disabled" | "failed">("all");
  const [menuOpenId, setMenuOpenId] = useState<string | null>(null);

  useEffect(() => {
    loadTasks();
  }, [loadTasks]);

  const filteredTasks = tasks
    .filter((t) => {
      if (filter === "enabled") return t.enabled;
      if (filter === "disabled") return !t.enabled;
      if (filter === "failed") return t.last_status === "failed";
      return true;
    })
    .sort((a, b) => a.created_at - b.created_at);

  const handleSave = async (task: ScheduledTask) => {
    const saved = await saveTask(task);
    if (saved) setFormTask(null);
  };

  const handleDelete = async (task: ScheduledTask) => {
    if (confirm(`确定删除任务「${task.name}」吗？`)) {
      await deleteTask(task.id);
    }
  };

  return (
    <div className={embedded ? "" : "h-full overflow-auto p-8"}>
      {!embedded && (
        <div className="mb-6 flex items-center justify-between">
          <div>
            <h1 className="text-2xl font-bold text-primary">定时任务</h1>
            <p className="mt-1 text-sm text-secondary">管理 SkillMint 中需要周期性执行的后台任务</p>
          </div>
          <Button variant="primary" size="sm" onClick={() => setFormTask({} as ScheduledTask)}>
            <Plus className="mr-1 h-4 w-4" />
            新建任务
          </Button>
        </div>
      )}

      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap gap-2">
          {FILTERS.map((f) => (
            <button
              key={f.key}
              onClick={() => setFilter(f.key)}
              className={`rounded-full px-3 py-1 text-sm font-medium transition-colors ${
                filter === f.key
                  ? "bg-accent text-white"
                  : "bg-secondary text-secondary hover:bg-tertiary hover:text-primary"
              }`}
            >
              {f.label}
            </button>
          ))}
        </div>
        {embedded && (
          <Button variant="primary" size="sm" onClick={() => setFormTask({} as ScheduledTask)}>
            <Plus className="mr-1 h-4 w-4" />
            新建任务
          </Button>
        )}
      </div>

      {loading && tasks.length === 0 ? (
        <div className="space-y-3">
          {[1, 2, 3].map((i) => (
            <Card key={i} className="h-24 animate-pulse" />
          ))}
        </div>
      ) : filteredTasks.length === 0 ? (
        <Card className="p-12 text-center">
          <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-secondary">
            <Clock className="h-7 w-7 text-tertiary" />
          </div>
          <div className="mt-4 text-lg font-medium text-primary">暂无任务</div>
          <p className="mt-1 text-sm text-secondary">点击右上角「新建任务」创建第一个定时任务。</p>
        </Card>
      ) : (
        <div className="space-y-3">
          {filteredTasks.map((task) => {
            const isRunning = runningIds.has(task.id) || task.last_status === "running";
            const TaskIcon = TASK_ICON[task.task_kind] || Clock;
            return (
              <Card key={task.id} className="group p-4 transition-shadow hover:shadow-sm">
                <div className="flex items-start gap-4">
                  <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-secondary text-accent">
                    <TaskIcon className="h-5 w-5" />
                  </div>

                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-semibold text-primary">{task.name}</span>
                      <StatusBadge status={task.last_status} errorCount={task.error_count} />
                      {!task.enabled && <Badge variant="default">已暂停</Badge>}
                    </div>
                    <p className="mt-0.5 text-sm text-secondary">{task.description}</p>
                    <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-tertiary">
                      <span className="flex items-center gap-1">
                        <Clock className="h-3.5 w-3.5" />
                        {formatStrategy(task.strategy)}
                      </span>
                      <span>{nextRunText(task.next_run_at)}</span>
                      <span>最后执行：{timeAgo(task.last_run_at)}</span>
                      <span>
                        运行 {task.run_count} 次
                        {task.error_count > 0 ? ` · 失败 ${task.error_count} 次` : ""}
                      </span>
                    </div>
                  </div>

                  <div className="flex shrink-0 items-center gap-1">
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => runNow(task.id)}
                      loading={isRunning}
                      disabled={isRunning}
                    >
                      {isRunning ? (
                        "运行中…"
                      ) : (
                        <>
                          <Play className="mr-1 h-4 w-4" />
                          执行
                        </>
                      )}
                    </Button>

                    <div className="relative">
                      <button
                        onClick={() => setMenuOpenId(menuOpenId === task.id ? null : task.id)}
                        className="rounded-lg p-2 text-tertiary hover:bg-tertiary hover:text-primary"
                      >
                        <MoreHorizontal className="h-5 w-5" />
                      </button>
                      {menuOpenId === task.id && (
                        <>
                          <div
                            className="fixed inset-0 z-30"
                            onClick={() => setMenuOpenId(null)}
                          />
                          <div className="absolute right-0 top-full z-40 mt-1 w-40 rounded-xl border border-[var(--border-subtle)] bg-elevated py-1 shadow-xl">
                            <button
                              onClick={() => {
                                setMenuOpenId(null);
                                setHistoryTask(task);
                              }}
                              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-secondary hover:bg-tertiary/50 hover:text-primary"
                            >
                              <History className="h-4 w-4" />
                              执行历史
                            </button>
                            <button
                              onClick={() => {
                                setMenuOpenId(null);
                                setFormTask(task);
                              }}
                              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-secondary hover:bg-tertiary/50 hover:text-primary"
                            >
                              <Edit3 className="h-4 w-4" />
                              编辑
                            </button>
                            <button
                              onClick={() => {
                                setMenuOpenId(null);
                                toggleEnabled(task);
                              }}
                              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-secondary hover:bg-tertiary/50 hover:text-primary"
                            >
                              {task.enabled ? (
                                <>
                                  <Pause className="h-4 w-4" />
                                  暂停
                                </>
                              ) : (
                                <>
                                  <Play className="h-4 w-4" />
                                  启用
                                </>
                              )}
                            </button>
                            <div className="my-1 border-t border-[var(--border-subtle)]" />
                            <button
                              onClick={() => {
                                setMenuOpenId(null);
                                handleDelete(task);
                              }}
                              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-danger hover:bg-danger/10"
                            >
                              <Trash2 className="h-4 w-4" />
                              删除
                            </button>
                          </div>
                        </>
                      )}
                    </div>
                  </div>
                </div>
              </Card>
            );
          })}
        </div>
      )}

      {formTask && (
        <ScheduledTaskForm
          task={formTask.id ? formTask : null}
          onSave={handleSave}
          onCancel={() => setFormTask(null)}
        />
      )}

      {historyTask && (
        <ScheduledTaskHistory
          taskId={historyTask.id}
          taskName={historyTask.name}
          onClose={() => setHistoryTask(null)}
        />
      )}
    </div>
  );
}
