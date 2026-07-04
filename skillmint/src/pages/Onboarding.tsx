import { useEffect, useState } from "react";
import { invoke } from "../lib/invoke";
import {
  CheckCircle2,
  ChevronRight,
  Database,
  FolderOpen,
  Loader2,
  RotateCcw,
  Shield,
  SkipForward,
  Sparkles,
  Terminal,
} from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { useCollectionStore } from "../stores/collectionStore";
import { showError, showSuccess } from "../stores/toastStore";
import { Button } from "../components/ui/Button";
import { agentDisplayName } from "./Agents";
import type { Agent, AppSettings, CollectedSource } from "../types";

export default function Onboarding() {
  const { settings, setSettings, loadData, skills, setActiveTab } = useAppStore();
  const collection = useCollectionStore();

  const [step, setStep] = useState(0);
  const [repoPath, setRepoPath] = useState(settings.center_repo || "~/.skillmint/repo");
  const [agents, setAgents] = useState<Agent[]>([]);
  const [selectedAgents, setSelectedAgents] = useState<string[]>([]);
  const [sources, setSources] = useState<CollectedSource[]>([]);
  const [scanning, setScanning] = useState(false);
  const [loadingSources, setLoadingSources] = useState(false);
  const [saving, setSaving] = useState(false);
  const [collectionStartedAt, setCollectionStartedAt] = useState<number | null>(null);
  const [collectionTimedOut, setCollectionTimedOut] = useState(false);
  const [collectionLongTimedOut, setCollectionLongTimedOut] = useState(false);
  const [cancelled, setCancelled] = useState(false);

  useEffect(() => {
    setRepoPath(settings.center_repo || "~/.skillmint/repo");
  }, [settings.center_repo]);

  // SPEC-F2 T4: if collection has been running for 10s with no progress, show downgrade UI.
  useEffect(() => {
    if (!collection.collecting || collectionStartedAt == null) {
      setCollectionTimedOut(false);
      return;
    }
    const okSources = collection.sources.filter((s) => s.status === "ok").length;
    if (okSources > 0) {
      setCollectionTimedOut(false);
      return;
    }
    const elapsed = Date.now() - collectionStartedAt;
    if (elapsed >= 10000) {
      setCollectionTimedOut(true);
      return;
    }
    const timer = setTimeout(() => {
      if (collection.sources.filter((s) => s.status === "ok").length === 0) {
        setCollectionTimedOut(true);
      }
    }, 10000 - elapsed);
    return () => clearTimeout(timer);
  }, [collection.collecting, collectionStartedAt, collection.sources]);

  // SPEC-F5 T1: after 60s of collecting with the job still unfinished, allow the
  // user to skip. Unlike the 10s short-timeout above, this gate is decoupled from
  // progress (okSources): "job still running past 60s" is the only condition, so
  // a partially-progressed-but-never-completing job (e.g. mock envs) still offers
  // the escape hatch.
  useEffect(() => {
    if (!collection.collecting || collectionStartedAt == null) {
      setCollectionLongTimedOut(false);
      return;
    }
    // Job already finished (success or error) — no need for the long-skip gate.
    if (collection.lastJobStats || collection.lastJobError) {
      setCollectionLongTimedOut(false);
      return;
    }
    const elapsed = Date.now() - collectionStartedAt;
    if (elapsed >= 60000) {
      setCollectionLongTimedOut(true);
      return;
    }
    const timer = setTimeout(() => {
      const state = useCollectionStore.getState();
      if (!state.lastJobStats && !state.lastJobError) {
        setCollectionLongTimedOut(true);
      }
    }, 60000 - elapsed);
    return () => clearTimeout(timer);
  }, [
    collection.collecting,
    collectionStartedAt,
    collection.lastJobStats,
    collection.lastJobError,
  ]);

  const handleScanAgents = async () => {
    setScanning(true);
    try {
      const scanned = await invoke<Agent[]>("scan_agents");
      setAgents(scanned);
      setSelectedAgents(scanned.filter((a) => a.is_enabled).map((a) => a.id));
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`扫描失败：${message}`);
    } finally {
      setScanning(false);
    }
  };

  const handlePreviewScope = async () => {
    setLoadingSources(true);
    try {
      const list = await invoke<CollectedSource[]>("get_collection_status");
      setSources(list);
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`读取采集范围失败：${message}`);
    } finally {
      setLoadingSources(false);
    }
  };

  const toggleAgent = (id: string) => {
    setSelectedAgents((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]
    );
  };

  const finish = async (navigateToSkills = false) => {
    if (saving) return;
    setSaving(true);
    try {
      const newSettings: AppSettings = {
        ...settings,
        center_repo: repoPath,
        onboarding_completed: true,
      };
      const saved = await invoke<AppSettings>("save_settings", { newSettings });
      setSettings(saved);

      let savedAgents = 0;
      let agentErrors = 0;
      for (const agent of agents) {
        const enabled = selectedAgents.includes(agent.id);
        if (enabled !== agent.is_enabled) {
          try {
            await invoke("save_agent", {
              agent: { ...agent, is_enabled: enabled },
            });
            savedAgents += 1;
          } catch (err) {
            agentErrors += 1;
            const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
            console.error(`[Onboarding] save_agent failed for ${agent.name}:`, message);
          }
        }
      }

      await loadData();
      if (agentErrors > 0) {
        showSuccess(`设置已保存（${savedAgents} 个 Agent 更新，${agentErrors} 个失败）`);
      } else {
        showSuccess("设置已保存");
      }
      if (navigateToSkills) {
        setActiveTab("skillLibrary");
      }
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`保存失败：${message}`);
    } finally {
      setSaving(false);
    }
  };

  const skip = () => finish(false);

  const startCollection = async () => {
    setCollectionStartedAt(Date.now());
    setCollectionTimedOut(false);
    const jobId = await collection.startCollect();
    if (!jobId) return;
    // also refresh source status while collecting
    collection.loadStatus(true);
  };

  const totalRecords = sources.reduce((sum, s) => sum + s.record_count, 0);

  const steps = [
    {
      title: "欢迎使用 SkillMint",
      description: "先把散落在你各个 AI Agent 里的 Skill 统一起来。",
      content: (
        <div className="space-y-6">
          <div className="mx-auto flex h-20 w-20 items-center justify-center rounded-2xl bg-accent text-4xl font-bold text-white shadow-sm shadow-accent/20">
            S
          </div>
          <div className="grid gap-4 sm:grid-cols-3">
            <BoundaryItem
              icon={<Shield className="h-5 w-5" />}
              title="只读本地文件"
              text="不会上传你的会话内容，所有分析都在本机完成。"
            />
            <BoundaryItem
              icon={<Database className="h-5 w-5" />}
              title="数据由你掌控"
              text="中心仓库、本地数据库和日志都在你的设备上。"
            />
            <BoundaryItem
              icon={<Sparkles className="h-5 w-5" />}
              title="随时可取消"
              text="采集、同步、分析都可以一键中断，不会锁死。"
            />
          </div>
        </div>
      ),
      action: () => {
        handleScanAgents().then(() => setStep(1));
      },
      actionLabel: "开始设置",
    },
    {
      title: "扫描本机 Agent",
      description: "选择要纳入同步和采集的 AI Agent。",
      content: (
        <div className="space-y-4">
          {agents.length === 0 ? (
            <div className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary p-6 text-center text-sm text-secondary">
              <Terminal className="mx-auto mb-2 h-6 w-6 text-tertiary" />
              未发现本地 Agent 目录。完成后你可以在「设置 → 数据采集」中手动添加。
            </div>
          ) : (
            <div className="space-y-2">
              {agents.map((agent) => (
                <label
                  key={agent.id}
                  className="flex items-center justify-between rounded-xl border border-[var(--border-subtle)] bg-secondary p-4"
                >
                  <div>
                    <div className="font-medium text-primary">
                      {agentDisplayName(agent, agents)}
                    </div>
                    <div className="text-xs text-secondary">{agent.skill_directory}</div>
                  </div>
                  <input
                    type="checkbox"
                    checked={selectedAgents.includes(agent.id)}
                    onChange={() => toggleAgent(agent.id)}
                    className="h-5 w-5 accent-accent"
                  />
                </label>
              ))}
            </div>
          )}
        </div>
      ),
      action: () => {
        handlePreviewScope().then(() => setStep(2));
      },
      actionLabel: "下一步：预览范围",
    },
    {
      title: "预览采集范围",
      description: "我们将从这些目录读取会话和 Skill 数据。",
      content: (
        <div className="space-y-4">
          {loadingSources ? (
            <div className="flex items-center justify-center py-10 text-sm text-secondary">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              估算中…
            </div>
          ) : sources.length === 0 ? (
            <div className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary p-6 text-center text-sm text-secondary">
              暂无数据源。先完成第一次采集后会自动识别。
            </div>
          ) : (
            <>
              <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary">
                {sources.map((s) => (
                  <div
                    key={s.source}
                    className="flex items-center justify-between border-b border-[var(--border-subtle)] px-4 py-3 last:border-b-0"
                  >
                    <div>
                      <div className="text-sm font-medium text-primary">{s.source}</div>
                      <div className="text-xs text-secondary">{s.data_path}</div>
                    </div>
                    <div className="text-right text-xs text-secondary">
                      <div>{s.record_count.toLocaleString()} 条记录</div>
                      <div className="text-tertiary">{statusText(s.status)}</div>
                    </div>
                  </div>
                ))}
              </div>
              <div className="text-right text-xs text-secondary">
                已识别 {sources.length} 个来源，共 {totalRecords.toLocaleString()} 条记录
              </div>
            </>
          )}
        </div>
      ),
      action: () => setStep(3),
      actionLabel: "下一步：开始采集",
    },
    {
      title: "执行第一次采集",
      description: "把历史会话和 Skill 信息读入本地数据库。",
      content: (
        <div className="space-y-4">
          {/* SPEC-F2 T4: downgrade when no agents were discovered. */}
          {agents.length === 0 && (
            <NoCollectableData onSkip={() => setStep(4)} />
          )}

          {agents.length > 0 && !collection.collecting && !collection.lastJobStats && !collection.lastJobError && !collectionTimedOut && (
            <div className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary p-6 text-center">
              <Button variant="primary" size="md" onClick={startCollection}>
                <RotateCcw className="mr-2 h-4 w-4" />
                开始第一次采集
              </Button>
              <p className="mt-3 text-xs text-secondary">
                可以随时取消，完成后会自动进入下一步。
              </p>
            </div>
          )}

          {agents.length > 0 && collection.collecting && (
            <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary p-6 text-center">
              <Loader2 className="mx-auto mb-2 h-6 w-6 animate-spin text-accent" />
              <div className="text-sm font-medium text-primary">采集中…</div>
              <div className="mt-1 text-xs text-secondary">
                已处理 {collection.sources.filter((s) => s.status === "ok").length} 个来源
              </div>
              {collection.currentJobId && (
                <div className="mt-3 flex items-center justify-center gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => {
                      if (collection.currentJobId) {
                        collection.cancelCollect(collection.currentJobId);
                        setCancelled(true);
                      }
                    }}
                  >
                    取消
                  </Button>
                  {collectionLongTimedOut && (
                    <Button variant="ghost" size="sm" onClick={() => setStep(4)}>
                      跳过
                    </Button>
                  )}
                </div>
              )}
            </div>
          )}

          {agents.length > 0 && collectionTimedOut && (
            <NoCollectableData
              onSkip={() => setStep(4)}
              onRetry={() => {
                setCollectionTimedOut(false);
                startCollection();
              }}
            />
          )}

          {collection.lastJobError && (
            <div className="rounded-xl border border-danger/20 bg-danger/10 p-4 text-sm text-danger">
              采集失败：{collection.lastJobError}
              <div className="mt-2">
                <Button variant="secondary" size="sm" onClick={startCollection}>
                  <RotateCcw className="mr-1 h-3 w-3" />
                  重试
                </Button>
              </div>
            </div>
          )}

          {collection.lastJobStats && !collection.collecting && (
            <div className="rounded-xl border border-success/20 bg-success/10 p-4 text-sm text-success">
              <div className="flex items-center gap-2 font-medium">
                <CheckCircle2 className="h-4 w-4" />
                采集完成
              </div>
              <div className="mt-1 text-secondary">
                共 {collection.lastJobStats.reduce((a, s) => a + s.sessions, 0)} 个会话，
                {collection.lastJobStats.reduce((a, s) => a + s.prompts, 0)} 条 Prompt
              </div>
            </div>
          )}
        </div>
      ),
      action: () => {
        loadData().then(() => setStep(4));
      },
      actionLabel: cancelled || collectionLongTimedOut ? "跳过并查看 Skill" : "下一步：查看 Skill",
      actionDisabled: !(collection.lastJobStats || collection.lastJobError || cancelled || collectionLongTimedOut),
    },
    {
      title: "你的第一批 Skill",
      description: "SkillMint 已从采集结果中识别出以下内容。",
      content: (
        <div className="space-y-4">
          {skills.length === 0 ? (
            <div className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary p-6 text-center text-sm text-secondary">
              暂无 Skill。多使用 Agent 产生几次会话后再来采集，就能生成 Skill 推荐。
            </div>
          ) : (
            <>
              <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary">
                {skills.slice(0, 6).map((skill) => (
                  <div
                    key={skill.id}
                    className="flex items-center justify-between border-b border-[var(--border-subtle)] px-4 py-3 last:border-b-0"
                  >
                    <div className="text-sm font-medium text-primary">{skill.name || skill.id}</div>
                    <div className="text-xs text-secondary">{skill.repo_path}</div>
                  </div>
                ))}
              </div>
              <div className="text-right text-xs text-secondary">
                共 {skills.length} 个 Skill
              </div>
            </>
          )}
        </div>
      ),
      action: () => finish(true),
      actionLabel: "进入 Skill 库",
      actionDisabled: saving,
    },
  ];

  const current = steps[step];
  const isLast = step === steps.length - 1;

  return (
    <div className="flex h-full items-center justify-center bg-primary p-6">
      <div className="w-full max-w-2xl rounded-2xl border border-[var(--border-subtle)] bg-secondary p-8 shadow-xl">
        <div className="mb-6 flex items-center justify-between">
          <div className="flex gap-1.5">
            {steps.map((_, idx) => (
              <div
                key={idx}
                className={`h-2 w-8 rounded-full ${
                  idx === step ? "bg-accent" : idx < step ? "bg-tertiary" : "bg-tertiary/50"
                }`}
              />
            ))}
          </div>
          <button
            onClick={skip}
            disabled={saving}
            className="flex items-center gap-1 text-xs text-secondary hover:text-primary disabled:opacity-50"
          >
            <SkipForward className="h-3.5 w-3.5" />
            跳过引导
          </button>
        </div>

        <div className="mb-2 text-center text-xs font-medium uppercase tracking-wide text-accent">
          步骤 {step + 1} / {steps.length}
        </div>
        <h1 className="mb-1 text-center text-2xl font-bold text-primary">{current.title}</h1>
        <p className="mb-8 text-center text-sm text-secondary">{current.description}</p>

        <div className="mb-10">{current.content}</div>

        <div className="flex justify-between">
          <div>
            {step > 0 && (
              <Button variant="secondary" size="sm" onClick={() => setStep(step - 1)}>
                上一步
              </Button>
            )}
          </div>
          <Button
            variant="primary"
            size="md"
            onClick={current.action}
            loading={scanning || loadingSources || saving || (step === 1 && scanning)}
            disabled={current.actionDisabled}
          >
            {isLast ? (
              <FolderOpen className="mr-1.5 h-4 w-4" />
            ) : (
              <ChevronRight className="mr-1.5 h-4 w-4" />
            )}
            {current.actionLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}

function BoundaryItem({ icon, title, text }: { icon: React.ReactNode; title: string; text: string }) {
  return (
    <div className="rounded-xl border border-[var(--border-subtle)] bg-primary p-4 text-center">
      <div className="mb-2 flex justify-center text-accent">{icon}</div>
      <div className="mb-1 text-sm font-medium text-primary">{title}</div>
      <div className="text-xs text-secondary leading-relaxed">{text}</div>
    </div>
  );
}

function statusText(status: string) {
  switch (status) {
    case "ok":
      return "可读取";
    case "not_found":
      return "路径不存在";
    case "error":
      return "读取失败";
    case "unsupported":
      return "不支持的格式";
    default:
      return status;
  }
}

function NoCollectableData({ onSkip, onRetry }: { onSkip: () => void; onRetry?: () => void }) {
  return (
    <div className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary p-6 text-center">
      <Terminal className="mx-auto mb-2 h-6 w-6 text-tertiary" />
      <div className="text-sm font-medium text-primary">未发现可采集的 Agent 数据</div>
      <p className="mt-1 text-xs text-secondary">
        你可以跳过此步，稍后在设置中添加数据源。
      </p>
      <div className="mt-3 flex items-center justify-center gap-2">
        <Button variant="secondary" size="sm" onClick={onSkip}>
          跳过
        </Button>
        {onRetry && (
          <Button variant="primary" size="sm" onClick={onRetry}>
            <RotateCcw className="mr-1 h-3 w-3" />
            重试
          </Button>
        )}
      </div>
    </div>
  );
}
