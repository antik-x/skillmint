import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "../lib/invoke";
import {
  ArrowLeft,
  Copy,
  Download,
  Edit3,
  ExternalLink,
  FileText,
  FolderOpen,
  GitBranch,
  Plus,
  Puzzle,
  RefreshCw,
  Terminal,
  Trash2,
} from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { showError, showSuccess } from "../stores/toastStore";
import type { Skill as SkillType, SkillRecommendation, SkillStatus, SkillUsageSummary, SyncAllResult, SyncStatus } from "../types";
import ImportSkillModal from "../components/ImportSkillModal";
import SkillEditor from "../components/SkillEditor";
import VersionPanel from "../components/VersionPanel";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { Dialog, DialogActions } from "../components/ui/Dialog";
import { Input } from "../components/ui/Input";
import { EmptyState } from "../components/ui/EmptyState";
import { HelpTip } from "../components/ui/HelpTip";
import { VirtualList } from "../components/ui/VirtualList";
import { cn } from "../components/ui/utils";
import { useListNavigation } from "../hooks/useListNavigation";

export default function Skills() {
  const skills = useAppStore((state) => state.skills);
  const agents = useAppStore((state) => state.agents);
  const syncTargets = useAppStore((state) => state.syncTargets);
  const loadData = useAppStore((state) => state.loadData);
  const settings = useAppStore((state) => state.settings);
  const selectedSkillId = useAppStore((state) => state.selectedSkillId);
  const setSelectedSkillId = useAppStore((state) => state.setSelectedSkillId);
  const selectedSkillName = useAppStore((state) => state.selectedSkillName);
  const setSelectedSkillName = useAppStore((state) => state.setSelectedSkillName);
  const newSkillRequest = useAppStore((state) => state.newSkillRequest);

  const [showCreate, setShowCreate] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [newSkillName, setNewSkillName] = useState("");
  const [selectedAgents, setSelectedAgents] = useState<string[]>([]);
  const [creating, setCreating] = useState(false);
  const [detailId, setDetailId] = useState<string | null>(selectedSkillId);
  const setSkillEditMode = useAppStore((state) => state.setSkillEditMode);
  const listRef = useRef<HTMLDivElement>(null);

  // PRD-09: allow other pages to navigate to a specific skill detail.
  useEffect(() => {
    if (selectedSkillId) {
      setDetailId(selectedSkillId);
      setSelectedSkillId(null);
    }
    if (selectedSkillName) {
      const matched = skills.find((s) => s.name === selectedSkillName);
      if (matched) {
        setDetailId(matched.id);
      }
      setSelectedSkillName(null);
    }
  }, [selectedSkillId, setSelectedSkillId, selectedSkillName, setSelectedSkillName, skills]);

  // SPEC-F2 T5: keyboard navigation for the skill list.
  const { activeIndex, getItemProps, listProps } = useListNavigation<SkillType>({
    items: skills,
    onSelect: (skill) => setDetailId(skill.id),
    listRef,
  });

  // I3: react to global "new skill" shortcut / command palette action.
  useEffect(() => {
    if (newSkillRequest > 0) {
      setShowCreate(true);
    }
  }, [newSkillRequest]);

  const isProjectMode = settings.skill_scope_mode === "project";

  const handleCreate = useCallback(async () => {
    if (!newSkillName.trim()) return;
    if (creating) return;
    setCreating(true);
    try {
      await invoke("create_skill", { name: newSkillName.trim(), agentIds: selectedAgents });
      setNewSkillName("");
      setSelectedAgents([]);
      setShowCreate(false);
      await loadData();
      showSuccess("Skill 创建成功");
    } catch (err) {
      // SPEC-F6 T3: 重名走结构化错误，给出恢复路径。
      showError(err, { context: "创建 Skill" });
    } finally {
      setCreating(false);
    }
  }, [newSkillName, selectedAgents, creating, loadData]);

  const toggleAgent = useCallback((agentId: string) => {
    setSelectedAgents((prev) =>
      prev.includes(agentId) ? prev.filter((id) => id !== agentId) : [...prev, agentId]
    );
  }, []);

  const enabledAgents = useMemo(() => agents.filter((a) => a.is_enabled), [agents]);

  if (detailId) {
    const skill = skills.find((s) => s.id === detailId);
    if (skill) {
      return <SkillDetailView skill={skill} onBack={() => setDetailId(null)} />;
    }
  }

  return (
    <div className="flex h-full flex-col p-8">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-semibold tracking-tight text-primary">技能</h1>
          <p className="mt-1 text-sm text-secondary">管理中心仓库中的 Skill 与 Agent 同步关系</p>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" size="sm" onClick={() => setShowImport(true)}>
            <Download className="h-4 w-4" />
            从 Agent 导入
          </Button>
          <Button variant="primary" size="sm" onClick={() => setShowCreate(true)}>
            <Plus className="h-4 w-4" />
            新建 Skill
          </Button>
        </div>
      </div>

      {isProjectMode && (
        <div className="mb-4 rounded-lg border border-warning/20 bg-warning/10 px-4 py-2.5 text-sm text-warning">
          项目模式已开启：全局自动同步已暂停。新建 Skill 时可选「保存到项目」。
        </div>
      )}

      {showImport && (
        <ImportSkillModal agents={agents} onClose={() => setShowImport(false)} onImported={loadData} />
      )}

      <Dialog
        open={showCreate}
        onClose={() => setShowCreate(false)}
        onSubmit={handleCreate}
        title="新建 Skill"
      >
        <div className="mb-4">
          <label className="mb-2 block text-sm font-medium text-secondary">Skill 名称</label>
          <Input
            type="text"
            value={newSkillName}
            onChange={(e) => setNewSkillName(e.target.value)}
            placeholder="例如：weekly-report"
          />
        </div>
        <div className="mb-5">
          <label className="mb-2 block text-sm font-medium text-secondary">同步到 Agent</label>
          <p className="mb-2 text-xs text-tertiary">
            Agent 是你在用的 AI 编程工具（如 Cursor、Claude Code）。勾选后，这个 Skill 会被放进对应工具的
            skills 目录，工具即可直接使用。
          </p>
          <div className="flex flex-wrap gap-2">
            {enabledAgents.map((agent) => (
              <label
                key={agent.id}
                className={cn(
                  "cursor-pointer rounded-lg border px-3 py-1.5 text-sm transition-colors",
                  selectedAgents.includes(agent.id)
                    ? "border-accent bg-accent/10 text-accent"
                    : "border-[var(--border-subtle)] bg-primary text-secondary hover:border-[var(--border-prominent)] hover:text-primary"
                )}
              >
                <input
                  type="checkbox"
                  className="mr-2 accent-[var(--accent)]"
                  checked={selectedAgents.includes(agent.id)}
                  onChange={() => toggleAgent(agent.id)}
                />
                {agent.name}
              </label>
            ))}
          </div>
        </div>
        <DialogActions>
          <Button variant="primary" size="sm" onClick={handleCreate} loading={creating}>
            创建
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setShowCreate(false)}>
            取消
          </Button>
        </DialogActions>
      </Dialog>

      {skills.length === 0 ? (
        <EmptyState
          icon={Puzzle}
          illustration="library"
          title="还没有 Skill"
          description="点击右上角「新建 Skill」开始，或从已扫描的 Agent 导入。"
          action={
            <Button variant="primary" size="sm" onClick={() => setShowCreate(true)}>
              <Plus className="h-4 w-4" />
              新建 Skill
            </Button>
          }
        />
      ) : (
        <Card className="flex-1 overflow-hidden p-0" variant="default" padding="none">
          <div ref={listRef} className="h-full outline-none" {...listProps}>
            <VirtualList
              items={skills}
              estimateSize={72}
              getItemKey={(skill) => skill.id}
              className="h-full"
              activeIndex={activeIndex}
              renderItem={(skill, index) => (
                <SkillListItem
                  skill={skill}
                  index={index}
                  active={index === activeIndex}
                  itemProps={getItemProps(index)}
                  syncStatus={aggregateSyncStatus(syncTargets.filter((t) => t.skill_id === skill.id))}
                  onOpen={() => setDetailId(skill.id)}
                  onEdit={() => {
                    setDetailId(skill.id);
                    setSkillEditMode(true);
                  }}
                />
              )}
            />
          </div>
        </Card>
      )}
    </div>
  );
}

export const SkillListItem = memo(function SkillListItem({
  skill,
  index,
  active,
  itemProps,
  syncStatus,
  onOpen,
  onEdit,
}: {
  skill: SkillType;
  index: number;
  active: boolean;
  itemProps: ReturnType<ReturnType<typeof useListNavigation<SkillType>>["getItemProps"]>;
  syncStatus: { status: SyncStatus; label: string; hint?: string };
  onOpen: () => void;
  onEdit: () => void;
}) {
  return (
    <div
      {...itemProps}
      className={cn(
        "group flex cursor-pointer items-center justify-between border-b border-[var(--divider)] px-6 py-4 transition-colors last:border-b-0 hover:bg-tertiary/40",
        active && "bg-tertiary/60"
      )}
      onClick={() => {
        itemProps.onClick();
        onOpen();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onOpen();
        }
      }}
      tabIndex={-1}
      data-index={index}
    >
      <div className="flex items-center gap-3 overflow-hidden">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-accent/10 text-accent">
          <Puzzle className="h-5 w-5" />
        </div>
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <div className="truncate font-medium text-primary">{skill.name}</div>
            <StatusBadge status={skill.status} />
          </div>
          <div className="truncate text-sm text-secondary">{skill.repo_path}</div>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-3 pl-4">
        {/* SPEC-F5 T4: surface the per-skill sync status (dot + label) so users
            can spot stale/conflicting skills without opening each detail page. */}
        <span
          className="flex items-center gap-1.5 text-xs text-secondary"
          title={syncStatus.hint}
          data-sync-status={syncStatus.status}
        >
          <span
            className={`inline-block h-2 w-2 rounded-full ${syncDotClass(syncStatus.status)}`}
          />
          <span className="hidden sm:inline">{syncStatus.label}</span>
        </span>
        <div className="flex gap-2" onClick={(e) => e.stopPropagation()}>
          <Button
            variant="ghost"
            size="sm"
            onClick={onEdit}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                e.stopPropagation();
                onEdit();
              }
            }}
            tabIndex={0}
            aria-label={`编辑 ${skill.name}`}
          >
            <Edit3 className="h-4 w-4" />
            编辑
          </Button>
        </div>
      </div>
    </div>
  );
});

export function SkillDetailView({
  skill,
  onBack,
}: {
  skill: SkillType;
  onBack: () => void;
}) {
  const loadData = useAppStore((state) => state.loadData);
  const syncTargets = useAppStore((state) => state.syncTargets);
  const skillEditMode = useAppStore((state) => state.skillEditMode);
  const setSkillEditMode = useAppStore((state) => state.setSkillEditMode);
  const [activeTab, setDetailTab] = useState<"overview" | "edit" | "versions">("overview");
  // SPEC-C3 T3: trash confirmation + undo affordance.
  const [trashDialogOpen, setTrashDialogOpen] = useState(false);
  const [trashing, setTrashing] = useState(false);

  // SPEC-I5: open the editor tab when navigated from an accept decision.
  useEffect(() => {
    if (skillEditMode) {
      setDetailTab("edit");
      setSkillEditMode(false);
    }
  }, [skillEditMode, setDetailTab, setSkillEditMode]);
  const [usage, setUsage] = useState<SkillUsageSummary | null>(null);
  const [related, setRelated] = useState<SkillRecommendation[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    Promise.all([
      invoke<SkillUsageSummary[]>("get_skill_usage", { days: 30 }).catch(() => []),
      invoke<SkillRecommendation[]>("get_related_skills", { skillId: skill.id }).catch(() => []),
    ]).then(([u, r]) => {
      if (cancelled) return;
      setUsage(u.find((s) => s.skill_name === skill.name) ?? null);
      setRelated(r);
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [skill.id, skill.name]);

  const handleReject = useCallback((skillName: string) => {
    setRelated((prev) => prev.filter((r) => r.skill_name !== skillName));
    showSuccess(`已隐藏与「${skillName}」的关系`);
  }, []);

  const [syncing, setSyncing] = useState(false);
  const handleSync = useCallback(async () => {
    if (syncing) return;
    setSyncing(true);
    try {
      const result = await invoke<SyncAllResult>("sync_all_command");
      if (result.failure_count > 0) {
        const first = result.failures[0];
        const hint = first?.recovery_hint ? ` · ${first.recovery_hint}` : "";
        showError(
          `同步完成：${result.success_count} 成功，${result.failure_count} 失败（${first?.agent_name ?? ""}）${hint}`,
          8000
        );
      } else {
        showSuccess(`同步完成：${result.success_count} 个目标成功`);
      }
      await loadData();
    } catch (err) {
      // SPEC-F6 T3: 走 humanizeError 全量结构，保留恢复动作（写目录失败 → 检查权限）。
      showError(err, { context: "同步到 Agent" });
    } finally {
      setSyncing(false);
    }
  }, [syncing, loadData]);

  // SPEC-F6 T4: 保存成功后的 toast 用 ref 调用同一个 handleSync（复用详情页同步逻辑）。
  const handleSyncRef = useRef<(() => void) | null>(null);
  useEffect(() => {
    handleSyncRef.current = handleSync;
  }, [handleSync]);

  // SPEC-C3 T3: move the skill to the recycle bin (snapshot-first, reversible).
  const handleTrash = useCallback(async () => {
    if (trashing) return;
    setTrashing(true);
    try {
      const trashId = await invoke<number>("remove_skill", { skillId: skill.id });
      setTrashDialogOpen(false);
      showSuccess(
        "已移入回收站，30 天内可恢复",
        {
          label: "撤销",
          onClick: async () => {
            try {
              await invoke("restore_trash_item", { id: trashId, conflictStrategy: null });
              showSuccess("已恢复");
              await loadData();
            } catch (err) {
              const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
              showError(`撤销失败：${message}`);
            }
          },
        },
        5000,
      );
      onBack();
      await loadData();
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`移入回收站失败：${message}`);
    } finally {
      setTrashing(false);
    }
  }, [trashing, skill.id, onBack, loadData]);

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-[var(--divider)] px-8 py-4">
        <div className="flex items-center gap-4">
          <button
            onClick={onBack}
            className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-sm text-secondary transition-colors hover:bg-tertiary/60 hover:text-primary"
          >
            <ArrowLeft className="h-4 w-4" />
            返回列表
          </button>
          <div className="flex h-10 w-10 items-center justify-center rounded-2xl bg-accent/10 text-accent">
            <Puzzle className="h-5 w-5" />
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h1 className="text-xl font-semibold text-primary">{skill.name}</h1>
              <StatusBadge status={skill.status} />
              <SyncStatusBadge syncStatus={aggregateSyncStatus(syncTargets.filter((t) => t.skill_id === skill.id))} />
            </div>
            <div className="mt-1 flex flex-wrap items-center gap-2">
              <p className="text-xs text-secondary break-all">{skill.repo_path}</p>
              <PathActions repoPath={skill.repo_path} />
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <StatusSelector skillId={skill.id} status={skill.status} onChanged={loadData} />
          <Button variant="primary" size="sm" onClick={handleSync} loading={syncing} disabled={syncing}>
            <RefreshCw className="mr-1 h-4 w-4" />
            同步到 Agent
          </Button>
          {/* SPEC-F6 T2: 就地解释「同步到 Agent」。 */}
          <HelpTip
            ariaLabel="什么是同步到 Agent"
            text="把中心仓库里的这份 Skill 写入各 Agent 的规则目录（如 ~/.cursor/rules），Agent 下次运行时生效。"
          />
          {typeof window !== "undefined" && "__TAURI_INTERNALS__" in window ? (
            <Button variant="ghost" size="sm" onClick={() => invoke("open_skill_in_editor", { skillId: skill.id })}>
              <ExternalLink className="mr-1 h-4 w-4" />
              外部编辑器
            </Button>
          ) : (
            <button
              className="rounded p-1 text-tertiary/50"
              title="需在桌面 App 中使用"
              disabled
            >
              <ExternalLink className="h-4 w-4" />
            </button>
          )}
          <Button variant="ghost" size="sm" onClick={() => setTrashDialogOpen(true)}>
            <Trash2 className="mr-1 h-4 w-4" />
            移入回收站
          </Button>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 border-b border-[var(--divider)] px-8 py-2">
        <TabButton active={activeTab === "overview"} onClick={() => setDetailTab("overview")} icon={FileText} label="概览" />
        <TabButton active={activeTab === "edit"} onClick={() => setDetailTab("edit")} icon={Edit3} label="编辑" />
        <TabButton active={activeTab === "versions"} onClick={() => setDetailTab("versions")} icon={GitBranch} label="版本" />
      </div>

      {/* Content */}
      <div className="flex-1 overflow-auto p-8">
        {activeTab === "overview" && (
          <>
            <Card className="mb-6" padding="lg">
              <h2 className="mb-4 text-lg font-semibold text-primary">使用情况（近 30 天）</h2>
              {loading ? (
                <div className="space-y-3">
                  <div className="h-4 w-1/3 rounded bg-tertiary/60" />
                  <div className="grid grid-cols-3 gap-4">
                    <div className="h-20 rounded-xl bg-tertiary/60" />
                    <div className="h-20 rounded-xl bg-tertiary/60" />
                    <div className="h-20 rounded-xl bg-tertiary/60" />
                  </div>
                </div>
              ) : usage && (usage.usage_count > 0 || usage.session_count > 0) ? (
                <div className="grid grid-cols-3 gap-4">
                  <Metric label="使用次数" value={String(usage.usage_count)} />
                  <Metric label="覆盖会话" value={String(usage.session_count)} />
                  <Metric label="覆盖项目" value={String(usage.project_count)} />
                </div>
              ) : (
                <EmptyState
                  icon={Puzzle}
                  title="暂无使用数据"
                  description="该 Skill 可能尚未被 Agent 调用，或未采集使用数据。"
                />
              )}
            </Card>

            <Card padding="none">
              <div className="border-b border-[var(--divider)] px-6 py-4 text-lg font-semibold text-primary">
                相关 Skill
              </div>
              {related.length === 0 ? (
                <div className="p-6">
                  <EmptyState
                    icon={Puzzle}
                    title="暂无相关 Skill"
                    description="在「知识图谱」点「重新分析」可自动发现关系。"
                  />
                </div>
              ) : (
                <ul className="divide-y divide-[var(--divider)]">
                  {related.map((r) => (
                    <li
                      key={r.skill_id}
                      className="flex items-center justify-between px-6 py-3 text-sm"
                    >
                      <div className="min-w-0 flex-1 pr-4">
                        <div className="font-medium text-primary">{r.skill_name}</div>
                        {r.reason && <div className="mt-1 text-xs text-secondary">{r.reason}</div>}
                      </div>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleReject(r.skill_name)}
                        className="text-danger hover:bg-danger/10 hover:text-danger"
                      >
                        <Trash2 className="h-4 w-4" />
                        隐藏
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </Card>
          </>
        )}

        {activeTab === "edit" && (
          <SkillEditor
            skillId={skill.id}
            skillName={skill.name}
            onSaved={(updated) => {
              // SPEC-F6 T4: 保存成功 toast 追加「同步到 Agent」内联动作，
              // 复用本详情页同步逻辑，把「改完要同步」从记忆负担变成顺手一点。
              showSuccess(`已保存「${updated.name}」`, {
                label: "同步到 Agent",
                onClick: () => handleSyncRef.current?.(),
              });
              // If the skill was renamed, go back to list so the user sees the updated name.
              if (updated.name !== skill.name) {
                onBack();
              }
            }}
          />
        )}

        {activeTab === "versions" && (
          <div className="max-w-2xl">
            <VersionPanel skillId={skill.id} />
          </div>
        )}
      </div>

      {/* SPEC-C3 T3: trash confirmation (light confirm — no confirm code). */}
      <Dialog
        open={trashDialogOpen}
        onClose={() => setTrashDialogOpen(false)}
        title="移入回收站？"
        description={`Skill「${skill.name}」将从中心仓库与所有 Agent 目录移除。30 天内可在 设置 → 数据管理 → 回收站 恢复。`}
      >
        <DialogActions>
          <Button variant="secondary" size="sm" onClick={() => setTrashDialogOpen(false)}>
            取消
          </Button>
          <Button variant="danger" size="sm" onClick={handleTrash} loading={trashing}>
            <Trash2 className="mr-1 h-4 w-4" />
            移入回收站
          </Button>
        </DialogActions>
      </Dialog>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  icon: Icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm transition-colors ${
        active ? "bg-accent/20 font-medium text-accent" : "text-secondary hover:bg-tertiary/60 hover:text-primary"
      }`}
    >
      <Icon className="h-4 w-4" />
      {label}
    </button>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary p-4">
      <div className="text-xs font-medium text-secondary">{label}</div>
      <div className="mt-1 text-2xl font-semibold text-accent">{value}</div>
    </div>
  );
}

const STATUS_OPTIONS: { value: SkillStatus; label: string }[] = [
  { value: "draft", label: "草稿" },
  { value: "candidate", label: "候选" },
  { value: "approved", label: "已批准" },
  { value: "deprecated", label: "已弃用" },
];

function aggregateSyncStatus(targets: { status: SyncStatus }[]): { status: SyncStatus; label: string; hint?: string } {
  if (targets.length === 0) return { status: "synced", label: "已同步" };
  const statuses = targets.map((t) => t.status);
  if (statuses.some((s) => s === "broken")) {
    return { status: "broken", label: "目标失效", hint: "一个或多个同步目标已失效" };
  }
  if (statuses.some((s) => s === "conflict")) {
    return { status: "conflict", label: "部分失败", hint: "部分同步目标存在冲突" };
  }
  if (statuses.some((s) => s === "local_changed")) {
    return { status: "local_changed", label: "有本地变更", hint: "Agent 本地副本有修改" };
  }
  if (statuses.some((s) => s === "center_changed")) {
    return { status: "center_changed", label: "中心已更新", hint: "中心仓库有更新未同步到 Agent" };
  }
  return { status: "synced", label: "已同步" };
}

function SyncStatusBadge({
  syncStatus,
  compact,
}: {
  syncStatus: { status: SyncStatus; label: string; hint?: string };
  compact?: boolean;
}) {
  if (compact) {
    return (
      <span
        className={`inline-block h-2 w-2 rounded-full ${syncStatus.status === "synced" ? "bg-success" : syncStatus.status === "broken" ? "bg-danger" : syncStatus.status === "conflict" ? "bg-warning" : "bg-accent"}`}
        title={`${syncStatus.label}${syncStatus.hint ? ` · ${syncStatus.hint}` : ""}`}
      />
    );
  }
  return (
    <span
      className={`rounded-full px-2 py-0.5 text-2xs font-medium uppercase tracking-wide ${syncStatusColor(syncStatus.status)}`}
      title={syncStatus.hint}
    >
      {syncStatus.label}
    </span>
  );
}

/** SPEC-F5 T4: dot color for the list-row sync indicator. Keeps the same
 * status→color mapping as the detail page (syncStatusColor), expressed as a
 * solid background so a 2px dot stays legible at row scale. */
function syncDotClass(status: SyncStatus): string {
  switch (status) {
    case "synced":
      return "bg-success";
    case "local_changed":
    case "center_changed":
      return "bg-accent";
    case "conflict":
      return "bg-warning";
    case "broken":
      return "bg-danger";
    default:
      return "bg-tertiary";
  }
}

function syncStatusColor(status: SyncStatus) {
  switch (status) {
    case "synced":
      return "bg-success/10 text-success";
    case "local_changed":
      return "bg-accent/10 text-accent";
    case "center_changed":
      return "bg-info/10 text-info";
    case "conflict":
      return "bg-warning/10 text-warning";
    case "broken":
      return "bg-danger/10 text-danger";
    default:
      return "bg-tertiary/50 text-secondary";
  }
}

function statusColor(status: SkillStatus) {
  switch (status) {
    case "approved":
      return "bg-success/10 text-success";
    case "candidate":
      return "bg-accent/10 text-accent";
    case "deprecated":
      return "bg-danger/10 text-danger";
    default:
      return "bg-tertiary/50 text-secondary";
  }
}

function StatusBadge({ status }: { status: SkillStatus }) {
  const label = STATUS_OPTIONS.find((o) => o.value === status)?.label || status;
  return (
    <span className={`rounded-full px-2 py-0.5 text-2xs font-medium uppercase tracking-wide ${statusColor(status)}`}>
      {label}
    </span>
  );
}

function StatusSelector({
  skillId,
  status,
  onChanged,
}: {
  skillId: string;
  status: SkillStatus;
  onChanged: () => void;
}) {
  const [value, setValue] = useState<SkillStatus>(status);
  const [saving, setSaving] = useState(false);

  const handleChange = async (next: SkillStatus) => {
    if (next === value || saving) return;
    setSaving(true);
    try {
      await invoke("update_skill_status", { id: skillId, status: next });
      setValue(next);
      onChanged();
      showSuccess("状态已更新");
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`更新失败：${message}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <select
      value={value}
      onChange={(e) => handleChange(e.target.value as SkillStatus)}
      disabled={saving}
      className="rounded-lg border border-[var(--border-subtle)] bg-primary px-2 py-1 text-xs text-secondary focus:border-accent focus:outline-none disabled:opacity-50"
    >
      {STATUS_OPTIONS.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

/** SPEC-F2 T11: path collaboration actions for a Skill. */
function PathActions({ repoPath }: { repoPath: string }) {
  const [copied, setCopied] = useState(false);
  const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  const handleCopy = async () => {
    try {
      if (navigator.clipboard) {
        await navigator.clipboard.writeText(repoPath);
      } else {
        const ta = document.createElement("textarea");
        ta.value = repoPath;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        document.body.removeChild(ta);
      }
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
      showSuccess("路径已复制");
    } catch {
      showError("复制路径失败");
    }
  };

  const handleReveal = async () => {
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(repoPath);
    } catch {
      showError("无法在 Finder 中显示");
    }
  };

  const handleTerminal = async () => {
    try {
      await invoke("open_path_in_terminal", { path: repoPath });
    } catch {
      showError("无法在终端中打开");
    }
  };

  return (
    <span className="inline-flex items-center gap-1">
      <button
        onClick={handleCopy}
        className="rounded p-1 text-tertiary hover:bg-tertiary hover:text-primary"
        title={copied ? "已复制" : "复制路径"}
      >
        <Copy className="h-3 w-3" />
      </button>
      {isTauri ? (
        <>
          <button
            onClick={handleReveal}
            className="rounded p-1 text-tertiary hover:bg-tertiary hover:text-primary"
            title="在 Finder 中显示"
          >
            <FolderOpen className="h-3 w-3" />
          </button>
          <button
            onClick={handleTerminal}
            className="rounded p-1 text-tertiary hover:bg-tertiary hover:text-primary"
            title="在终端中打开"
          >
            <Terminal className="h-3 w-3" />
          </button>
        </>
      ) : (
        <>
          <button
            disabled
            className="rounded p-1 text-tertiary/50"
            title="需在桌面 App 中使用"
          >
            <FolderOpen className="h-3 w-3" />
          </button>
          <button
            disabled
            className="rounded p-1 text-tertiary/50"
            title="需在桌面 App 中使用"
          >
            <Terminal className="h-3 w-3" />
          </button>
        </>
      )}
    </span>
  );
}
