import { memo, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Puzzle,
  Bot,
  RefreshCw,
  Plus,
  ScanSearch,
  CheckCircle2,
  AlertCircle,
  AlertTriangle,
  HelpCircle,
  ArrowRight,
  X,
} from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { showError, showInfo, showSuccess } from "../stores/toastStore";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Skeleton } from "../components/ui/Skeleton";
import type { ConflictContent, ConflictResolution, SyncAllResult, SyncTarget } from "../types";

const SYNC_HEALTHY_THRESHOLD_MS = 24 * 60 * 60 * 1000;

type HealthLevel = "normal" | "warning" | "danger";

interface HealthState {
  level: HealthLevel;
  title: string;
  message: string;
  suggestedAction: string;
}

function computeHealth(
  syncTotal: number,
  conflictCount: number,
  lastSyncAt: number | null
): HealthState {
  if (conflictCount > 0) {
    return {
      level: "danger",
      title: `${conflictCount} 个冲突待处理`,
      message: "部分 Skill 在 Agent 与中心仓库之间出现内容冲突，需要手动处理。",
      suggestedAction: "处理冲突",
    };
  }

  if (syncTotal === 0) {
    return {
      level: "warning",
      title: "尚未建立同步关系",
      message: "当前没有 Skill 与 Agent 的同步目标，建议先添加 Skill 并启用 Agent。",
      suggestedAction: "添加 Skill",
    };
  }

  if (!lastSyncAt || Date.now() - lastSyncAt > SYNC_HEALTHY_THRESHOLD_MS) {
    const timeText = lastSyncAt ? formatTimeAgo(lastSyncAt) : "从未";
    return {
      level: "warning",
      title: `建议立即同步（上次同步 ${timeText}）`,
      message: "同步记录已超过 24 小时，可能无法反映最新的 Skill 变更。",
      suggestedAction: "立即同步",
    };
  }

  return {
    level: "normal",
    title: "一切正常",
    message: `所有 ${syncTotal} 个同步目标均处于最新状态。`,
    suggestedAction: "立即同步",
  };
}

function formatTimeAgo(ts: number): string {
  const diff = Math.max(0, Date.now() - ts);
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  return `${Math.floor(diff / 86_400_000)} 天前`;
}

async function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    console.error(`[SkillMint] ${cmd} failed:`, err);
    const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
    showError(`操作失败：${message}`);
    throw err;
  }
}

export default function Dashboard() {
  const skills = useAppStore((state) => state.skills);
  const agents = useAppStore((state) => state.agents);
  const syncTargets = useAppStore((state) => state.syncTargets);
  const settings = useAppStore((state) => state.settings);
  const loadData = useAppStore((state) => state.loadData);
  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const setSkillLibrarySubTab = useAppStore((state) => state.setSkillLibrarySubTab);

  const [loading, setLoading] = useState<"sync" | "scan" | null>(null);
  const [selectedConflict, setSelectedConflict] = useState<SyncTarget | null>(null);

  const conflictTargets = useMemo(
    () => syncTargets.filter((t) => t.status === "conflict"),
    [syncTargets]
  );
  const conflictCount = conflictTargets.length;
  const syncedCount = syncTargets.filter((t) => t.status === "synced").length;
  const syncTotal = syncTargets.length;
  const lastSyncAt = useMemo(() => {
    if (syncTargets.length === 0) return null;
    // Backend stores last_sync_at as Unix seconds; UI expects JS epoch millis.
    const timestamps = syncTargets
      .map((t) => (typeof t.last_sync_at === "number" ? t.last_sync_at * 1000 : null))
      .filter((ts): ts is number => ts !== null);
    return timestamps.length > 0 ? Math.max(...timestamps) : null;
  }, [syncTargets]);

  const health = useMemo(
    () => computeHealth(syncTotal, conflictCount, lastSyncAt),
    [syncTotal, conflictCount, lastSyncAt]
  );

  const handleSync = async () => {
    if (loading) return;
    setLoading("sync");
    try {
      const result = await safeInvoke<SyncAllResult>("sync_all_command");
      await loadData();
      const targets = result.targets;
      const total = targets.length;
      const synced = targets.filter((t) => t.status === "synced").length;
      const conflicts = targets.filter((t) => t.status === "conflict").length;
      const localChanged = targets.filter((t) => t.status === "local_changed").length;

      const parts: string[] = [];
      if (result.imported_skills > 0) {
        parts.push(`新导入 ${result.imported_skills} 个 Skill`);
      }
      if (result.import_conflicts > 0) {
        parts.push(`${result.import_conflicts} 个同名 Skill 冲突未导入`);
      }
      if (total > 0) {
        parts.push(`同步 ${synced}/${total}`);
        if (conflicts > 0) parts.push(`${conflicts} 个冲突`);
        if (localChanged > 0) parts.push(`${localChanged} 个本地变更`);
      }

      if (parts.length === 0) {
        showInfo("未发现可同步的 Skill，请先在 Skill 库中创建或导入");
      } else {
        showSuccess(parts.join("，"));
      }
    } finally {
      setLoading(null);
    }
  };

  const handleScanAgents = async () => {
    if (loading) return;
    setLoading("scan");
    try {
      const scanned = await safeInvoke<{ id: string }[]>("scan_agents");
      await loadData();
      showSuccess(`扫描完成，发现 ${scanned.length} 个 Agent 目录`);
    } finally {
      setLoading(null);
    }
  };

  const handleAddSkill = () => {
    setActiveTab("skillLibrary");
    showInfo("已切换到 Skill 库页面");
  };

  const handleViewConflicts = () => {
    setActiveTab("skillLibrary");
    setSkillLibrarySubTab("skills");
  };

  const subtitle = lastSyncAt
    ? `上次同步：${formatTimeAgo(lastSyncAt)}`
    : "尚未完成过同步";

  return (
    <div className="h-full overflow-auto p-8">
      <div className="mb-6 flex items-end justify-between">
        <div>
          <h1 className="text-3xl font-semibold tracking-tight text-primary">概览</h1>
          <p className="mt-1 text-sm text-secondary">{subtitle}</p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={handleScanAgents}
            loading={loading === "scan"}
            disabled={loading !== null}
            title="扫描本机 Agent 目录（不会读取会话内容）"
          >
            <ScanSearch className="h-4 w-4" />
            扫描 Agent 目录
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={handleAddSkill}
            disabled={loading !== null}
          >
            <Plus className="h-4 w-4" />
            添加 Skill
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={handleSync}
            loading={loading === "sync"}
            disabled={loading !== null}
            title="扫描 Agent → 首次导入 Skill → 按 hash 增量同步到 Agent"
          >
            <RefreshCw className={`h-4 w-4 ${loading === "sync" ? "animate-spin" : ""}`} />
            立即同步
          </Button>
        </div>
      </div>

      <div className="mb-6 grid grid-cols-1 gap-4 lg:grid-cols-3">
        <HealthCard
          health={health}
          onPrimaryAction={health.level === "danger" ? handleViewConflicts : handleSync}
          loading={loading === "sync"}
          className="lg:col-span-1"
        />

        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:col-span-2">
          <StatCard
            title="Skill 总数"
            value={skills.length}
            color="accent"
            icon={Puzzle}
            description="中心仓库中已管理的 Skill 数量。"
          />
          <StatCard
            title="已启用 Agent"
            value={agents.filter((a) => a.is_enabled).length}
            color="success"
            icon={Bot}
            description="当前已启用同步的 Agent 目录数量。"
          />
          <StatCard
            title="已同步"
            value={syncTotal > 0 ? Math.round((syncedCount / syncTotal) * 100) : 0}
            color="accent"
            icon={CheckCircle2}
            description={`${syncedCount}/${syncTotal} 个同步目标处于已同步状态。`}
            suffix="%"
          />
          <StatCard
            title="冲突"
            value={conflictCount}
            color={conflictCount > 0 ? "danger" : "success"}
            icon={AlertTriangle}
            description={conflictCount > 0 ? "需要处理的 Skill 冲突数量。" : "当前没有冲突。"}
          />
        </div>
      </div>

      {conflictCount > 0 && (
        <Card className="border-danger/20 bg-danger/5" padding="lg">
          <div className="mb-3 flex items-center justify-between">
            <div className="flex items-center gap-2 text-danger">
              <AlertTriangle className="h-5 w-5" />
              <span className="font-semibold">待处理冲突</span>
            </div>
            <Button variant="danger" size="sm" onClick={handleViewConflicts}>
              查看全部
              <ArrowRight className="ml-1 h-4 w-4" />
            </Button>
          </div>
          <div className="space-y-2">
            {conflictTargets.slice(0, 5).map((target) => (
              <ConflictRow key={target.id} target={target} onClick={() => setSelectedConflict(target)} />
            ))}
            {conflictCount > 5 && (
              <p className="text-xs text-tertiary">还有 {conflictCount - 5} 个冲突未显示</p>
            )}
          </div>
        </Card>
      )}

      {selectedConflict && (
        <ConflictModal
          target={selectedConflict}
          onClose={() => setSelectedConflict(null)}
          onResolved={() => {
            setSelectedConflict(null);
            loadData();
          }}
        />
      )}

      {syncTotal === 0 && (
        <Card className="p-10 text-center">
          <div className="text-lg font-medium text-primary">开始使用 SkillMint</div>
          <p className="mt-2 text-sm text-secondary">
            中心仓库路径：{settings.center_repo || "~/.skillmint/repo"}
          </p>
          <div className="mt-4 flex justify-center gap-2">
            <Button variant="secondary" size="sm" onClick={handleScanAgents}>
              扫描 Agent
            </Button>
            <Button variant="primary" size="sm" onClick={handleAddSkill}>
              添加第一个 Skill
            </Button>
          </div>
        </Card>
      )}
    </div>
  );
}

function ConflictRow({ target, onClick }: { target: SyncTarget; onClick?: () => void }) {
  return (
    <div
      className={`flex items-center justify-between rounded-lg bg-danger/10 px-3 py-2 ${onClick ? "cursor-pointer hover:bg-danger/20" : ""}`}
      onClick={onClick}
    >
      <div className="min-w-0 flex-1">
        <span className="truncate text-sm font-medium text-primary">{target.skill_name || target.skill_id}</span>
        <span className="mx-2 text-tertiary">·</span>
        <span className="truncate text-sm text-secondary">{target.agent_name || target.agent_id}</span>
      </div>
      <span className="shrink-0 rounded bg-danger/20 px-2 py-0.5 text-xs font-medium text-danger">冲突</span>
    </div>
  );
}

function ConflictModal({
  target,
  onClose,
  onResolved,
}: {
  target: SyncTarget;
  onClose: () => void;
  onResolved: () => void;
}) {
  const [content, setContent] = useState<ConflictContent | null>(null);
  const [resolving, setResolving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<ConflictContent>("get_conflict_contents", { syncTargetId: target.id })
      .then((c) => {
        if (!cancelled) setContent(c);
      })
      .catch((err) => {
        const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`读取冲突内容失败：${message}`);
        onClose();
      });
    return () => {
      cancelled = true;
    };
  }, [target.id, onClose]);

  const resolve = async (resolution: ConflictResolution) => {
    if (resolving) return;
    setResolving(true);
    try {
      await invoke("resolve_conflict", { payload: { sync_target_id: target.id, resolution } });
      showSuccess("冲突已处理");
      onResolved();
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`处理失败：${message}`);
    } finally {
      setResolving(false);
    }
  };

  const timeText = (ts?: number) => (ts ? new Date(ts * 1000).toLocaleString() : "未知");

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div className="max-h-[90vh] w-full max-w-3xl overflow-hidden rounded-2xl border border-[var(--border-subtle)] bg-secondary shadow-xl">
        <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-6 py-4">
          <div>
            <h2 className="text-lg font-semibold text-primary">处理冲突</h2>
            <p className="text-xs text-secondary">{content?.skill_name || target.skill_name} · {content?.agent_name || target.agent_name}</p>
          </div>
          <button onClick={onClose} className="rounded p-1 text-tertiary hover:bg-tertiary hover:text-primary">
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="grid h-[60vh] grid-cols-2 divide-x divide-[var(--border-subtle)] overflow-hidden">
          <div className="flex flex-col">
            <div className="border-b border-[var(--border-subtle)] bg-primary/50 px-4 py-2">
              <div className="text-sm font-medium text-primary">中心仓库</div>
              <div className="text-xs text-secondary">更新于 {timeText(content?.center_updated_at)}</div>
              {content?.center_author && <div className="text-xs text-tertiary">作者：{content.center_author}</div>}
            </div>
            {content?.center_content ? (
              <pre className="flex-1 overflow-auto whitespace-pre-wrap p-4 text-xs text-secondary">{content.center_content}</pre>
            ) : (
              <div className="flex-1 space-y-2 p-4">
                <Skeleton className="h-3 w-3/4" />
                <Skeleton className="h-3 w-1/2" />
                <Skeleton className="h-3 w-2/3" />
              </div>
            )}
          </div>
          <div className="flex flex-col">
            <div className="border-b border-[var(--border-subtle)] bg-primary/50 px-4 py-2">
              <div className="text-sm font-medium text-primary">本地 Agent</div>
              <div className="text-xs text-secondary">更新于 {timeText(content?.local_updated_at)}</div>
              {content?.local_author && <div className="text-xs text-tertiary">作者：{content.local_author}</div>}
            </div>
            {content?.local_content ? (
              <pre className="flex-1 overflow-auto whitespace-pre-wrap p-4 text-xs text-secondary">{content.local_content}</pre>
            ) : (
              <div className="flex-1 space-y-2 p-4">
                <Skeleton className="h-3 w-3/4" />
                <Skeleton className="h-3 w-1/2" />
                <Skeleton className="h-3 w-2/3" />
              </div>
            )}
          </div>
        </div>

        <div className="flex justify-end gap-2 border-t border-[var(--border-subtle)] px-6 py-4">
          <Button variant="ghost" size="sm" onClick={onClose} disabled={resolving}>
            取消
          </Button>
          <Button variant="secondary" size="sm" onClick={() => resolve("skip")} loading={resolving}>
            跳过
          </Button>
          <Button variant="primary" size="sm" onClick={() => resolve("keep_local")} loading={resolving}>
            保留本地
          </Button>
          <Button variant="primary" size="sm" onClick={() => resolve("keep_center")} loading={resolving}>
            保留中心
          </Button>
        </div>
      </div>
    </div>
  );
}

function HealthCard({
  health,
  onPrimaryAction,
  loading,
  className,
}: {
  health: HealthState;
  onPrimaryAction: () => void;
  loading: boolean;
  className?: string;
}) {
  const meta = {
    normal: {
      border: "border-success/20",
      bg: "bg-success/5",
      text: "text-success",
      icon: CheckCircle2,
      buttonVariant: "primary" as const,
    },
    warning: {
      border: "border-warning/20",
      bg: "bg-warning/5",
      text: "text-warning",
      icon: AlertCircle,
      buttonVariant: "primary" as const,
    },
    danger: {
      border: "border-danger/20",
      bg: "bg-danger/5",
      text: "text-danger",
      icon: AlertTriangle,
      buttonVariant: "danger" as const,
    },
  };

  const style = meta[health.level];
  const Icon = style.icon;

  return (
    <Card
      className={`flex flex-col justify-between border ${style.border} ${style.bg} ${className || ""}`}
      padding="lg"
    >
      <div>
        <div className={`mb-3 flex h-10 w-10 items-center justify-center rounded-xl ${style.bg}`}>
          <Icon className={`h-6 w-6 ${style.text}`} />
        </div>
        <h2 className={`text-xl font-semibold ${style.text}`}>{health.title}</h2>
        <p className="mt-1 text-sm text-secondary">{health.message}</p>
      </div>
      <div className="mt-4">
        <Button
          variant={style.buttonVariant}
          size="sm"
          onClick={onPrimaryAction}
          loading={loading}
          disabled={loading}
        >
          {health.suggestedAction}
        </Button>
      </div>
    </Card>
  );
}

const StatCard = memo(function StatCard({
  title,
  value,
  color,
  icon: Icon,
  description,
  suffix,
}: {
  title: string;
  value: number;
  color: "accent" | "success" | "warning" | "danger";
  icon: React.ElementType;
  description?: string;
  suffix?: string;
}) {
  const meta = {
    accent: { border: "border-accent/20", bg: "bg-accent/5", text: "text-accent", icon: "text-accent" },
    success: { border: "border-success/20", bg: "bg-success/5", text: "text-success", icon: "text-success" },
    warning: { border: "border-warning/20", bg: "bg-warning/5", text: "text-warning", icon: "text-warning" },
    danger: { border: "border-danger/20", bg: "bg-danger/5", text: "text-danger", icon: "text-danger" },
  };

  const style = meta[color];

  return (
    <Card
      className={`group relative overflow-hidden border ${style.border} ${style.bg} transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md`}
      padding="lg"
    >
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-1.5 text-sm font-medium text-secondary">
            {title}
            {description && (
              <span className="group/tooltip relative cursor-help">
                <HelpCircle className="h-4 w-4 text-tertiary" />
                <span className="pointer-events-none absolute bottom-full left-1/2 z-10 mb-2 w-56 -translate-x-1/2 rounded-lg border border-[var(--border-subtle)] bg-elevated p-2 text-xs text-secondary opacity-0 shadow-lg transition-opacity group-hover/tooltip:opacity-100">
                  {description}
                  <span className="absolute left-1/2 top-full -translate-x-1/2 border-4 border-transparent border-t-elevated" />
                </span>
              </span>
            )}
          </div>
          <div className={`mt-2 text-3xl font-bold tracking-tight ${style.text}`}>
            {value}
            {suffix && <span className="text-lg">{suffix}</span>}
          </div>
        </div>
        <div className={`rounded-xl ${style.bg} p-2.5`}>
          <Icon className={`h-5 w-5 ${style.icon}`} />
        </div>
      </div>
    </Card>
  );
});
