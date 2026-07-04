import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronDown,
  ChevronRight,
  FileText,
  FolderGit,
  Package,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import { showError, showSuccess } from "../stores/toastStore";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/ui/EmptyState";
import { Skeleton, SkeletonList } from "../components/ui/Skeleton";
import type {
  DiffStrategy,
  ProjectDetail,
  ProjectUsageSummary,
  ResolvedSkill,
  ResolveResult,
  Skill,
  SkillProjectBinding,
  SkillVersion,
} from "../types";

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

export default function Projects() {
  const [projects, setProjects] = useState<ProjectUsageSummary[]>([]);
  const [bindings, setBindings] = useState<SkillProjectBinding[]>([]);
  const [loading, setLoading] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [projs, binds] = await Promise.all([
        invoke<ProjectUsageSummary[]>("get_projects"),
        invoke<SkillProjectBinding[]>("get_skill_bindings"),
      ]);
      setProjects(projs);
      setBindings(binds);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`加载项目失败：${msg}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const handleScan = useCallback(async () => {
    if (scanning) return;
    setScanning(true);
    try {
      const n = await invoke<number>("scan_projects");
      showSuccess(`已关联 ${n} 个会话到项目`);
      await load();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`扫描失败：${msg}`);
    } finally {
      setScanning(false);
    }
  }, [scanning, load]);

  if (selectedId) {
    return <ProjectDetailView projectId={selectedId} onBack={() => setSelectedId(null)} />;
  }

  return (
    <div className="h-full overflow-auto p-8">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-semibold tracking-tight text-primary">项目</h1>
          <p className="mt-1 text-sm text-secondary">管理项目与 Skill 的关联关系</p>
        </div>
        <Button variant="primary" size="sm" onClick={handleScan} loading={scanning} disabled={scanning}>
          <RefreshCw className={`h-4 w-4 ${scanning ? "animate-spin" : ""}`} />
          {scanning ? "扫描中…" : "扫描项目"}
        </Button>
      </div>

      {projects.length === 0 ? (
        <EmptyState
          icon={FolderGit}
          illustration="generic"
          title="还没有项目"
          description="用 Claude Code 或 Codex 打开过一个代码仓库、并在「使用洞察」采集过数据后，点击「扫描项目」即可发现。"
          action={
            <button
              onClick={handleScan}
              disabled={loading}
              className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
            >
              {loading ? "扫描中…" : "扫描项目"}
            </button>
          }
        />
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {projects.map((p) => (
            <button
              type="button"
              key={p.project_id}
              className="cursor-pointer rounded-xl border border-[var(--border-subtle)] bg-secondary p-5 text-left transition-colors hover:border-accent"
              onClick={() => setSelectedId(p.project_id)}
            >
              {/* pointer-events-none on inner content so click target is always the button itself */}
              <div className="pointer-events-none">
                <div className="mb-2 flex items-center justify-between">
                  <div className="font-medium">{p.name}</div>
                  <div className="text-xs text-tertiary">{formatTokens(p.total_tokens)} tokens</div>
                </div>
                <div className="break-all text-xs text-tertiary">{p.path}</div>
                <div className="mt-3 flex items-center gap-4 text-sm text-primary">
                  <span>{p.session_count} 会话</span>
                  <span className="text-accent">查看详情 →</span>
                </div>
              </div>
            </button>
          ))}
        </div>
      )}

      {/* Skill-project bindings (legacy flat view, kept for global overview) */}
      {bindings.length > 0 && (
        <section className="mt-8 rounded-xl border border-[var(--border-subtle)] bg-secondary">
          <div className="border-b border-[var(--border-subtle)] px-6 py-4 text-lg font-semibold">项目级 Skill 绑定</div>
          <ul className="divide-y divide-[var(--divider)]">
            {bindings.map((b) => (
              <li key={b.id} className="flex items-center justify-between px-6 py-3 text-sm">
                <div>
                  <span className="font-medium">{b.skill_name ?? b.skill_id}</span>
                  <span className="ml-2 text-tertiary">→ {b.project_name ?? b.project_id ?? "全局"}</span>
                  {b.pinned_version && (
                    <span className="ml-2 rounded bg-blue-900/40 px-1.5 py-0.5 text-xs text-blue-300">
                      📌 {b.pinned_version}
                    </span>
                  )}
                </div>
                <div className="flex items-center gap-3 text-secondary">
                  <span>{b.mode}</span>
                  <span className={b.is_enabled ? "text-green-400" : "text-tertiary"}>
                    {b.is_enabled ? "启用" : "禁用"}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

// =============================================================================
// Project detail view (FR-A / FR-B / FR-C / FR-D / FR-F)
// =============================================================================

function ProjectDetailView({ projectId, onBack }: { projectId: string; onBack: () => void }) {
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [loading, setLoading] = useState(true);
  const [showInstall, setShowInstall] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  // expandAll > 0 means "force-expand all agents for this consistency check cycle".
  const [expandAll, setExpandAll] = useState(0);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const d = await invoke<ProjectDetail>("get_project_detail", { projectId });
      setDetail(d);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`加载项目详情失败：${msg}`);
    } finally {
      setLoading(false);
    }
  }, [projectId]);

  useEffect(() => {
    load();
  }, [load]);

  // FR-E §4.4a: manual "检测一致性" = expand all agents + bump refreshKey to force re-resolve.
  const handleCheckConsistency = () => {
    if (detail && detail.agents.length > 0) {
      const all: Record<string, boolean> = {};
      detail.agents.forEach((a) => (all[a.agent_id] = true));
      setExpanded(all);
    }
    setRefreshKey((k) => k + 1);
    setExpandAll((n) => n + 1);
    showSuccess("已重新检测一致性");
  };

  if (loading || !detail) {
    return (
      <div className="h-full overflow-auto p-8">
        <button onClick={onBack} className="mb-4 text-sm text-secondary hover:text-white">
          ← 返回项目列表
        </button>
        <div className="mb-6 rounded-xl border border-[var(--border-subtle)] bg-secondary p-6">
          <Skeleton className="mb-3 h-8 w-1/3" />
          <Skeleton className="mb-4 h-4 w-2/3" />
          <div className="flex flex-wrap gap-x-6 gap-y-1">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-4 w-24" />
          </div>
        </div>
        <SkeletonList count={5} />
      </div>
    );
  }

  return (
    <div className="h-full overflow-auto p-8">
      <button onClick={onBack} className="mb-4 text-sm text-secondary hover:text-white">
        ← 返回项目列表
      </button>

      {/* Header */}
      <div className="mb-6 rounded-xl border border-[var(--border-subtle)] bg-secondary p-6">
        <div className="flex items-center justify-between">
          <h1 className="text-2xl font-bold">{detail.name}</h1>
          <div className="flex gap-2">
            <button
              onClick={handleCheckConsistency}
              disabled={detail.agents.length === 0}
              className="rounded-lg border border-[var(--border-prominent)] px-3 py-1.5 text-sm text-primary hover:bg-tertiary disabled:cursor-not-allowed disabled:opacity-50"
            >
              检测一致性
            </button>
            <button
              onClick={() => setShowInstall(true)}
              className="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-primary hover:bg-accent-hover"
            >
              + 安装 Skill（软链接）
            </button>
            <button
              onClick={() => invoke("open_project_in_finder", { path: detail.path }).catch(() => {})}
              className="rounded-lg border border-[var(--border-prominent)] px-3 py-1.5 text-sm text-primary hover:bg-tertiary"
            >
              打开 Finder
            </button>
          </div>
        </div>
        <div className="mt-2 break-all text-sm text-tertiary">{detail.path}</div>
        <div className="mt-3 flex flex-wrap gap-x-6 gap-y-1 text-sm text-primary">
          <span>最近活跃：{timeAgo(detail.last_active_at)}</span>
          <span>{detail.session_count} 会话</span>
          <span>{formatTokens(detail.total_tokens)} tokens</span>
          <span>Agents：{detail.agents.length}</span>
        </div>
      </div>

      {/* Agents grouped */}
      {detail.agents.length === 0 ? (
        <section className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary p-8 text-center text-sm text-secondary">
          该项目尚未发现任何 Agent 活动。前往「使用洞察」点「立即采集」后，重新扫描项目即可看到关联 Agent。
        </section>
      ) : (
        <div className="space-y-3">
          {detail.agents.map((a) => (
            <AgentGroup
              key={`${a.agent_id}-${refreshKey}`}
              projectId={projectId}
              agent={a}
              forceExpanded={expandAll > 0}
              expanded={!!expanded[a.agent_id]}
              onToggle={() => setExpanded((e) => ({ ...e, [a.agent_id]: !e[a.agent_id] }))}
              refreshKey={refreshKey}
            />
          ))}
        </div>
      )}

      {showInstall && detail && (
        <InstallSkillDialog
          projectId={projectId}
          onClose={() => setShowInstall(false)}
          onDone={() => {
            setShowInstall(false);
            load();
          }}
        />
      )}
    </div>
  );
}

function AgentGroup({
  projectId,
  agent,
  forceExpanded,
  expanded,
  onToggle,
  refreshKey,
}: {
  projectId: string;
  agent: ProjectDetail["agents"][number];
  forceExpanded: boolean;
  expanded: boolean;
  onToggle: () => void;
  refreshKey: number;
}) {
  const isOpen = expanded || forceExpanded;
  return (
    <div className={`rounded-xl border border-[var(--border-subtle)] bg-secondary ${agent.is_enabled ? "" : "opacity-60"}`}>
      <button
        onClick={onToggle}
        className="flex w-full items-center justify-between px-6 py-4 text-left hover:bg-secondary/40"
      >
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className={`font-medium ${agent.is_enabled ? "" : "text-tertiary"}`}>{agent.agent_name}</span>
            {!agent.is_enabled && (
              <span className="rounded bg-tertiary px-1.5 py-0.5 text-xs text-secondary">已禁用</span>
            )}
            <span className="text-xs text-tertiary">{agent.skill_count ?? "-"} 个 Skill</span>
          </div>
          <div className="mt-1 truncate text-xs text-tertiary">{agent.skill_directory}</div>
        </div>
        <div className="flex shrink-0 items-center gap-4 text-xs text-secondary">
          <span>{agent.session_count} 会话</span>
          <span>{formatTokens(agent.total_tokens)} tokens</span>
          <span className="text-accent">{isOpen ? <ChevronDown className="inline h-4 w-4" /> : <ChevronRight className="inline h-4 w-4" />}</span>
        </div>
      </button>
      {isOpen && <AgentSkillTable projectId={projectId} agentId={agent.agent_id} refreshKey={refreshKey} />}
    </div>
  );
}

function AgentSkillTable({
  projectId,
  agentId,
  refreshKey,
}: {
  projectId: string;
  agentId: string;
  refreshKey: number;
}) {
  const [items, setItems] = useState<ResolvedSkill[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // Get the physical skill names first via scan_agent_skills, then resolve each.
      const raw = await invoke<{ name: string }[]>("scan_agent_skills", { agentId });
      const resolved = await Promise.all(
        raw.map((r) =>
          invoke<ResolvedSkill>("resolve_skill_link_command", {
            projectId,
            agentId,
            skillName: r.name,
          }).catch((err) => {
            const msg = typeof err === "string" ? err : String(err);
            console.error(`[Projects] resolve_skill_link_command failed for ${r.name}:`, msg);
            return null;
          }),
        ),
      );
      setItems(resolved.filter((x): x is ResolvedSkill => x !== null));
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`加载 Agent Skill 列表失败：${msg}`);
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, [projectId, agentId, refreshKey]);

  useEffect(() => {
    load();
  }, [load]);

  if (loading) return <SkeletonList count={4} />;
  if (items.length === 0)
    return <div className="border-t border-[var(--border-subtle)] px-6 py-4 text-sm text-secondary">该 Agent 目录下暂无 Skill。</div>;

  return (
    <div className="border-t border-[var(--border-subtle)]">
      <table className="w-full text-sm">
        <thead className="text-xs text-tertiary">
          <tr className="border-b border-[var(--border-subtle)]">
            <th className="px-6 py-2 text-left font-normal">Skill 名称</th>
            <th className="px-3 py-2 text-left font-normal">来源</th>
            <th className="px-3 py-2 text-left font-normal">指向原件 / 路径</th>
            <th className="px-3 py-2 text-left font-normal">状态</th>
            <th className="px-3 py-2 text-right font-normal">操作</th>
          </tr>
        </thead>
        <tbody>
          {items.map((it) => (
            <SkillRow key={it.name} projectId={projectId} agentId={agentId} skill={it} onResolved={load} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function sourceLabel(source: string): { icon: React.ReactNode; text: string } {
  switch (source) {
    case "symlink":
      return { icon: <Sparkles className="inline h-3.5 w-3.5" />, text: "软链接" };
    case "local_copy":
      return { icon: <FileText className="inline h-3.5 w-3.5" />, text: "复制" };
    case "local":
      return { icon: <Package className="inline h-3.5 w-3.5" />, text: "项目本地" };
    default:
      return { icon: "—", text: "失效" };
  }
}

function statusFor(skill: ResolvedSkill): { label: string; color: string } {
  if (skill.source === "broken") return { label: "失效", color: "text-red-400" };
  if (skill.pinned_version) return { label: `版本固定 📌 ${skill.pinned_version}`, color: "text-blue-300" };
  if (skill.source === "local") return { label: "仅本项目 —", color: "text-tertiary" };
  if (skill.content_match === true) return { label: "已同步", color: "text-green-400" };
  if (skill.content_match === false) return { label: "有差异 ⚠", color: "text-yellow-400" };
  return { label: "—", color: "text-tertiary" };
}

function SkillRow({
  projectId,
  agentId,
  skill,
  onResolved,
}: {
  projectId: string;
  agentId: string;
  skill: ResolvedSkill;
  onResolved: () => void;
}) {
  const [showDiff, setShowDiff] = useState(false);
  const src = sourceLabel(skill.source);
  const st = statusFor(skill);

  const hasDiff = skill.content_match === false && skill.source !== "broken";
  const isPinned = !!skill.pinned_version;

  return (
    <tr className="border-b border-[var(--border-subtle)]/60 last:border-0">
      <td className="px-6 py-3 font-medium">
        <div className="flex items-center gap-2">
          {skill.name}
          {!skill.is_registered && skill.source !== "broken" && (
            <span className="rounded bg-tertiary/60 px-1.5 py-0.5 text-2xs text-secondary">
              未登记（自动发现）
            </span>
          )}
        </div>
      </td>
      <td className="px-3 py-3 text-primary">
        {src.icon} {src.text}
      </td>
      <td className="px-3 py-3 text-xs text-tertiary">
        {skill.target_path ?? (skill.source === "local" ? "（项目本地）" : "（独立副本）")}
      </td>
      <td className={`px-3 py-3 ${st.color}`}>{st.label}</td>
      <td className="px-3 py-3 text-right">
        <div className="flex justify-end gap-1">
          {hasDiff && (
            <button
              onClick={() => setShowDiff(true)}
              className="rounded border border-[var(--border-prominent)] px-2 py-1 text-xs text-primary hover:bg-tertiary"
            >
              处理差异
            </button>
          )}
          {isPinned && (
            <FollowLatestButton
              projectId={projectId}
              agentId={agentId}
              skillName={skill.name}
              onDone={onResolved}
            />
          )}
        </div>
      </td>
      {showDiff && (
        <DiffDialog
          projectId={projectId}
          agentId={agentId}
          skill={skill}
          onClose={() => setShowDiff(false)}
          onDone={() => {
            setShowDiff(false);
            onResolved();
          }}
        />
      )}
    </tr>
  );
}

// FR-F §4.5f F-4: "跟随 latest" — unpin a versionized binding by re-running KeepCenter.
// Per DECISIONS 12, this reuses resolve_skill_diff_command(KeepCenter): clears pin +
// relinks the project copy to center latest, keeping DB/fs consistent.
function FollowLatestButton({
  projectId,
  agentId,
  skillName,
  onDone,
}: {
  projectId: string;
  agentId: string;
  skillName: string;
  onDone: () => void;
}) {
  const [busy, setBusy] = useState(false);

  const handleFollow = async () => {
    setBusy(true);
    try {
      // Look up the binding id for this project+agent+skill.
      const binds = await invoke<SkillProjectBinding[]>("get_skill_bindings");
      const b = binds.find(
        (x) =>
          x.project_id === projectId &&
          x.agent_id === agentId &&
          (x.skill_name === skillName || x.skill_id === skillName),
      );
      if (!b) {
        showError("未找到绑定记录");
        return;
      }
      await invoke("resolve_skill_diff_command", {
        bindingId: b.id,
        strategy: "keep_center",
        note: null,
      });
      showSuccess("已跟随 latest（解除版本固定）");
      onDone();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`跟随 latest 失败：${msg}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <button
      onClick={handleFollow}
      disabled={busy}
      className="rounded border border-[var(--border-prominent)] px-2 py-1 text-xs text-primary hover:bg-tertiary disabled:opacity-50"
    >
      {busy ? "…" : "跟随 latest"}
    </button>
  );
}

// FR-F: diff resolution dialog (merge / versionize) + version management
function DiffDialog({
  projectId,
  agentId,
  skill,
  onClose,
  onDone,
}: {
  projectId: string;
  agentId: string;
  skill: ResolvedSkill;
  onClose: () => void;
  onDone: () => void;
}) {
  const [strategy, setStrategy] = useState<DiffStrategy>("versionize");
  const [busy, setBusy] = useState(false);
  const [versions, setVersions] = useState<SkillVersion[]>([]);
  const [note, setNote] = useState(""); // PRD §4.5c version note
  const [backup, setBackup] = useState(false); // PRD §4.5c KeepProject 留底

  // Resolve binding id + skill id for this project+agent+skill.
  const [bindingId, setBindingId] = useState<string | null>(null);
  const [skillId, setSkillId] = useState<string | null>(null);

  const loadVersions = useCallback(async (sid: string) => {
    try {
      const vs = await invoke<SkillVersion[]>("list_skill_versions_command", { skillId: sid });
      setVersions(vs);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`加载版本列表失败：${msg}`);
      setVersions([]);
    }
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const binds = await invoke<SkillProjectBinding[]>("get_skill_bindings");
        const b = binds.find(
          (x) =>
            x.project_id === projectId &&
            x.agent_id === agentId &&
            (x.skill_name === skill.name || x.skill_id === skill.name),
        );
        setBindingId(b?.id ?? null);
        setSkillId(b?.skill_id ?? null);
        if (b?.skill_id) {
          await loadVersions(b.skill_id);
        }
      } catch (err) {
        const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`加载 Skill 绑定失败：${msg}`);
        setBindingId(null);
        setSkillId(null);
      }
    })();
  }, [projectId, agentId, skill.name, loadVersions]);

  const handleResolve = async () => {
    if (!bindingId) {
      showError("未找到该 Skill 的绑定记录，无法处理差异。请先在项目里登记绑定。");
      return;
    }
    setBusy(true);
    try {
      const effectiveStrategy: DiffStrategy =
        strategy === "keep_project" && backup ? "keep_project_backup" : strategy;
      const res = await invoke<ResolveResult>("resolve_skill_diff_command", {
        bindingId,
        strategy: effectiveStrategy,
        note: strategy === "versionize" && note.trim() ? note.trim() : null,
      });
      const msg =
        effectiveStrategy === "versionize"
          ? `已多版本化：创建 ${res.new_version} 并固定该项目到此版本`
          : effectiveStrategy === "keep_center"
            ? "已用中心 latest 覆盖项目副本"
            : effectiveStrategy === "keep_project_backup"
              ? `已反哺到 latest（旧版已留底为 ${res.new_version ?? "?"}）`
              : "已用项目副本更新中心 latest（反哺）";
      showSuccess(msg);
      onDone();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`处理失败：${msg}`);
    } finally {
      setBusy(false);
    }
  };

  const handlePin = async (version: string | null) => {
    if (!bindingId || !skillId) return;
    setBusy(true);
    try {
      await invoke("pin_binding_version", { bindingId, version });
      showSuccess(version ? `已固定到 ${version}` : "已跟随 latest");
      await loadVersions(skillId);
      onDone();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`固定失败：${msg}`);
    } finally {
      setBusy(false);
    }
  };

  const handleDeleteVersion = async (version: string) => {
    if (!skillId) return;
    if (!confirm(`确定删除版本 ${version}？此操作不可恢复。`)) return;
    setBusy(true);
    try {
      await invoke("delete_skill_version", { skillId, version });
      showSuccess(`已删除 ${version}`);
      await loadVersions(skillId);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`删除失败：${msg}`);
    } finally {
      setBusy(false);
    }
  };

  const handleSetNote = async (version: string, newNote: string) => {
    if (!skillId) return;
    try {
      await invoke("set_version_note", {
        skillId,
        version,
        note: newNote.trim() || null,
      });
      await loadVersions(skillId);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`保存备注失败：${msg}`);
    }
  };

  const isPinnedHere = (v: SkillVersion) =>
    skill.pinned_version === v.version && v.version !== "latest";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="w-full max-w-xl rounded-xl border border-[var(--border-subtle)] bg-secondary p-6">
        <h3 className="mb-1 text-lg font-semibold">处理 {skill.name} 的差异</h3>
        <p className="mb-4 text-sm text-secondary">项目副本与中心原件内容不同，请选择处理方式：</p>

        <div className="space-y-2">
          <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-[var(--border-subtle)] p-3 hover:bg-secondary/50">
            <input
              type="radio"
              name="strategy"
              checked={strategy === "keep_center"}
              onChange={() => setStrategy("keep_center")}
              className="mt-1"
            />
            <div>
              <div className="text-sm font-medium">合并 · 以中心为准</div>
              <div className="text-xs text-tertiary">用中心 latest 覆盖项目副本（项目变体将丢失）</div>
            </div>
          </label>
          <div className="rounded-lg border border-[var(--border-subtle)] p-3">
            <label className="flex cursor-pointer items-start gap-3 hover:bg-secondary/30">
              <input
                type="radio"
                name="strategy"
                checked={strategy === "keep_project"}
                onChange={() => setStrategy("keep_project")}
                className="mt-1"
              />
              <div className="flex-1">
                <div className="text-sm font-medium">合并 · 以项目为准</div>
                <div className="text-xs text-tertiary">用项目副本更新中心 latest（反哺，其他项目随之更新）</div>
                {strategy === "keep_project" && (
                  <label className="mt-2 flex cursor-pointer items-center gap-2 text-xs text-secondary">
                    <input
                      type="checkbox"
                      checked={backup}
                      onChange={(e) => setBackup(e.target.checked)}
                    />
                    反哺前先备份当前 latest（留底为 v&lt;x&gt;，PRD §4.5c）
                  </label>
                )}
              </div>
            </label>
          </div>
          <div className="rounded-lg border border-blue-700/50 bg-blue-900/10 p-3">
            <label className="flex cursor-pointer items-start gap-3 hover:bg-blue-900/20">
              <input
                type="radio"
                name="strategy"
                checked={strategy === "versionize"}
                onChange={() => setStrategy("versionize")}
                className="mt-1"
              />
              <div className="flex-1">
                <div className="text-sm font-medium">多版本化（推荐）</div>
                <div className="text-xs text-tertiary">
                  把当前中心内容快照为 v&lt;x&gt;（只读），项目固定到 v&lt;x&gt;；中心 latest 保持最新
                </div>
                {strategy === "versionize" && (
                  <input
                    type="text"
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                    placeholder="版本备注（可选，PRD §4.5c）"
                    className="mt-2 w-full rounded border border-[var(--border-prominent)] bg-primary px-2 py-1 text-xs text-primary placeholder:text-tertiary"
                  />
                )}
              </div>
            </label>
          </div>
        </div>

        {/* Version management panel */}
        {versions.length > 0 && (
          <div className="mt-4 rounded-lg border border-[var(--border-subtle)] p-3">
            <div className="mb-2 text-xs font-medium text-secondary">版本管理</div>
            <div className="space-y-2">
              {versions.map((v) => (
                <VersionRow
                  key={v.version}
                  version={v}
                  isPinnedHere={isPinnedHere(v)}
                  onPin={() => handlePin(isPinnedHere(v) ? null : v.version)}
                  onDelete={() => handleDeleteVersion(v.version)}
                  onSetNote={(note) => handleSetNote(v.version, note)}
                  busy={busy}
                />
              ))}
            </div>
          </div>
        )}

        <div className="mt-6 flex justify-end gap-2">
          <button onClick={onClose} className="rounded-lg px-4 py-2 text-sm text-secondary hover:text-white">
            取消
          </button>
          <button
            onClick={handleResolve}
            disabled={busy || !bindingId}
            className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-primary hover:bg-accent-hover disabled:opacity-50"
          >
            {busy ? "执行中…" : "执行"}
          </button>
        </div>
      </div>
    </div>
  );
}

function VersionRow({
  version,
  isPinnedHere,
  onPin,
  onDelete,
  onSetNote,
  busy,
}: {
  version: SkillVersion;
  isPinnedHere: boolean;
  onPin: () => void;
  onDelete: () => void;
  onSetNote: (note: string) => void;
  busy: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(version.note ?? "");

  const isLatest = version.version === "latest";
  const isPinnedAnywhere = version.pinned_by.length > 0;

  return (
    <div className="flex items-start justify-between gap-2 text-xs">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span
            className={`rounded px-1.5 py-0.5 ${
              isLatest
                ? "bg-green-900/40 text-green-300"
                : isPinnedAnywhere
                  ? "bg-blue-900/40 text-blue-300"
                  : "bg-tertiary/60 text-secondary"
            }`}
          >
            {version.version}
            {isPinnedAnywhere && " 📌"}
          </span>
          <span className="text-tertiary">{timeAgo(version.created_at)}</span>
          {version.pinned_by.length > 0 && (
            <span className="text-blue-300">固定于：{version.pinned_by.join("、")}</span>
          )}
        </div>
        {editing ? (
          <div className="mt-1 flex items-center gap-2">
            <input
              type="text"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="版本备注"
              className="flex-1 rounded border border-[var(--border-prominent)] bg-primary px-2 py-1 text-xs text-primary placeholder:text-tertiary"
            />
            <button
              onClick={() => {
                onSetNote(draft);
                setEditing(false);
              }}
              className="text-green-300 hover:text-green-200"
            >
              保存
            </button>
            <button
              onClick={() => {
                setDraft(version.note ?? "");
                setEditing(false);
              }}
              className="text-tertiary hover:text-secondary"
            >
              取消
            </button>
          </div>
        ) : (
          <div className="mt-1 flex items-center gap-2 text-tertiary">
            {version.note ? <span>✎ {version.note}</span> : <span>无备注</span>}
            {!isLatest && (
              <button
                onClick={() => setEditing(true)}
                className="text-accent hover:text-accent-hover"
              >
                编辑
              </button>
            )}
          </div>
        )}
      </div>
      <div className="flex shrink-0 gap-1">
        {!isLatest && (
          <button
            onClick={onPin}
            disabled={busy}
            className="rounded border border-[var(--border-prominent)] px-2 py-1 text-primary hover:bg-tertiary disabled:opacity-50"
          >
            {isPinnedHere ? "解固" : "固定"}
          </button>
        )}
        {!isLatest && !isPinnedAnywhere && (
          <button
            onClick={onDelete}
            disabled={busy}
            className="rounded border border-red-700/50 px-2 py-1 text-red-300 hover:bg-red-900/20 disabled:opacity-50"
          >
            删除
          </button>
        )}
      </div>
    </div>
  );
}

// FR-D: install skill dialog
function InstallSkillDialog({
  projectId,
  onClose,
  onDone,
}: {
  projectId: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [selectedSkill, setSelectedSkill] = useState<string>("");
  const [agents, setAgents] = useState<{ id: string; name: string }[]>([]);
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set());
  const [mode, setMode] = useState<"symlink" | "copy">("symlink");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    (async () => {
      try {
        const [allSkills, detail] = await Promise.all([
          invoke<Skill[]>("get_skills"),
          invoke<ProjectDetail>("get_project_detail", { projectId }),
        ]);
        setSkills(allSkills);
        if (allSkills.length > 0) setSelectedSkill(allSkills[0].id);
        setAgents(detail.agents.map((a) => ({ id: a.agent_id, name: a.agent_name })));
      } catch (err) {
        const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        showError(`加载安装对话框失败：${msg}`);
      }
    })();
  }, [projectId]);

  const toggleAgent = (id: string) => {
    setSelectedAgents((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const handleInstall = async () => {
    if (!selectedSkill || selectedAgents.size === 0) {
      showError("请选择源 Skill 和至少一个目标 Agent");
      return;
    }
    setBusy(true);
    try {
      await invoke("install_skill_to_project", {
        skillId: selectedSkill,
        projectId,
        agentIds: Array.from(selectedAgents),
        mode,
      });
      showSuccess(`已安装到 ${selectedAgents.size} 个 Agent`);
      onDone();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`安装失败：${msg}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="w-full max-w-lg rounded-xl border border-[var(--border-subtle)] bg-secondary p-6">
        <h3 className="mb-4 text-lg font-semibold">安装 Skill 到项目（软链接）</h3>

        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-xs text-secondary">源 Skill（中心仓库）</label>
            <select
              value={selectedSkill}
              onChange={(e) => setSelectedSkill(e.target.value)}
              className="w-full rounded-lg border border-[var(--border-subtle)] bg-primary px-3 py-2 text-sm"
            >
              {skills.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="mb-1 block text-xs text-secondary">安装到 Agent</label>
            <div className="space-y-1">
              {agents.map((a) => (
                <label key={a.id} className="flex cursor-pointer items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={selectedAgents.has(a.id)}
                    onChange={() => toggleAgent(a.id)}
                  />
                  {a.name}
                </label>
              ))}
              {agents.length === 0 && <div className="text-xs text-tertiary">该项目暂无 Agent</div>}
            </div>
          </div>

          <div>
            <label className="mb-1 block text-xs text-secondary">安装方式</label>
            <div className="flex gap-4 text-sm">
              <label className="flex cursor-pointer items-center gap-2">
                <input type="radio" checked={mode === "symlink"} onChange={() => setMode("symlink")} />
                软链接（推荐）
              </label>
              <label className="flex cursor-pointer items-center gap-2">
                <input type="radio" checked={mode === "copy"} onChange={() => setMode("copy")} />
                复制
              </label>
            </div>
          </div>
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button onClick={onClose} className="rounded-lg px-4 py-2 text-sm text-secondary hover:text-white">
            取消
          </button>
          <button
            onClick={handleInstall}
            disabled={busy}
            className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-primary hover:bg-accent-hover disabled:opacity-50"
          >
            {busy ? "创建中…" : mode === "symlink" ? "创建软链接" : "创建复制"}
          </button>
        </div>
      </div>
    </div>
  );
}
