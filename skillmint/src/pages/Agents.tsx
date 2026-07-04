import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { invoke } from "../lib/invoke";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Bot,
  FolderOpen,
  Loader2,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { showError, showInfo, showSuccess } from "../stores/toastStore";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { EmptyState } from "../components/ui/EmptyState";
import { SkeletonCard, SkeletonList } from "../components/ui/Skeleton";
import { VirtualList } from "../components/ui/VirtualList";
import type { AgentDetail, AgentDirectory, AgentSkillItem } from "../types";

/**
 * SPEC-F6 T1: 返回单个 Agent 的显示名。
 * - 无 allAgents 上下文：返回原名（无名为退回到目录末段）。
 * - 有 allAgents：当且仅当原名在列表内出现 >1 次时才消歧，依次追加
 *   目录末段 → 目录末两段 → id 短后缀，直到列表内唯一。
 * 保证：对任意输入，同一列表内每个 Agent 的 agentDisplayName 互不相同
 *（只要它们 id 互不相同；id 相同时回退到目录路径区分）。
 *
 * 防御性：name / skill_directory 均可能缺失（mock 或未来字段变动），全程不抛。
 */
export function agentDisplayName(
  agent: { name?: string; skill_directory?: string; id?: string },
  allAgents?: { name?: string; skill_directory?: string; id?: string }[]
): string {
  const raw = (agent.name ?? "").trim();
  const segments = dirSegments(agent.skill_directory);
  const lastSegment = segments[segments.length - 1] || "";
  const lastTwoSegments = segments.slice(-2).filter(Boolean).join("/");
  const fallback = lastSegment || "未命名 Agent";

  // 无 allAgents 上下文：无法判断重名，直接返回原名（向后兼容旧调用点）。
  if (!allAgents || allAgents.length === 0) {
    return raw || fallback;
  }

  // 起步候选：有名用原名，无名用目录末段。
  const base = raw || fallback;

  // 仅当 base 在列表内出现 >1 次（统计 raw 与 fallback 两种口径）才消歧。
  const sameBaseCount = allAgents.filter((a) => {
    const aRaw = (a.name ?? "").trim();
    const aSegs = dirSegments(a.skill_directory);
    const aFallback = aSegs[aSegs.length - 1] || "未命名 Agent";
    return (aRaw || aFallback) === base;
  }).length;

  if (sameBaseCount <= 1) return base;

  // 三层候选，逐层判断「在列表内唯一」。比较口径统一用 sameAgentKey。
  const candidates: string[] = [];
  if (lastSegment && lastSegment !== base) candidates.push(`${base} · ${lastSegment}`);
  if (lastTwoSegments && lastTwoSegments !== lastSegment) {
    candidates.push(raw ? `${raw} · ${lastTwoSegments}` : lastTwoSegments);
  }
  if (agent.id) {
    // SPEC-F6 T1 兜底：4 位短后缀仍冲突时（如多个 *-code），逐步加长 id 尾部直到唯一。
    const idTailBase = agent.id.replace(/[^a-zA-Z0-9]/g, "");
    for (let len = 4; len <= idTailBase.length; len++) {
      candidates.push(`${base} · ${idTailBase.slice(-len)}`);
    }
  }

  for (const candidate of candidates) {
    if (isCandidateUnique(candidate, agent, allAgents)) return candidate;
  }
  // 兜底：若所有候选都不唯一（理论上只有 id 完全相同时发生），返回最后的候选。
  return candidates[candidates.length - 1] ?? base;
}

/** 内部：安全解析 skill_directory 为路径段数组，缺失/非字符串时返回 []。 */
function dirSegments(dir: unknown): string[] {
  if (typeof dir !== "string" || dir.length === 0) return [];
  try {
    return dir.replace(/\\/g, "/").split("/").filter(Boolean);
  } catch {
    return [];
  }
}

/** 内部：判断 candidate 作为显示名时，在 allAgents 中除自身外不与任何 Agent 冲突。 */
function isCandidateUnique(
  candidate: string,
  agent: { name?: string; skill_directory?: string; id?: string },
  allAgents: { name?: string; skill_directory?: string; id?: string }[],
): boolean {
  // 计算 agent 自身的 key（用于跳过自己）。
  const selfKey = agent.id ?? agent.skill_directory ?? "";
  for (const a of allAgents) {
    const aKey = a.id ?? a.skill_directory ?? "";
    if (aKey === selfKey && aKey !== "") continue;
    // 如果 a 的原名/任何消歧候选也等于 candidate，则视为冲突。
    if (rawOrCandidates(a).includes(candidate)) return false;
  }
  return true;
}

/** 内部：给定一个 Agent，枚举它的所有可能显示候选（含原名与各层消歧名），用于冲突检测。
 * 必须与 agentDisplayName 的候选生成保持一致（含 id 尾部逐步加长）。 */
function rawOrCandidates(a: { name?: string; skill_directory?: string; id?: string }): string[] {
  const raw = (a.name ?? "").trim();
  const segs = dirSegments(a.skill_directory);
  const last = segs[segs.length - 1] || "";
  const lastTwo = segs.slice(-2).filter(Boolean).join("/");
  const fallback = last || "未命名 Agent";
  const base = raw || fallback;
  const list = [base];
  if (last && last !== base) list.push(`${base} · ${last}`);
  if (lastTwo && lastTwo !== last) {
    list.push(raw ? `${raw} · ${lastTwo}` : lastTwo);
  }
  if (a.id) {
    const idTailBase = a.id.replace(/[^a-zA-Z0-9]/g, "");
    for (let len = 4; len <= idTailBase.length; len++) {
      list.push(`${base} · ${idTailBase.slice(-len)}`);
    }
  }
  return list;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function timeAgo(ts?: number): string {
  if (!ts) return "—";
  const diff = Math.max(0, Date.now() / 1000 - ts);
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

export default function Agents() {
  const agents = useAppStore((state) => state.agents);
  const loadData = useAppStore((state) => state.loadData);
  const [scanning, setScanning] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [skillCounts, setSkillCounts] = useState<Record<string, number>>({});
  const [countsLoading, setCountsLoading] = useState(true);

  const handleScan = useCallback(async () => {
    if (scanning) return;
    setScanning(true);
    try {
      const scanned = await invoke<{ id: string }[]>("scan_agents");
      await loadData();
      showSuccess(`扫描完成，发现 ${scanned.length} 个智能体目录`);
    } catch (err) {
      const message =
        typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`扫描失败：${message}`);
    } finally {
      setScanning(false);
    }
  }, [scanning, loadData]);

  // Batch-load skill counts once per agent list change.
  useEffect(() => {
    if (agents.length === 0) {
      setCountsLoading(false);
      return;
    }
    let cancelled = false;
    setCountsLoading(true);
    invoke<Record<string, number>>("get_agent_skill_counts", {
      agentIds: agents.map((a) => a.id),
    })
      .then((counts) => {
        if (!cancelled) {
          setSkillCounts(counts);
          setCountsLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setCountsLoading(false);
          showError(typeof err === "string" ? err : String(err));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [agents]);

  const handleSelect = useCallback((id: string) => {
    setSelectedId(id);
  }, []);

  const handleBack = useCallback(() => {
    setSelectedId(null);
  }, []);

  if (selectedId) {
    return <AgentDetailView agentId={selectedId} onBack={handleBack} />;
  }

  return (
    <div className="flex h-full flex-col p-8">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-semibold tracking-tight text-primary">智能体目录</h1>
          <p className="mt-1 text-sm text-secondary">管理本地智能体目录与同步关系</p>
        </div>
        <Button
          variant="primary"
          size="sm"
          onClick={handleScan}
          loading={scanning}
          disabled={scanning}
        >
          <RefreshCw className={`h-4 w-4 ${scanning ? "animate-spin" : ""}`} />
          {scanning ? "扫描中…" : "重新扫描"}
        </Button>
      </div>

      {agents.length === 0 ? (
        <EmptyState
          icon={Bot}
          illustration="generic"
          title="未发现本地智能体目录"
          description="安装智能体后点击右上角「重新扫描」即可发现。"
          action={
            <Button variant="primary" size="sm" onClick={handleScan} loading={scanning}>
              <RefreshCw className={`h-4 w-4 ${scanning ? "animate-spin" : ""}`} />
              重新扫描
            </Button>
          }
        />
      ) : (
        <Card className="flex-1 overflow-hidden p-0" variant="default" padding="none">
          <VirtualList
            items={agents}
            estimateSize={88}
            className="h-full"
            renderItem={(agent) => (
              <AgentListItem
                agent={agent}
                displayName={agentDisplayName(agent, agents)}
                skillCount={skillCounts[agent.id]}
                countsLoading={countsLoading}
                onClick={handleSelect}
              />
            )}
            getItemKey={(agent) => agent.id}
          />
        </Card>
      )}
    </div>
  );
}

const AgentListItem = memo(function AgentListItem({
  agent,
  displayName,
  skillCount,
  countsLoading,
  onClick,
}: {
  agent: { id: string; name: string; description?: string; skill_directory: string; is_enabled: boolean };
  displayName: string;
  skillCount?: number;
  countsLoading: boolean;
  onClick: (id: string) => void;
}) {
  const handleClick = useCallback(() => onClick(agent.id), [onClick, agent.id]);

  return (
    <div
      className="group flex cursor-pointer items-center justify-between border-b border-[var(--divider)] px-6 py-5 transition-colors last:border-b-0 hover:bg-tertiary/40"
      onClick={handleClick}
    >
      <div className="flex min-w-0 flex-1 items-center gap-4 pr-4">
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-accent/10 text-accent">
          <Bot className="h-5 w-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate font-medium text-primary">{displayName}</span>
            <Badge variant={agent.is_enabled ? "success" : "default"} size="sm">
              {agent.is_enabled ? "已启用" : "已禁用"}
            </Badge>
          </div>
          {agent.description && (
            <div className="mt-0.5 truncate text-sm text-secondary">{agent.description}</div>
          )}
          <div className="mt-0.5 truncate text-sm text-tertiary">{agent.skill_directory}</div>
        </div>
      </div>
      <div className="shrink-0 text-right">
        <div className="text-sm font-medium text-primary">
          {countsLoading || skillCount === undefined ? (
            <span className="inline-flex items-center gap-1.5 text-secondary">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              统计中
            </span>
          ) : (
            `${skillCount} 个 Skill`
          )}
        </div>
        <div className="mt-0.5 text-xs text-secondary">点击查看详情</div>
      </div>
    </div>
  );
});

function AgentDetailView({ agentId, onBack }: { agentId: string; onBack: () => void }) {
  const [detail, setDetail] = useState<AgentDetail | null>(null);
  const [tab, setTab] = useState<"overview" | "projects" | "directories" | "skill">("overview");
  const [loading, setLoading] = useState(true);

  const refreshDetail = useCallback(async () => {
    try {
      const d = await invoke<AgentDetail>("get_agent_detail", { agentId });
      setDetail(d);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`加载智能体详情失败：${msg}`);
    }
  }, [agentId]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    refreshDetail().then(() => {
      if (!cancelled) setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [refreshDetail]);

  if (loading || !detail) {
    return (
      <div className="h-full overflow-auto p-8">
        <BackButton onBack={onBack} />
        <SkeletonCard />
      </div>
    );
  }

  const dirCount = detail.directories?.length ?? 0;

  return (
    <div className="h-full overflow-auto p-8">
      <BackButton onBack={onBack} />

      <Card className="mb-6" padding="lg">
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-4">
            <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-accent/10 text-accent">
              <Bot className="h-6 w-6" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-2xl font-semibold text-primary">{agentDisplayName(detail)}</h1>
                <Badge variant={detail.is_enabled ? "success" : "default"} size="sm">
                  {detail.is_enabled ? "已启用" : "已禁用"}
                </Badge>
              </div>
              <div className="mt-1 text-sm text-tertiary">{detail.skill_directory}</div>
              {detail.description && (
                <div className="mt-1 text-sm text-secondary">{detail.description}</div>
              )}
            </div>
          </div>
        </div>
        <div className="mt-4 flex flex-wrap gap-x-6 gap-y-1 text-sm text-secondary">
          <span>关联项目：{detail.project_count}</span>
          <span>最近使用：{timeAgo(detail.last_used_at)}</span>
          {detail.source && <span>采集源：{detail.source}</span>}
          {dirCount > 1 && <span>目录：{dirCount} 个</span>}
        </div>
      </Card>

      <div className="mb-4 flex gap-1 border-b border-[var(--border-subtle)]">
        {(
          [
            ["overview", "概览"],
            ["projects", "项目"],
            ["directories", `目录${dirCount ? ` (${dirCount})` : ""}`],
            ["skill", "技能"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            onClick={() => setTab(key)}
            className={`-mb-px border-b-2 px-4 py-2 text-sm font-medium transition-colors ${
              tab === key
                ? "border-accent text-accent"
                : "border-transparent text-secondary hover:text-primary"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {tab === "overview" && (
        <Card padding="lg">
          {detail.usage_7d ? (
            <div className="grid grid-cols-3 gap-4">
              <MetricCard label="近 7 天会话" value={String(detail.usage_7d.session_count)} />
              <MetricCard label="近 7 天 Prompt" value={String(detail.usage_7d.prompt_count)} />
              <MetricCard label="近 7 天 Token" value={formatTokens(detail.usage_7d.total_tokens)} />
            </div>
          ) : (
            <EmptyState
              icon={Bot}
              title="暂无使用数据"
              description="前往「使用洞察」点「立即采集」可读取该智能体的本地会话数据。"
            />
          )}
        </Card>
      )}

      {tab === "projects" && (
        <Card padding="none">
          {detail.projects.length === 0 ? (
            <div className="p-6 text-sm text-secondary">
              该智能体暂未发现关联项目。采集数据后项目会自动出现。
            </div>
          ) : (
            <ul className="divide-y divide-[var(--divider)]">
              {detail.projects.map((p) => (
                <li key={p.project_id} className="flex items-center justify-between px-6 py-4 text-sm">
                  <div>
                    <div className="font-medium text-primary">{p.name}</div>
                    <div className="mt-1 text-xs text-tertiary">{p.path}</div>
                  </div>
                  <div className="text-right text-secondary">
                    <div>{p.session_count} 会话</div>
                    <div>{formatTokens(p.total_tokens)} tokens</div>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </Card>
      )}

      {tab === "directories" && (
        <AgentDirectoryList
          agentId={agentId}
          directories={detail.directories ?? []}
          onChanged={refreshDetail}
        />
      )}

      {tab === "skill" && <AgentSkillList agentId={agentId} />}
    </div>
  );
}

function BackButton({ onBack }: { onBack: () => void }) {
  return (
    <button
      onClick={onBack}
      className="mb-4 inline-flex items-center gap-1 rounded-lg px-2 py-1 text-sm text-secondary transition-colors hover:bg-tertiary/60 hover:text-primary"
    >
      <ArrowLeft className="h-4 w-4" />
      返回列表
    </button>
  );
}

function AgentDirectoryList({
  agentId,
  directories,
  onChanged,
}: {
  agentId: string;
  directories: AgentDirectory[];
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [roleDraft, setRoleDraft] = useState("");
  const [dirSkills, setDirSkills] = useState<Record<string, AgentSkillItem[]>>({});
  const [rescanningId, setRescanningId] = useState<string | null>(null);
  const [togglingId, setTogglingId] = useState<string | null>(null);
  const [removingId, setRemovingId] = useState<string | null>(null);

  const handleAdd = useCallback(async () => {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || Array.isArray(selected)) return;
    if (busy) return;
    setBusy(true);
    try {
      await invoke("add_agent_directory", { agentId, path: selected, role: null });
      showSuccess("已添加目录");
      onChanged();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`添加目录失败：${msg}`);
    } finally {
      setBusy(false);
    }
  }, [agentId, busy, onChanged]);

  const handleRemove = useCallback(
    async (dir: AgentDirectory) => {
      const ok = window.confirm(
        `确定解绑目录「${dir.path}」吗？\n该目录下的同步关系将被一并移除（磁盘文件不受影响）。`,
      );
      if (!ok) return;
      if (removingId) return;
      setRemovingId(dir.id);
      try {
        const detached = await invoke<number>("remove_agent_directory", { directoryId: dir.id });
        if (detached > 0) {
          showInfo(`已解绑目录，并移除 ${detached} 条同步关系`);
        } else {
          showSuccess("已解绑目录");
        }
        onChanged();
      } catch (err) {
        const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`解绑目录失败：${msg}`);
      } finally {
        setRemovingId(null);
      }
    },
    [removingId, onChanged],
  );

  const handleToggle = useCallback(
    async (dir: AgentDirectory) => {
      if (togglingId || busy) return;
      setTogglingId(dir.id);
      try {
        await invoke("update_agent_directory", {
          directoryId: dir.id,
          role: null,
          isEnabled: !dir.is_enabled,
        });
        onChanged();
      } catch (err) {
        const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`切换启用状态失败：${msg}`);
      } finally {
        setTogglingId(null);
      }
    },
    [togglingId, busy, onChanged],
  );

  const startEditRole = useCallback((dir: AgentDirectory) => {
    setEditingId(dir.id);
    setRoleDraft(dir.role ?? "skills");
  }, []);

  const cancelEditRole = useCallback(() => {
    setEditingId(null);
    setRoleDraft("");
  }, []);

  const handleSaveRole = useCallback(
    async (dir: AgentDirectory) => {
      const trimmed = roleDraft.trim();
      if (!trimmed) {
        showError("role 不能为空");
        return;
      }
      if (trimmed === (dir.role ?? "skills")) {
        cancelEditRole();
        return;
      }
      if (busy) return;
      setBusy(true);
      try {
        await invoke("update_agent_directory", {
          directoryId: dir.id,
          role: trimmed,
          isEnabled: null,
        });
        cancelEditRole();
        onChanged();
      } catch (err) {
        const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`修改 role 失败：${msg}`);
      } finally {
        setBusy(false);
      }
    },
    [roleDraft, busy, cancelEditRole, onChanged],
  );

  // Load cached scan results for all directories on first mount / when changed.
  useEffect(() => {
    let cancelled = false;
    Promise.all(
      directories.map((dir) =>
        invoke<AgentSkillItem[]>("scan_directory_skills", {
          agentId,
          path: dir.path,
          force: false,
        }).then((items) => ({ id: dir.id, items })),
      ),
    )
      .then((results) => {
        if (cancelled) return;
        const next: Record<string, AgentSkillItem[]> = {};
        for (const r of results) next[r.id] = r.items;
        setDirSkills(next);
      })
      .catch((err) => showError(typeof err === "string" ? err : String(err)));
    return () => {
      cancelled = true;
    };
  }, [agentId, directories]);

  const handleRescan = useCallback(
    async (dir: AgentDirectory) => {
      if (rescanningId) return;
      setRescanningId(dir.id);
      try {
        const items = await invoke<AgentSkillItem[]>("scan_directory_skills", {
          agentId,
          path: dir.path,
          force: true,
        });
        setDirSkills((prev) => ({ ...prev, [dir.id]: items }));
        const inCenter = items.filter((i) => i.exists_in_center).length;
        showSuccess(`${dir.path} 下有 ${items.length} 个技能（${inCenter} 已在中心）`);
      } catch (err) {
        const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`扫描目录失败：${msg}`);
      } finally {
        setRescanningId(null);
      }
    },
    [rescanningId, agentId],
  );

  const dirStats = useMemo(() => {
    const stats: Record<string, { total: number; synced: number; diff: number; local: number }> = {};
    for (const d of directories) {
      const items = dirSkills[d.id] ?? [];
      let synced = 0;
      let diff = 0;
      let local = 0;
      for (const i of items) {
        if (!i.exists_in_center) {
          local += 1;
        } else if (i.content_match === false) {
          diff += 1;
        } else {
          synced += 1;
        }
      }
      stats[d.id] = { total: items.length, synced, diff, local };
    }
    return stats;
  }, [directories, dirSkills]);

  return (
    <Card padding="none">
      <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-6 py-3">
        <span className="text-sm text-secondary">该智能体拥有 {directories.length} 个目录</span>
        <Button variant="secondary" size="sm" onClick={handleAdd} loading={busy} disabled={busy}>
          <Plus className="h-4 w-4" />
          添加目录
        </Button>
      </div>
      {directories.length === 0 ? (
        <div className="p-6 text-sm text-secondary">
          该智能体暂无目录记录。点「添加目录」为它绑定一个目录。
        </div>
      ) : (
        <ul className="divide-y divide-[var(--divider)]">
          {directories.map((d) => (
            <li key={d.id} className="flex items-center justify-between px-6 py-4 text-sm">
              <div className="min-w-0 flex-1 pr-4">
                <div className="flex flex-wrap items-center gap-2">
                  {editingId === d.id ? (
                    <>
                      <input
                        autoFocus
                        value={roleDraft}
                        onChange={(e) => setRoleDraft(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleSaveRole(d);
                          if (e.key === "Escape") cancelEditRole();
                        }}
                        placeholder="skills / commands / rules / ..."
                        className="w-40 rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1 text-sm text-primary outline-none focus:border-accent"
                      />
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => handleSaveRole(d)}
                        loading={busy}
                        disabled={busy}
                      >
                        保存
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={cancelEditRole}
                        disabled={busy}
                      >
                        取消
                      </Button>
                    </>
                  ) : (
                    <>
                      <span className="font-medium text-primary">{d.role ?? "skills"}</span>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => startEditRole(d)}
                        disabled={busy || editingId !== null}
                      >
                        编辑
                      </Button>
                    </>
                  )}
                  {!d.is_enabled && <Badge variant="default">已禁用</Badge>}
                </div>
                <div className="mt-1 truncate text-xs text-tertiary">{d.path}</div>
                {dirSkills[d.id] && (
                  <div className="mt-1 text-xs text-secondary">
                    {dirStats[d.id].total} 个技能
                    <span className="ml-2">
                      （{dirStats[d.id].synced} 已同步 · {dirStats[d.id].diff} 有差异 ·{" "}
                      {dirStats[d.id].local} 未导入中心）
                    </span>
                  </div>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleRescan(d)}
                  loading={rescanningId === d.id}
                  disabled={rescanningId !== null || togglingId !== null || removingId !== null}
                >
                  <RefreshCw
                    className={`h-4 w-4 ${rescanningId === d.id ? "animate-spin" : ""}`}
                  />
                  {rescanningId === d.id ? "扫描中" : "重新扫描"}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleToggle(d)}
                  loading={togglingId === d.id}
                  disabled={togglingId !== null || removingId !== null || busy}
                >
                  {d.is_enabled ? "禁用" : "启用"}
                </Button>
                <Button
                  variant="danger"
                  size="sm"
                  onClick={() => handleRemove(d)}
                  loading={removingId === d.id}
                  disabled={removingId !== null || togglingId !== null || busy}
                >
                  <Trash2 className="h-4 w-4" />
                  {removingId === d.id ? "解绑中" : "解绑"}
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

function AgentSkillList({ agentId }: { agentId: string }) {
  const [items, setItems] = useState<AgentSkillItem[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    invoke<AgentSkillItem[]>("scan_agent_skills", { agentId, force: false })
      .then((list) => {
        if (!cancelled) setItems(list);
      })
      .catch(() => {
        if (!cancelled) setItems([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [agentId]);

  if (loading) return <SkeletonList count={5} />;
  if (items.length === 0)
    return (
      <EmptyState
        icon={FolderOpen}
        title="该智能体目录下暂无技能"
        description="点击目录行的「重新扫描」可刷新技能列表。"
      />
    );

  return (
    <Card padding="none">
      <ul className="divide-y divide-[var(--divider)]">
        {items.map((item) => (
          <li key={item.name} className="flex items-center justify-between px-6 py-3 text-sm">
            <span className="font-medium text-primary">{item.name}</span>
            <Badge
              variant={
                item.exists_in_center
                  ? item.content_match
                    ? "success"
                    : "warning"
                  : "default"
              }
              size="sm"
            >
              {item.exists_in_center ? (item.content_match ? "已同步" : "有差异") : "未导入中心"}
            </Badge>
          </li>
        ))}
      </ul>
    </Card>
  );
}

function MetricCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary p-4">
      <div className="text-xs text-secondary">{label}</div>
      <div className="mt-1 text-2xl font-semibold text-accent">{value}</div>
    </div>
  );
}
