import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Download,
  FolderOutput,
  Layers,
  Package,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { showError, showSuccess } from "../stores/toastStore";
import type {
  Agent,
  ApplyBundleResult,
  ProjectUsageSummary,
  Skill,
  SkillBundle,
  SkillBundleItem,
} from "../types";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";
import { EmptyState } from "./ui/EmptyState";
import { Input, TextArea } from "./ui/Input";

interface BundleManagerProps {
  showHeader?: boolean;
  showCreate?: boolean;
  onShowCreateChange?: (show: boolean) => void;
}

export default function BundleManager({
  showHeader = true,
  showCreate: controlledShowCreate,
  onShowCreateChange,
}: BundleManagerProps) {
  const skills = useAppStore((state) => state.skills);
  const agents = useAppStore((state) => state.agents);

  const [bundles, setBundles] = useState<SkillBundle[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [items, setItems] = useState<SkillBundleItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [internalShowCreate, setInternalShowCreate] = useState(false);
  const [newName, setNewName] = useState("");
  const [newDesc, setNewDesc] = useState("");
  const [creating, setCreating] = useState(false);

  const isControlled = controlledShowCreate !== undefined && onShowCreateChange !== undefined;
  const showCreate = isControlled ? controlledShowCreate : internalShowCreate;
  const setShowCreate = (value: boolean) => {
    if (isControlled) {
      onShowCreateChange(value);
    } else {
      setInternalShowCreate(value);
    }
  };

  const loadBundles = useCallback(async () => {
    setLoading(true);
    try {
      const list = await invoke<SkillBundle[]>("list_bundles");
      setBundles(list);
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`加载技能集失败：${msg}`);
    } finally {
      setLoading(false);
    }
  }, []);

  const loadItems = useCallback(async (bundleId: string) => {
    try {
      const [, its] = await invoke<[SkillBundle, SkillBundleItem[]]>(
        "get_bundle_detail",
        { id: bundleId }
      );
      setItems(its);
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`加载技能集详情失败：${msg}`);
    }
  }, []);

  useEffect(() => {
    loadBundles();
  }, [loadBundles]);

  useEffect(() => {
    if (selectedId) {
      loadItems(selectedId);
    } else {
      setItems([]);
    }
  }, [selectedId, loadItems]);

  const handleCreate = async () => {
    const name = newName.trim();
    if (!name) return;
    setCreating(true);
    try {
      await invoke("create_bundle", {
        name,
        description: newDesc.trim() || undefined,
      });
      setNewName("");
      setNewDesc("");
      setShowCreate(false);
      await loadBundles();
      showSuccess("技能集创建成功");
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`创建失败：${msg}`);
    } finally {
      setCreating(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm("确定删除该技能集？成员 Skill 不会被删除。")) return;
    try {
      await invoke("delete_bundle", { id });
      await loadBundles();
      if (selectedId === id) setSelectedId(null);
      showSuccess("已删除技能集");
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`删除失败：${msg}`);
    }
  };

  const selectedBundle = useMemo(
    () => bundles.find((b) => b.id === selectedId) || null,
    [bundles, selectedId]
  );

  if (selectedBundle) {
    return (
      <BundleDetail
        bundle={selectedBundle}
        items={items}
        skills={skills}
        agents={agents}
        onBack={() => setSelectedId(null)}
        onRefresh={() => {
          loadBundles();
          loadItems(selectedBundle.id);
        }}
        onDelete={() => handleDelete(selectedBundle.id)}
      />
    );
  }

  return (
    <div className="flex h-full flex-col">
      {showHeader && (
        <div className="mb-4 flex items-center justify-between">
          <p className="text-sm text-secondary">
            把常用 Skill 打包成技能集，一键应用到项目或导出分享。
          </p>
          <Button variant="primary" size="sm" onClick={() => setShowCreate(true)}>
            <Plus className="h-4 w-4" />
            新建技能集
          </Button>
        </div>
      )}

      {showCreate && (
        <Card className="mb-4" padding="md">
          <div className="mb-3 flex items-center justify-between">
            <h3 className="text-sm font-semibold text-primary">新建技能集</h3>
            <button
              onClick={() => setShowCreate(false)}
              className="rounded-md p-1 text-tertiary hover:bg-tertiary/60 hover:text-primary"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="mb-3">
            <label className="mb-1 block text-xs font-medium text-secondary">名称</label>
            <Input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="例如：后端 API 开发套装"
            />
          </div>
          <div className="mb-4">
            <label className="mb-1 block text-xs font-medium text-secondary">描述</label>
            <TextArea
              value={newDesc}
              onChange={(e) => setNewDesc(e.target.value)}
              placeholder="简单说明这个技能集的用途"
              rows={2}
            />
          </div>
          <div className="flex gap-2">
            <Button variant="primary" size="sm" onClick={handleCreate} loading={creating}>
              创建
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setShowCreate(false)}>
              取消
            </Button>
          </div>
        </Card>
      )}

      {loading ? (
        <div className="space-y-3">
          <div className="h-20 rounded-xl bg-tertiary/60" />
          <div className="h-20 rounded-xl bg-tertiary/60" />
        </div>
      ) : bundles.length === 0 ? (
        <EmptyState
          icon={Layers}
          title="还没有技能集"
          description="点击右上角「新建技能集」，把多个 Skill 打包成可复用的组合。"
          action={
            <Button variant="primary" size="sm" onClick={() => setShowCreate(true)}>
              <Plus className="h-4 w-4" />
              新建技能集
            </Button>
          }
        />
      ) : (
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
          {bundles.map((bundle) => (
            <Card
              key={bundle.id}
              className="cursor-pointer"
              padding="md"
              onClick={() => setSelectedId(bundle.id)}
            >
              <div className="mb-2 flex items-start justify-between">
                <div className="flex items-center gap-2">
                  <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-accent/10 text-accent">
                    <Layers className="h-4 w-4" />
                  </div>
                  <span className="font-medium text-primary">{bundle.name}</span>
                </div>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    handleDelete(bundle.id);
                  }}
                  className="rounded p-1 text-tertiary hover:bg-danger/10 hover:text-danger"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </div>
              {bundle.description && (
                <p className="mb-3 line-clamp-2 text-xs text-secondary">{bundle.description}</p>
              )}
              <div className="text-xs text-tertiary">
                <Package className="mr-1 inline h-3 w-3" />
                {bundle.skill_count} 个 Skill
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}

function BundleDetail({
  bundle,
  items,
  skills,
  agents,
  onBack,
  onRefresh,
  onDelete,
}: {
  bundle: SkillBundle;
  items: SkillBundleItem[];
  skills: Skill[];
  agents: Agent[];
  onBack: () => void;
  onRefresh: () => void;
  onDelete: () => void;
}) {
  const [showApply, setShowApply] = useState(false);
  const [selectedSkillId, setSelectedSkillId] = useState("");
  const [adding, setAdding] = useState(false);

  const availableSkills = useMemo(
    () => skills.filter((s) => !items.some((i) => i.skill_id === s.id)),
    [skills, items]
  );

  const handleAddSkill = async () => {
    if (!selectedSkillId) return;
    setAdding(true);
    try {
      await invoke("add_skill_to_bundle", {
        bundleId: bundle.id,
        skillId: selectedSkillId,
      });
      setSelectedSkillId("");
      onRefresh();
      showSuccess("已添加 Skill");
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`添加失败：${msg}`);
    } finally {
      setAdding(false);
    }
  };

  const handleRemoveSkill = async (skillId: string) => {
    try {
      await invoke("remove_skill_from_bundle", {
        bundleId: bundle.id,
        skillId,
      });
      onRefresh();
      showSuccess("已移除 Skill");
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`移除失败：${msg}`);
    }
  };

  const handleExport = async () => {
    try {
      const json = await invoke<string>("export_bundle", { id: bundle.id });
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${bundle.name}.bundle.json`;
      a.click();
      URL.revokeObjectURL(url);
      showSuccess("已导出技能集");
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`导出失败：${msg}`);
    }
  };

  const handleExportDirectory = async () => {
    try {
      const dir = await open({ directory: true, title: "选择导出目录" });
      if (!dir) return;
      const path = await invoke<string>("export_bundle_directory", { id: bundle.id, outputDir: dir });
      showSuccess(`已导出到 ${path}`);
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`导出失败：${msg}`);
    }
  };

  return (
    <div className="flex h-full flex-col">
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <button
            onClick={onBack}
            className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-sm text-secondary transition-colors hover:bg-tertiary/60 hover:text-primary"
          >
            <ArrowLeft className="h-4 w-4" />
            返回
          </button>
          <div>
            <h2 className="text-lg font-semibold text-primary">{bundle.name}</h2>
            {bundle.description && (
              <p className="text-xs text-secondary">{bundle.description}</p>
            )}
          </div>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" size="sm" onClick={handleExport}>
            <Download className="h-4 w-4" />
            导出 JSON
          </Button>
          <Button variant="secondary" size="sm" onClick={handleExportDirectory}>
            <FolderOutput className="h-4 w-4" />
            导出目录
          </Button>
          <Button variant="primary" size="sm" onClick={() => setShowApply(true)}>
            应用到项目
          </Button>
          <Button variant="danger" size="sm" onClick={onDelete}>
            <Trash2 className="h-4 w-4" />
            删除
          </Button>
        </div>
      </div>

      <Card className="mb-4" padding="md">
        <div className="mb-3 flex items-center justify-between">
          <h3 className="text-sm font-semibold text-primary">成员 Skill</h3>
          <span className="text-xs text-tertiary">{items.length} 个</span>
        </div>

        {availableSkills.length > 0 && (
          <div className="mb-4 flex gap-2">
            <select
              value={selectedSkillId}
              onChange={(e) => setSelectedSkillId(e.target.value)}
              className="flex-1 rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-2 text-sm text-primary outline-none"
            >
              <option value="">选择要添加的 Skill...</option>
              {availableSkills.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
            <Button variant="primary" size="sm" onClick={handleAddSkill} loading={adding}>
              <Plus className="h-4 w-4" />
              添加
            </Button>
          </div>
        )}

        {items.length === 0 ? (
          <div className="py-6 text-center text-sm text-tertiary">
            还没有成员 Skill。从上方下拉框选择一个添加。
          </div>
        ) : (
          <ul className="divide-y divide-[var(--divider)]">
            {items.map((item) => (
              <li
                key={item.id}
                className="flex items-center justify-between py-2.5 text-sm"
              >
                <div className="flex items-center gap-2">
                  <Package className="h-4 w-4 text-accent" />
                  <span className="text-primary">{item.skill_name}</span>
                </div>
                <button
                  onClick={() => handleRemoveSkill(item.skill_id)}
                  className="rounded p-1 text-tertiary hover:bg-danger/10 hover:text-danger"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {showApply && (
        <ApplyDialog
          bundle={bundle}
          agents={agents}
          onClose={() => setShowApply(false)}
          onApplied={() => {
            setShowApply(false);
            onRefresh();
          }}
        />
      )}
    </div>
  );
}

function ApplyDialog({
  bundle,
  agents,
  onClose,
  onApplied,
}: {
  bundle: SkillBundle;
  agents: Agent[];
  onClose: () => void;
  onApplied: () => void;
}) {
  const [projects, setProjects] = useState<ProjectUsageSummary[]>([]);
  const [selectedProject, setSelectedProject] = useState("");
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set());
  const [mode, setMode] = useState<"symlink" | "copy">("symlink");
  const [applying, setApplying] = useState(false);

  useEffect(() => {
    invoke<ProjectUsageSummary[]>("get_projects")
      .then(setProjects)
      .catch(() => setProjects([]));
    setSelectedAgents(new Set(agents.filter((a) => a.is_enabled).map((a) => a.id)));
  }, [agents]);

  const toggleAgent = (id: string) => {
    setSelectedAgents((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const handleApply = async () => {
    if (!selectedProject) {
      showError("请选择目标项目");
      return;
    }
    if (selectedAgents.size === 0) {
      showError("请至少选择一个 Agent");
      return;
    }
    setApplying(true);
    try {
      const res = await invoke<ApplyBundleResult>("apply_bundle_to_project", {
        bundleId: bundle.id,
        projectId: selectedProject,
        agentIds: Array.from(selectedAgents),
        mode,
      });
      const skipped = res.skipped ?? [];
      const msg =
        `已应用 ${res.applied.length} 个 Skill` +
        (skipped.length > 0 ? `，跳过 ${skipped.length} 个` : "");
      showSuccess(msg);
      if (skipped.length > 0) {
        showError(
          "跳过原因：\n" + skipped.map(([name, reason]) => `• ${name}：${reason}`).join("\n")
        );
      }
      onApplied();
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`应用失败：${msg}`);
    } finally {
      setApplying(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <Card
        className="max-h-[85vh] w-[560px] overflow-auto"
        padding="lg"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-lg font-bold text-primary">应用技能集到项目</h3>
          <button onClick={onClose} className="text-secondary hover:text-white">
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="mb-4">
          <label className="mb-1 block text-xs font-medium text-secondary">目标项目</label>
          <select
            value={selectedProject}
            onChange={(e) => setSelectedProject(e.target.value)}
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-2 text-sm text-primary outline-none"
          >
            <option value="">选择项目...</option>
            {projects.map((p) => (
              <option key={p.project_id} value={p.project_id}>
                {p.name}
              </option>
            ))}
          </select>
        </div>

        <div className="mb-4">
          <label className="mb-1 block text-xs font-medium text-secondary">同步目标 Agent</label>
          <div className="space-y-1">
            {agents.map((a) => (
              <label
                key={a.id}
                className="flex items-center gap-2 rounded px-2 py-1 hover:bg-tertiary/40"
              >
                <input
                  type="checkbox"
                  checked={selectedAgents.has(a.id)}
                  onChange={() => toggleAgent(a.id)}
                />
                <span className="text-sm text-primary">{a.name}</span>
                <span className="text-xs text-tertiary">{a.skill_directory}</span>
              </label>
            ))}
          </div>
        </div>

        <div className="mb-6">
          <label className="mb-1 block text-xs font-medium text-secondary">同步模式</label>
          <div className="flex gap-4 text-sm">
            <label className="flex items-center gap-1 text-primary">
              <input
                type="radio"
                checked={mode === "symlink"}
                onChange={() => setMode("symlink")}
              />
              软链接（推荐）
            </label>
            <label className="flex items-center gap-1 text-primary">
              <input
                type="radio"
                checked={mode === "copy"}
                onChange={() => setMode("copy")}
              />
              复制
            </label>
          </div>
        </div>

        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" size="sm" onClick={handleApply} loading={applying}>
            {applying ? "应用中..." : "确认应用"}
          </Button>
        </div>
      </Card>
    </div>
  );
}
