/**
 * P3-4: the skills library, rebuilt on the `npx skills` bridge (P3 design:
 * SkillMint is the GUI layer over the CLI; the on-disk locks + agent dirs +
 * private hubs are the only source of truth).
 *
 * Three sections driven by the rebuildable index:
 * - 已安装：npx-managed installs (global / project scope), with update/remove;
 * - 未管理：agent-dir skills no lock knows about (collect them into a hub);
 * - 私有 hub：authored skills (global ~/.skillmint/hub, project .skillmint/hub),
 *   installable anywhere via `npx skills add <hub>`.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Download, FolderPlus, Plus, RefreshCw, Search, Trash2, Upload } from "lucide-react";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { SkeletonList } from "../components/ui/Skeleton";
import { showError, showSuccess } from "../stores/toastStore";
import { cn } from "../components/ui/utils";
import {
  getAgentsTable,
  getProjects,
  hubInstallSource,
  getSkillIndex,
  hubCollectSkill,
  hubCreateSkill,
  hubListSkills,
  hubPush,
  hubSetRemote,
  hubStatus,
  npxInstall,
  npxRemove,
  npxUpdate,
  openPathInTerminal,
  previewInstallCommand,
  readSkillIndexContent,
  skillsSearch,
  rebuildSkillIndex,
  type AgentDef,
  type HubStatus,
  type InstallRequest,
  type ProjectRow,
  type SkillIndexEntry,
} from "../lib/npxskills";

type ScopeFilter = "all" | "installed" | "hub" | "unmanaged";

const SCOPE_LABEL: Record<string, string> = {
  global: "全局",
  project: "项目",
  "hub-global": "Hub · 全局",
  "hub-project": "Hub · 项目",
};

const MANAGED_LABEL: Record<string, string> = {
  npx: "npx 管理",
  unmanaged: "未管理",
  hub: "Hub 创作",
};

export default function InstalledSkills() {
  const [rows, setRows] = useState<SkillIndexEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<ScopeFilter>("all");
  const [query, setQuery] = useState("");
  const [agents, setAgents] = useState<AgentDef[]>([]);
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [projectRoot, setProjectRoot] = useState<string | null>(null);
  const [installOpen, setInstallOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [hub, setHub] = useState<HubStatus | null>(null);
  const [hubRemoteInput, setHubRemoteInput] = useState("");
  const [viewing, setViewing] = useState<{ name: string; content: string } | null>(null);

  const agentName = useCallback(
    (key: string) => agents.find((a) => a.key === key)?.display_name ?? key,
    [agents],
  );

  const refresh = useCallback(
    async (root: string | null) => {
      setLoading(true);
      try {
        await rebuildSkillIndex(root);
        const [installed, globalHubRows] = await Promise.all([
          getSkillIndex(root),
          hubListSkills("global", null).catch(() => []),
        ]);
        setRows([...installed, ...globalHubRows]);
      } catch (err) {
        showError(err, { context: "刷新技能索引" });
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  const refreshHub = useCallback(async () => {
    try {
      const st = await hubStatus("global", null);
      setHub(st);
      setHubRemoteInput(st.remote ?? "");
    } catch {
      setHub(null);
    }
  }, []);

  useEffect(() => {
    Promise.all([getAgentsTable(), getProjects().catch(() => [])])
      .then(([table, projectRows]) => {
        setAgents(table);
        setProjects(projectRows);
      })
      .catch((err) => showError(err, { context: "加载 agent 列表" }));
  }, []);

  useEffect(() => {
    void refresh(projectRoot);
    void refreshHub();
  }, [refresh, refreshHub, projectRoot]);

  // Auto-detect external `npx skills` activity: rescan when the window regains
  // focus if the on-disk fingerprint changed (P3 design: 轻量轮询 + 事件刷新).
  useEffect(() => {
    const onFocus = () => {
      void refresh(projectRoot);
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh, projectRoot]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return rows.filter((r) => {
      if (filter === "installed" && r.scope !== "global" && r.scope !== "project") return false;
      if (filter === "hub" && r.managed_by !== "hub") return false;
      if (filter === "unmanaged" && r.managed_by !== "unmanaged") return false;
      if (q && !r.name.toLowerCase().includes(q) && !(r.description ?? "").toLowerCase().includes(q)) {
        return false;
      }
      return true;
    });
  }, [rows, filter, query]);

  const counts = useMemo(
    () => ({
      installed: rows.filter((r) => r.scope === "global" || r.scope === "project").length,
      hub: rows.filter((r) => r.managed_by === "hub").length,
      unmanaged: rows.filter((r) => r.managed_by === "unmanaged").length,
    }),
    [rows],
  );

  return (
    <div className="flex h-full flex-col overflow-auto px-8 py-6">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold text-primary">技能库</h2>
          <p className="mt-0.5 text-xs text-secondary">
            通过 npx skills 安装 / 更新 / 卸载；私有技能由本地 Hub 承载
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="secondary" onClick={() => void refresh(projectRoot)} disabled={loading}>
            <RefreshCw className={cn("mr-1.5 h-4 w-4", loading && "animate-spin")} />
            刷新
          </Button>
          <Button variant="secondary" onClick={() => setCreateOpen(true)}>
            <Plus className="mr-1.5 h-4 w-4" />
            新建 Skill
          </Button>
          <Button variant="primary" onClick={() => setInstallOpen(true)}>
            <Download className="mr-1.5 h-4 w-4" />
            安装技能
          </Button>
        </div>
      </div>

      <div className="mb-3 flex flex-wrap items-center gap-2">
        <FilterChip active={filter === "all"} onClick={() => setFilter("all")} label={`全部 ${rows.length}`} />
        <FilterChip active={filter === "installed"} onClick={() => setFilter("installed")} label={`已安装 ${counts.installed}`} />
        <FilterChip active={filter === "hub"} onClick={() => setFilter("hub")} label={`Hub ${counts.hub}`} />
        <FilterChip active={filter === "unmanaged"} onClick={() => setFilter("unmanaged")} label={`未管理 ${counts.unmanaged}`} />
        <div className="ml-auto flex items-center gap-2">
          <select
            aria-label="项目上下文"
            value={projectRoot ?? ""}
            onChange={(e) => setProjectRoot(e.target.value || null)}
            className="rounded border border-[var(--border-subtle)] bg-transparent px-2 py-1.5 text-xs text-primary"
          >
            <option value="">仅全局</option>
            {projects.map((p) => (
              <option key={p.project_id} value={p.path}>
                {p.name || p.path}
              </option>
            ))}
          </select>
          <div className="relative">
            <Search className="pointer-events-none absolute left-2 top-2 h-3.5 w-3.5 text-secondary" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索技能…"
              className="w-48 rounded border border-[var(--border-subtle)] bg-transparent py-1.5 pl-7 pr-2 text-xs text-primary placeholder:text-secondary"
            />
          </div>
        </div>
      </div>

      {loading ? (
        <SkeletonList count={5} />
      ) : filtered.length === 0 ? (
        <EmptyHint onInstall={() => setInstallOpen(true)} />
      ) : (
        <ul className="space-y-2">
          {filtered.map((row) => (
            <SkillRow
              key={`${row.scope}/${row.name}/${row.path}`}
              row={row}
              agentName={agentName}
              projectRoot={projectRoot}
              onView={async () => {
                try {
                  const content = await readSkillIndexContent(row.skill_md_path);
                  setViewing({ name: row.name, content });
                } catch (err) {
                  showError(err, { context: "读取 SKILL.md" });
                }
              }}
              onUpdate={async () => {
                try {
                  const isGlobal = row.scope === "global";
                  await npxUpdate(isGlobal, row.name, isGlobal ? null : projectRoot);
                  showSuccess(`已更新 ${row.name}`);
                  void refresh(projectRoot);
                } catch (err) {
                  showError(err, { context: "更新技能" });
                }
              }}
              onRemove={async () => {
                const isGlobal = row.scope === "global";
                const ok = window.confirm(
                  `卸载 ${row.name}？\n\n将移除以下 agent 目录中的链接/副本：\n${
                    row.agents.map((a) => `- ${agentName(a)}`).join("\n") || "-（无 agent 链接）"
                  }\n\n删除前会快照到 ~/.skillmint/trash。`,
                );
                if (!ok) return;
                try {
                  await npxRemove(row.name, row.agents, isGlobal, isGlobal ? null : projectRoot);
                  showSuccess(`已卸载 ${row.name}`);
                  void refresh(projectRoot);
                } catch (err) {
                  showError(err, { context: "卸载技能" });
                }
              }}
              onCollect={async () => {
                const scope = row.scope === "project" ? "project" : "global";
                try {
                  await hubCollectSkill(scope, scope === "project" ? projectRoot : null, row.path);
                  showSuccess(`已收集 ${row.name} 到${scope === "global" ? "全局" : "项目"} Hub`);
                  void refresh(projectRoot);
                } catch (err) {
                  showError(err, { context: "收集到 Hub" });
                }
              }}
              onInstallFromHub={async () => {
                setInstallOpen(true);
              }}
            />
          ))}
        </ul>
      )}

      {/* Global hub management */}
      <div className="mt-6 rounded-lg border border-[var(--border-subtle)] p-4">
        <div className="mb-2 flex items-center justify-between">
          <div>
            <h3 className="text-sm font-semibold text-primary">全局私有 Hub（~/.skillmint/hub）</h3>
            <p className="mt-0.5 text-xs text-secondary">
              个人技能的创作/承载目录。其他机器可通过 <code>npx skills add &lt;远端 URL&gt;</code> 安装。
            </p>
          </div>
          {hub?.last_commit && (
            <span className="text-xs text-secondary">
              {hub.last_commit.hash} · {hub.last_commit.message}
            </span>
          )}
        </div>
        {hub?.exists ? (
          <div className="flex flex-wrap items-center gap-2 text-xs text-secondary">
            <Badge variant={hub.git_inited ? "default" : "warning"}>
              {hub.git_inited ? `git · ${hub.branch}` : "未 git 化"}
            </Badge>
            <span>{hub.skill_count} 个 skill</span>
            {hub.git_inited && <span>待推送 {hub.ahead}</span>}
            {hub.dirty_files > 0 && <span className="text-warning">未提交 {hub.dirty_files}</span>}
            <input
              value={hubRemoteInput}
              onChange={(e) => setHubRemoteInput(e.target.value)}
              placeholder="私有 git 远端（git@github.com:me/skills.git）"
              className="ml-1 w-72 rounded border border-[var(--border-subtle)] bg-transparent px-2 py-1 text-xs text-primary"
            />
            <Button
              variant="secondary"
              onClick={async () => {
                try {
                  await hubSetRemote("global", null, hubRemoteInput);
                  showSuccess("已保存 Hub 远端");
                  void refreshHub();
                } catch (err) {
                  showError(err, { context: "设置 Hub 远端" });
                }
              }}
            >
              保存远端
            </Button>
            <Button
              variant="secondary"
              onClick={async () => {
                try {
                  await hubPush("global", null);
                  showSuccess("已推送到远端");
                  void refreshHub();
                } catch (err) {
                  showError(err, { context: "推送 Hub" });
                }
              }}
            >
              <Upload className="mr-1.5 h-3.5 w-3.5" />
              推送
            </Button>
          </div>
        ) : (
          <p className="text-xs text-secondary">尚未创建——点「新建 Skill」会自动初始化。</p>
        )}
      </div>

      {installOpen && (
        <InstallModal
          agents={agents}
          projects={projects}
          projectRoot={projectRoot}
          onClose={() => setInstallOpen(false)}
          onDone={() => {
            setInstallOpen(false);
            void refresh(projectRoot);
            void refreshHub();
          }}
        />
      )}
      {createOpen && (
        <CreateSkillModal
          projects={projects}
          onClose={() => setCreateOpen(false)}
          onDone={() => {
            setCreateOpen(false);
            void refresh(projectRoot);
            void refreshHub();
          }}
        />
      )}
      {viewing && <ContentModal name={viewing.name} content={viewing.content} onClose={() => setViewing(null)} />}
    </div>
  );
}

// ---------------------------------------------------------------------------

function FilterChip({ active, onClick, label }: { active: boolean; onClick: () => void; label: string }) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "rounded-full border px-3 py-1 text-xs font-medium transition-colors",
        active
          ? "border-accent bg-accent/10 text-accent"
          : "border-[var(--border-subtle)] text-secondary hover:text-primary",
      )}
    >
      {label}
    </button>
  );
}

function SkillRow({
  row,
  agentName,
  projectRoot,
  onView,
  onUpdate,
  onRemove,
  onCollect,
}: {
  row: SkillIndexEntry;
  agentName: (key: string) => string;
  projectRoot: string | null;
  onView: () => void;
  onUpdate: () => void;
  onRemove: () => void;
  onCollect: () => void;
  onInstallFromHub: () => void;
}) {
  const canUpdateRemove = row.managed_by === "npx" && (row.scope === "global" || row.scope === "project");
  return (
    <li className="flex items-start justify-between gap-4 rounded-lg border border-[var(--border-subtle)] px-4 py-3">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-primary">{row.name}</span>
          <Badge variant="default">{SCOPE_LABEL[row.scope] ?? row.scope}</Badge>
          <Badge variant={row.managed_by === "npx" ? "default" : row.managed_by === "hub" ? "default" : "warning"}>
            {MANAGED_LABEL[row.managed_by] ?? row.managed_by}
          </Badge>
          {row.status === "modified" && <Badge variant="warning">已被本地修改</Badge>}
          {row.status === "broken" && <Badge variant="warning">失效链接</Badge>}
        </div>
        {row.description && <p className="mt-1 line-clamp-2 text-xs text-secondary">{row.description}</p>}
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          {row.agents.map((a) => (
            <span key={a} className="rounded bg-[var(--bg-subtle)] px-1.5 py-0.5 text-[10px] text-secondary">
              {agentName(a)}
            </span>
          ))}
          {row.source && (
            <span className="text-[10px] text-secondary">
              来源：{row.source}
              {row.ref_spec ? `@${row.ref_spec}` : ""}
            </span>
          )}
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
        <Button variant="ghost" onClick={onView}>
          查看
        </Button>
        {row.managed_by === "unmanaged" && (
          <Button variant="secondary" onClick={onCollect}>
            <FolderPlus className="mr-1 h-3.5 w-3.5" />
            收集到 Hub
          </Button>
        )}
        {row.managed_by === "npx" && (
          <Button variant="secondary" onClick={onCollect}>
            复制到 Hub
          </Button>
        )}
        {canUpdateRemove && (
          <Button variant="secondary" onClick={onUpdate}>
            <RefreshCw className="mr-1 h-3.5 w-3.5" />
            更新
          </Button>
        )}
        {canUpdateRemove && (
          <Button variant="danger" onClick={onRemove}>
            <Trash2 className="mr-1 h-3.5 w-3.5" />
            卸载
          </Button>
        )}
        {row.managed_by === "hub" && <InstallFromHubButton row={row} projectRoot={projectRoot} />}
      </div>
    </li>
  );
}

function InstallFromHubButton({ row, projectRoot }: { row: SkillIndexEntry; projectRoot: string | null }) {
  const [busy, setBusy] = useState(false);
  return (
    <Button
      variant="secondary"
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        try {
          const source = await hubInstallSource(row.scope === "hub-project" ? "project" : "global", projectRoot);
          await npxInstall({
            source,
            skills: [row.name],
            agents: [],
            global: false,
            copy: false,
            project_root: projectRoot,
          });
          showSuccess(`已从 Hub 安装 ${row.name}（项目级）`);
        } catch (err) {
          showError(err, { context: "从 Hub 安装" });
        } finally {
          setBusy(false);
        }
      }}
    >
      安装到项目
    </Button>
  );
}

function EmptyHint({ onInstall }: { onInstall: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center rounded-lg border border-dashed border-[var(--border-subtle)] py-16 text-center">
      <p className="text-sm text-secondary">还没有安装任何技能。</p>
      <p className="mt-1 text-xs text-secondary">
        可以从 skills.sh 安装，或把 agent 目录里的现有技能收集到 Hub。
      </p>
      <Button variant="primary" className="mt-4" onClick={onInstall}>
        <Download className="mr-1.5 h-4 w-4" />
        安装技能
      </Button>
    </div>
  );
}

function ContentModal({ name, content, onClose }: { name: string; content: string; onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-8" onClick={onClose}>
      <div
        className="flex max-h-[80vh] w-[720px] max-w-full flex-col rounded-lg bg-[var(--bg-primary)] p-4 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-2 flex items-center justify-between">
          <h3 className="text-sm font-semibold text-primary">{name} · SKILL.md</h3>
          <Button variant="ghost" onClick={onClose}>
            关闭
          </Button>
        </div>
        <pre className="flex-1 overflow-auto whitespace-pre-wrap rounded bg-[var(--bg-subtle)] p-3 text-xs text-primary">
          {content}
        </pre>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Install modal — Mole-style transparency: shows the exact CLI command before
// running and streams live output while it runs.
// ---------------------------------------------------------------------------

export function InstallModal({
  agents,
  projects,
  projectRoot,
  onClose,
  onDone,
}: {
  agents: AgentDef[];
  projects: ProjectRow[];
  projectRoot: string | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const [source, setSource] = useState("");
  const [skills, setSkills] = useState("");
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set());
  const [global, setGlobal] = useState(false);
  const [copy, setCopy] = useState(false);
  const [useProject, setUseProject] = useState<string>(projectRoot ?? projects[0]?.path ?? "");
  const [preview, setPreview] = useState("");
  const [running, setRunning] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [searchHits, setSearchHits] = useState<{ name: string; source: string }[] | null>(null);

  const agentOptions = useMemo(
    () => (global ? agents.filter((a) => a.global_dir) : agents),
    [agents, global],
  );

  const req: InstallRequest = useMemo(
    () => ({
      source: source.trim(),
      skills: skills.split(/[,\s]+/).filter(Boolean),
      agents: [...selectedAgents],
      global,
      copy,
      project_root: global ? null : useProject || null,
    }),
    [source, skills, selectedAgents, global, copy, useProject],
  );

  // Live command preview whenever any input changes (and the request is valid).
  useEffect(() => {
    if (!req.source || req.skills.length === 0 || running) {
      setPreview("");
      return;
    }
    let cancelled = false;
    previewInstallCommand(req)
      .then((cmd) => {
        if (!cancelled) setPreview(cmd);
      })
      .catch(() => setPreview(""));
    return () => {
      cancelled = true;
    };
  }, [req, running]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    void listen<string>("npx-output", (e) => {
      setLog((prev) => [...prev.slice(-400), e.payload]);
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  const searchSkills = async () => {
    try {
      const hits = await skillsSearch(source.trim() || skills.trim());
      setSearchHits(hits.map((h) => ({ name: h.name, source: h.source })));
    } catch (err) {
      showError(err, { context: "搜索 skills.sh" });
      setSearchHits(null);
    }
  };

  const run = async () => {
    setRunning(true);
    setLog([]);
    try {
      const result = await npxInstall(req);
      if (result.run.success) {
        const risky = result.safety.filter((s) => s.findings > 0);
        showSuccess(
          risky.length > 0
            ? `安装完成，但 ${risky.map((r) => r.name).join(", ")} 的 SKILL.md 含需注意的指令，请在详情中确认`
            : "安装完成",
        );
        onDone();
      } else {
        showError(`安装失败（退出码 ${result.run.exit_code ?? "?"}），详见输出日志`);
      }
    } catch (err) {
      showError(err, { context: "安装技能" });
    } finally {
      setRunning(false);
    }
  };

  const canRun = !running && req.source.length > 0 && req.skills.length > 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-8" onClick={running ? undefined : onClose}>
      <div
        className="flex max-h-[85vh] w-[680px] max-w-full flex-col gap-3 overflow-auto rounded-lg bg-[var(--bg-primary)] p-5 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-base font-semibold text-primary">安装技能（npx skills）</h3>

        <label className="text-xs font-medium text-secondary">
          来源（owner/repo、git URL、或本地 Hub 路径）
          <div className="mt-1 flex gap-2">
            <input
              value={source}
              onChange={(e) => setSource(e.target.value)}
              placeholder="例如 vercel-labs/agent-skills 或 ~/.skillmint/hub"
              className="flex-1 rounded border border-[var(--border-subtle)] bg-transparent px-2 py-1.5 text-xs text-primary"
            />
            <Button variant="secondary" onClick={searchSkills}>
              搜索
            </Button>
          </div>
        </label>
        {searchHits && (
          <div className="max-h-28 overflow-auto rounded border border-[var(--border-subtle)] p-2 text-xs">
            {searchHits.length === 0 && <span className="text-secondary">没有匹配结果</span>}
            {searchHits.map((h) => (
              <button
                key={`${h.source}/${h.name}`}
                className="block w-full rounded px-1.5 py-1 text-left hover:bg-[var(--bg-subtle)]"
                onClick={() => {
                  setSource(h.source);
                  setSkills(h.name);
                }}
              >
                <span className="font-medium text-primary">{h.name}</span>
                <span className="ml-2 text-secondary">{h.source}</span>
              </button>
            ))}
          </div>
        )}

        <label className="text-xs font-medium text-secondary">
          Skill 名称（逗号分隔；GUI 安装必须显式指定）
          <input
            value={skills}
            onChange={(e) => setSkills(e.target.value)}
            placeholder="例如 pdf, docx"
            className="mt-1 w-full rounded border border-[var(--border-subtle)] bg-transparent px-2 py-1.5 text-xs text-primary"
          />
        </label>

        <div className="flex items-center gap-4 text-xs text-secondary">
          <label className="flex items-center gap-1.5">
            <input type="checkbox" checked={global} onChange={(e) => setGlobal(e.target.checked)} />
            全局安装（-g）
          </label>
          {!global && (
            <select
              aria-label="目标项目"
              value={useProject}
              onChange={(e) => setUseProject(e.target.value)}
              className="rounded border border-[var(--border-subtle)] bg-transparent px-2 py-1 text-xs text-primary"
            >
              <option value="">选择项目…</option>
              {projects.map((p) => (
                <option key={p.project_id} value={p.path}>
                  {p.name || p.path}
                </option>
              ))}
            </select>
          )}
          <label className="flex items-center gap-1.5">
            <input type="checkbox" checked={copy} onChange={(e) => setCopy(e.target.checked)} />
            复制而非符号链接（--copy）
          </label>
        </div>

        <div>
          <div className="mb-1 flex items-center justify-between text-xs font-medium text-secondary">
            <span>目标 Agent（不选 = 本机已检测到的全部 agent）</span>
            <button
              className="text-accent"
              onClick={() =>
                setSelectedAgents((prev) =>
                  prev.size === agentOptions.filter((a) => a.global_dir || !global).length
                    ? new Set()
                    : new Set(agentOptions.filter((a) => a.global_dir || !global).map((a) => a.key)),
                )
              }
            >
              全选 / 清空
            </button>
          </div>
          <div className="grid max-h-32 grid-cols-3 gap-1 overflow-auto rounded border border-[var(--border-subtle)] p-2">
            {agentOptions.map((a) => (
              <label key={a.key} className="flex items-center gap-1.5 text-xs text-primary">
                <input
                  type="checkbox"
                  checked={selectedAgents.has(a.key)}
                  onChange={(e) =>
                    setSelectedAgents((prev) => {
                      const next = new Set(prev);
                      if (e.target.checked) next.add(a.key);
                      else next.delete(a.key);
                      return next;
                    })
                  }
                />
                {a.display_name}
              </label>
            ))}
          </div>
        </div>

        {preview && (
          <div className="rounded border border-[var(--border-subtle)] bg-[var(--bg-subtle)] p-2">
            <div className="mb-1 flex items-center justify-between">
              <span className="text-[10px] font-semibold uppercase tracking-wide text-secondary">等价命令</span>
              <div className="flex gap-2">
                <Button
                  variant="ghost"
                  onClick={() => {
                    void navigator.clipboard.writeText(preview);
                    showSuccess("已复制命令");
                  }}
                >
                  复制
                </Button>
                <Button variant="ghost" onClick={() => void openPathInTerminal(global ? "~" : useProject || "~")}>
                  在终端中打开
                </Button>
              </div>
            </div>
            <code className="break-all text-xs text-primary">{preview}</code>
          </div>
        )}

        {log.length > 0 && (
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded bg-[var(--bg-subtle)] p-2 text-[11px] text-secondary">
            {log.join("\n")}
          </pre>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose} disabled={running}>
            关闭
          </Button>
          <Button variant="primary" onClick={run} disabled={!canRun}>
            {running ? "安装中…" : "安装"}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Create-skill modal — writes into the chosen private hub (git auto-commit).
// ---------------------------------------------------------------------------

function CreateSkillModal({
  projects,
  onClose,
  onDone,
}: {
  projects: ProjectRow[];
  onClose: () => void;
  onDone: () => void;
}) {
  const [scope, setScope] = useState<"global" | "project">("global");
  const [projectPath, setProjectPath] = useState(projects[0]?.path ?? "");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-8" onClick={onClose}>
      <div
        className="flex w-[460px] max-w-full flex-col gap-3 rounded-lg bg-[var(--bg-primary)] p-5 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-base font-semibold text-primary">新建 Skill</h3>
        <div className="flex gap-4 text-xs text-secondary">
          <label className="flex items-center gap-1.5">
            <input type="radio" checked={scope === "global"} onChange={() => setScope("global")} />
            全局 Hub（~/.skillmint/hub）
          </label>
          <label className="flex items-center gap-1.5">
            <input
              type="radio"
              checked={scope === "project"}
              onChange={() => setScope("project")}
              disabled={projects.length === 0}
            />
            项目 Hub（.skillmint/hub）
          </label>
        </div>
        {scope === "project" && (
          <select
            aria-label="选择项目"
            value={projectPath}
            onChange={(e) => setProjectPath(e.target.value)}
            className="rounded border border-[var(--border-subtle)] bg-transparent px-2 py-1.5 text-xs text-primary"
          >
            <option value="">选择项目…</option>
            {projects.map((p) => (
              <option key={p.project_id} value={p.path}>
                {p.name || p.path}
              </option>
            ))}
          </select>
        )}
        <label className="text-xs font-medium text-secondary">
          名称（kebab-case）
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="例如 deploy-checklist"
            className="mt-1 w-full rounded border border-[var(--border-subtle)] bg-transparent px-2 py-1.5 text-xs text-primary"
          />
        </label>
        <label className="text-xs font-medium text-secondary">
          描述
          <input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="一句话说明这个技能做什么"
            className="mt-1 w-full rounded border border-[var(--border-subtle)] bg-transparent px-2 py-1.5 text-xs text-primary"
          />
        </label>
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            variant="primary"
            disabled={busy || !name.trim() || (scope === "project" && !projectPath)}
            onClick={async () => {
              setBusy(true);
              try {
                await hubCreateSkill(scope, scope === "project" ? projectPath : null, name.trim(), description.trim());
                showSuccess(`已在${scope === "global" ? "全局" : "项目"} Hub 创建 ${name.trim()}`);
                onDone();
              } catch (err) {
                showError(err, { context: "新建 Skill" });
              } finally {
                setBusy(false);
              }
            }}
          >
            创建
          </Button>
        </div>
      </div>
    </div>
  );
}
