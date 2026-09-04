import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { Button } from "./ui/Button";
import type { ScheduledTask, TaskKind, ScheduleStrategy, IntervalUnit } from "../types";

interface ScheduledTaskFormProps {
  task?: ScheduledTask | null;
  onSave: (task: ScheduledTask) => void;
  onCancel: () => void;
}

const TASK_KIND_OPTIONS: { value: TaskKind; label: string }[] = [
  { value: "collect_usage_data", label: "使用数据采集（增量）" },
  { value: "scan_agents", label: "扫描 Agent 目录" },
  { value: "sync_all_skills", label: "同步 Skill 到 Agent（增量）" },
  { value: "import_agent_skills", label: "从 Agent 导入 Skill 到中心仓库" },
  { value: "scan_agent_directories", label: "扫描 Agent 目录技能缓存" },
  { value: "scan_projects", label: "扫描项目" },
  { value: "generate_knowledge_graph", label: "生成知识图谱" },
  { value: "generate_daily_summary", label: "生成每日 AI 摘要" },
  { value: "sync_remote_sources", label: "刷新远程源与 Git 拉取" },
];

const DEFAULTS: Record<
  TaskKind,
  { name: string; description: string; strategy: ScheduleStrategy; enabled: boolean }
> = {
  collect_usage_data: {
    name: "使用数据采集（增量）",
    description: "增量读取各 Agent 本地落盘的使用数据，未变化文件自动跳过。",
    strategy: { kind: "interval", value: 30, unit: "minutes" },
    enabled: true,
  },
  scan_agents: {
    name: "扫描 Agent 目录",
    description: "扫描本机已安装的 Agent 并更新目录列表。",
    strategy: { kind: "interval", value: 1, unit: "hours" },
    enabled: false,
  },
  sync_all_skills: {
    name: "同步 Skill 到 Agent（增量）",
    description: "按 hash 比对，仅把中心仓库的变更分发到已启用 Agent。",
    strategy: { kind: "cron", expression: "0 3 * * *" },
    enabled: false,
  },
  import_agent_skills: {
    name: "从 Agent 导入 Skill 到中心仓库",
    description: "当 Center Repo 为空时，把 Agent 目录现有 Skill 复制到中心仓库。",
    strategy: { kind: "manual" },
    enabled: false,
  },
  scan_agent_directories: {
    name: "扫描 Agent 目录技能缓存",
    description: "刷新各 Agent 目录下的技能缓存，供 UI 快速展示。",
    strategy: { kind: "interval", value: 1, unit: "hours" },
    enabled: false,
  },
  scan_projects: {
    name: "扫描项目",
    description: "扫描近期使用过的项目并关联到 Agent。",
    strategy: { kind: "interval", value: 1, unit: "hours" },
    enabled: false,
  },
  generate_knowledge_graph: {
    name: "生成知识图谱",
    description: "分析中心仓库所有 SKILL.md 并生成/更新知识图谱。",
    strategy: { kind: "cron", expression: "0 2 * * *" },
    enabled: false,
  },
  generate_daily_summary: {
    name: "生成每日 AI 摘要",
    description: "生成前一日的 AI 工作摘要。",
    strategy: { kind: "cron", expression: "0 8 * * *" },
    enabled: false,
  },
  sync_remote_sources: {
    name: "刷新远程源与 Git 拉取",
    description: "刷新已连接远程源的本地缓存，并执行 Center Repo 的 Git 拉取。不会自动把 Skill 同步到 Agent。",
    strategy: { kind: "cron", expression: "0 4 * * *" },
    enabled: false,
  },
  backup_center_repo: {
    name: "备份 Center Repo",
    description: "将 Center Repo 打包备份到本地备份目录。",
    strategy: { kind: "cron", expression: "0 2 * * 0" },
    enabled: false,
  },
};

export default function ScheduledTaskForm({ task, onSave, onCancel }: ScheduledTaskFormProps) {
  const isEditing = Boolean(task);
  const [taskKind, setTaskKind] = useState<TaskKind>(task?.task_kind ?? "collect_usage_data");
  const [name, setName] = useState(task?.name ?? "");
  const [description, setDescription] = useState(task?.description ?? "");
  const [strategyKind, setStrategyKind] = useState<ScheduleStrategy["kind"]>(
    task?.strategy.kind ?? "interval",
  );
  const [intervalValue, setIntervalValue] = useState<number>(task?.strategy.value ?? 30);
  const [intervalUnit, setIntervalUnit] = useState<IntervalUnit>(task?.strategy.unit ?? "minutes");
  const [cronExpression, setCronExpression] = useState(task?.strategy.expression ?? "0 3 * * *");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isEditing) {
      const def = DEFAULTS[taskKind];
      setName(def.name);
      setDescription(def.description);
      setStrategyKind(def.strategy.kind);
      setIntervalValue(def.strategy.value ?? 30);
      setIntervalUnit(def.strategy.unit ?? "minutes");
      setCronExpression(def.strategy.expression ?? "0 3 * * *");
    }
  }, [taskKind, isEditing]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    let strategy: ScheduleStrategy;
    if (strategyKind === "manual") {
      strategy = { kind: "manual" };
    } else if (strategyKind === "interval") {
      if (!intervalValue || intervalValue <= 0) {
        setError("间隔必须大于 0");
        return;
      }
      strategy = { kind: "interval", value: intervalValue, unit: intervalUnit };
    } else {
      if (!cronExpression.trim()) {
        setError("Cron 表达式不能为空");
        return;
      }
      strategy = { kind: "cron", expression: cronExpression.trim() };
    }

    if (!name.trim()) {
      setError("任务名称不能为空");
      return;
    }

    const now = Math.floor(Date.now() / 1000);
    const saved: ScheduledTask = {
      id: task?.id ?? "",
      task_kind: isEditing ? task!.task_kind : taskKind,
      name: name.trim(),
      description: description.trim(),
      enabled: task?.enabled ?? DEFAULTS[taskKind].enabled,
      strategy,
      created_at: task?.created_at ?? now,
      updated_at: now,
      last_run_at: task?.last_run_at,
      last_status: task?.last_status,
      next_run_at: task?.next_run_at,
      run_count: task?.run_count ?? 0,
      error_count: task?.error_count ?? 0,
    };
    onSave(saved);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div className="w-full max-w-lg rounded-xl border border-[var(--border-subtle)] bg-secondary p-6 shadow-2xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold">{isEditing ? "编辑任务" : "新建任务"}</h2>
          <button onClick={onCancel} className="text-secondary hover:text-primary">
            <X className="h-5 w-5" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {!isEditing && (
            <div>
              <label className="mb-1 block text-sm text-secondary">任务类型</label>
              <select
                value={taskKind}
                onChange={(e) => setTaskKind(e.target.value as TaskKind)}
                className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-2 text-sm text-primary focus:border-accent focus:outline-none"
              >
                {TASK_KIND_OPTIONS.map((opt) => (
                  <option key={opt.value} value={opt.value}>
                    {opt.label}
                  </option>
                ))}
              </select>
            </div>
          )}

          <div>
            <label className="mb-1 block text-sm text-secondary">名称</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-2 text-sm text-primary focus:border-accent focus:outline-none"
            />
          </div>

          <div>
            <label className="mb-1 block text-sm text-secondary">描述</label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={3}
              className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-2 text-sm text-primary focus:border-accent focus:outline-none"
            />
          </div>

          <div>
            <label className="mb-2 block text-sm text-secondary">执行策略</label>
            <div className="inline-flex rounded-lg border border-[var(--border-prominent)] p-1">
              {(
                [
                  ["manual", "手动执行"],
                  ["interval", "按间隔"],
                  ["cron", "Cron"],
                ] as const
              ).map(([kind, label]) => (
                <button
                  key={kind}
                  type="button"
                  onClick={() => setStrategyKind(kind)}
                  className={`rounded-md px-4 py-1.5 text-sm font-medium transition-colors ${
                    strategyKind === kind
                      ? "bg-accent text-white"
                      : "text-secondary hover:text-primary"
                  }`}
                >
                  {label}
                </button>
              ))}
            </div>

            <div className="mt-3 rounded-lg border border-[var(--border-prominent)] bg-primary p-3">
              {strategyKind === "manual" && (
                <p className="text-sm text-secondary">仅在被手动触发时执行，不会按定时计划运行。</p>
              )}
              {strategyKind === "interval" && (
                <div className="flex items-center gap-3">
                  <span className="text-sm text-secondary">每</span>
                  <input
                    type="number"
                    min={1}
                    value={intervalValue}
                    onChange={(e) => setIntervalValue(parseInt(e.target.value) || 1)}
                    className="w-24 rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
                  />
                  <select
                    value={intervalUnit}
                    onChange={(e) => setIntervalUnit(e.target.value as IntervalUnit)}
                    className="rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
                  >
                    <option value="minutes">分钟</option>
                    <option value="hours">小时</option>
                    <option value="days">天</option>
                  </select>
                  执行一次
                </div>
              )}
              {strategyKind === "cron" && (
                <div>
                  <input
                    type="text"
                    value={cronExpression}
                    onChange={(e) => setCronExpression(e.target.value)}
                    placeholder="0 3 * * *"
                    className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
                  />
                  <div className="mt-2 flex flex-wrap gap-2">
                    {[
                      { expr: "*/30 * * * *", label: "每 30 分钟" },
                      { expr: "0 * * * *", label: "每小时" },
                      { expr: "0 3 * * *", label: "每天 03:00" },
                    ].map((preset) => (
                      <button
                        key={preset.expr}
                        type="button"
                        onClick={() => setCronExpression(preset.expr)}
                        className="rounded bg-tertiary px-2 py-1 text-xs text-secondary hover:text-primary"
                      >
                        {preset.label}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
          </div>

          {error && <p className="text-sm text-danger">{error}</p>}

          <div className="flex justify-end gap-2 pt-2">
            <Button variant="secondary" size="sm" onClick={onCancel}>
              取消
            </Button>
            <Button variant="primary" size="sm" type="submit">
              保存
            </Button>
          </div>
        </form>
      </div>
    </div>
  );
}
