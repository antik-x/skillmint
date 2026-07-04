import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Bot, CheckCircle2, X, XCircle } from "lucide-react";
import { showError, showSuccess } from "../stores/toastStore";
import { useAppStore } from "../stores/appStore";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { SkeletonList } from "./ui/Skeleton";
import { agentDisplayName } from "../pages/Agents";
import type { Agent, AgentSkillItem, Skill } from "../types";

interface Props {
  agents: Agent[];
  onClose: () => void;
  onImported: () => void;
}

type SelectedItems = Set<string>;
type ConflictChoices = Record<string, "center" | "local">;

export default function ImportSkillModal({ agents, onClose, onImported }: Props) {
  // SPEC-F6 T4: 导入成功后用 navigateToSkill 把用户牵引到新导入项。
  const navigateToSkill = useAppStore((state) => state.navigateToSkill);
  const [selectedAgent, setSelectedAgent] = useState<string>("");
  const [items, setItems] = useState<AgentSkillItem[]>([]);
  const [selectedItems, setSelectedItems] = useState<SelectedItems>(new Set());
  const [conflictChoices, setConflictChoices] = useState<ConflictChoices>({});
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    if (selectedAgent) {
      setLoading(true);
      invoke<AgentSkillItem[]>("scan_agent_skills", { agentId: selectedAgent })
        .then(setItems)
        .catch((err) => {
          const message =
            typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
          showError(`扫描 Agent Skill 失败：${message}`);
          setItems([]);
        })
        .finally(() => setLoading(false));
    } else {
      setItems([]);
    }
  }, [selectedAgent]);

  const toggleItem = (name: string) => {
    setSelectedItems((prev) => {
      const next = new Set(prev);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  };

  const setConflictChoice = (name: string, choice: "center" | "local") => {
    setConflictChoices((prev) => ({ ...prev, [name]: choice }));
  };

  const handleImport = async () => {
    if (selectedItems.size === 0) return;
    if (importing) return;
    setImporting(true);
    let imported = 0;
    let failed = 0;
    let lastError = "";
    let lastImportedSkill: Skill | null = null;
    for (const name of selectedItems) {
      const item = items.find((i) => i.name === name);
      const resolution =
        item?.exists_in_center && item.content_match === false ? conflictChoices[name] : undefined;
      try {
        const skill = await invoke<Skill>("import_skill", {
          agentId: selectedAgent,
          skillName: name,
          resolution,
        });
        imported += 1;
        lastImportedSkill = skill;
      } catch (err) {
        failed += 1;
        lastError = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
        console.error(`[ImportSkillModal] import_skill failed for ${name}:`, lastError);
      }
    }
    if (imported > 0) {
      onImported();
    }
    if (failed > 0) {
      showError(lastError, { context: `导入 ${imported} 成功，${failed} 失败` });
    } else {
      // SPEC-F6 T4: 成功 toast 增加「查看导入的 Skill」动作，点击直达 Skill 库并高亮。
      const targetId = lastImportedSkill?.id ?? null;
      showSuccess(`成功导入 ${imported} 个 Skill`, {
        label: "查看导入的 Skill",
        onClick: () => {
          if (targetId) navigateToSkill(targetId);
        },
      });
      onClose();
    }
    setImporting(false);
  };

  const enabledAgents = agents.filter((a) => a.is_enabled);

  // SPEC-F6 T1: 导入弹窗按 skill_directory 去重。指向同一目录的多个 Agent
  // 扫描出的 Skill 内容完全相同——展示一个代表项即可，避免在 27 个等价选项里
  // 做无意义选择。代表项取每个目录组里的第一个 Agent；label 用消歧名以便用户辨识。
  const dedupedAgents = useMemo(() => {
    const seen = new Set<string>();
    const result: Agent[] = [];
    for (const a of enabledAgents) {
      const key = a.skill_directory;
      if (seen.has(key)) continue;
      seen.add(key);
      result.push(a);
    }
    return result;
  }, [enabledAgents]);

  const canImport =
    selectedItems.size > 0 &&
    !Array.from(selectedItems).some((name) => {
      const item = items.find((i) => i.name === name);
      return item?.exists_in_center && item.content_match === false && !conflictChoices[name];
    });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div className="surface-glass w-full max-w-xl rounded-2xl border border-[var(--border-subtle)] bg-secondary p-6 shadow-2xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-xl font-semibold text-primary">从 Agent 导入 Skill</h2>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-tertiary hover:bg-tertiary/60 hover:text-primary"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="mb-4">
          <label className="mb-2 block text-sm font-medium text-secondary">选择 Agent</label>
          <div className="relative">
            <select
              value={selectedAgent}
              onChange={(e) => setSelectedAgent(e.target.value)}
              disabled={importing}
              className="w-full appearance-none rounded-lg border border-[var(--border-prominent)] bg-primary px-4 py-2 pr-10 text-sm text-primary focus:border-accent focus:outline-none disabled:opacity-50"
            >
              <option value="">请选择…</option>
              {dedupedAgents.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agentDisplayName(agent, dedupedAgents)}
                </option>
              ))}
            </select>
            <Bot className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-tertiary" />
          </div>
        </div>

        <div className="mb-4 max-h-80 overflow-auto rounded-xl border border-[var(--border-subtle)]">
          {loading ? (
            <SkeletonList count={4} />
          ) : items.length === 0 ? (
            <div className="p-6 text-center text-sm text-secondary">
              {selectedAgent ? "该 Agent 目录下没有发现 Skill。" : "请先选择一个 Agent。"}
            </div>
          ) : (
            <ul className="divide-y divide-[var(--divider)]">
              {items.map((item) => {
                const isConflict = item.exists_in_center && item.content_match === false;
                const isSame = item.exists_in_center && item.content_match === true;
                return (
                  <li key={item.name} className="px-4 py-3">
                    <div className="flex items-center justify-between">
                      <label className="flex items-center gap-3">
                        <input
                          type="checkbox"
                          checked={selectedItems.has(item.name)}
                          onChange={() => toggleItem(item.name)}
                          disabled={importing}
                          className="h-4 w-4 accent-accent disabled:opacity-50"
                        />
                        <span className="font-medium text-primary">{item.name}</span>
                      </label>
                      <div className="text-xs">
                        {isSame ? (
                          <Badge variant="success" size="sm">
                            <CheckCircle2 className="mr-1 h-3 w-3" />
                            与中心一致
                          </Badge>
                        ) : isConflict ? (
                          <Badge variant="danger" size="sm">
                            <XCircle className="mr-1 h-3 w-3" />
                            内容冲突
                          </Badge>
                        ) : (
                          <Badge variant="accent" size="sm">
                            新 Skill
                          </Badge>
                        )}
                      </div>
                    </div>
                    {isConflict && selectedItems.has(item.name) && (
                      <div className="mt-2 flex gap-2 pl-7">
                        <button
                          onClick={() => setConflictChoice(item.name, "center")}
                          disabled={importing}
                          className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors disabled:opacity-50 ${
                            conflictChoices[item.name] === "center"
                              ? "bg-accent text-white"
                              : "bg-tertiary text-primary hover:bg-[var(--border-subtle)]"
                          }`}
                        >
                          保留中心
                        </button>
                        <button
                          onClick={() => setConflictChoice(item.name, "local")}
                          disabled={importing}
                          className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors disabled:opacity-50 ${
                            conflictChoices[item.name] === "local"
                              ? "bg-warning text-white"
                              : "bg-tertiary text-primary hover:bg-[var(--border-subtle)]"
                          }`}
                        >
                          保留本地
                        </button>
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div className="flex justify-end gap-3">
          <Button variant="secondary" size="sm" onClick={onClose} disabled={importing}>
            取消
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={handleImport}
            loading={importing}
            disabled={!canImport || importing}
          >
            {importing ? "导入中…" : `导入 ${selectedItems.size} 个`}
          </Button>
        </div>
      </div>
    </div>
  );
}
