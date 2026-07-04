import { useEffect, useState, useCallback } from "react";
import { invoke } from "../lib/invoke";
import {
  BarChart3,
  CheckCircle2,
  Folder,
  GitBranch,
  Globe,
  LinkIcon,
  Lock,
  Package,
  Search,
} from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { showError, showSuccess } from "../stores/toastStore";
import { ErrorState } from "../components/ui/ErrorState";
import { SkeletonList } from "../components/ui/Skeleton";
import type {
  Source,
  SourceType,
  SkillRemoteMeta,
  SafetyScanResult,
  SearchResult,
  Agent,
} from "../types";

type Tab = "search" | "sources" | "updates";

export default function Discover() {
  const { settings } = useAppStore();
  const [tab, setTab] = useState<Tab>("search");
  const [sources, setSources] = useState<Source[]>([]);

  const refreshSources = useCallback(async () => {
    try {
      const list = await invoke<Source[]>("get_sources");
      setSources(list);
    } catch (err) {
      showError(err, { context: "加载远程源" });
    }
  }, []);

  useEffect(() => {
    refreshSources();
  }, [refreshSources]);

  // Remote not enabled: show a gated empty state with guidance.
  if (!settings.remote_enabled) {
    return (
      <div className="flex h-full items-center justify-center p-8">
        <div className="max-w-md text-center">
          <div className="mb-3"><Lock className="h-12 w-12 text-tertiary" /></div>
          <h2 className="mb-2 text-xl font-bold">远程功能未开启</h2>
          <p className="mb-4 text-sm text-text-secondary">
            SkillMint 默认完全离线运行，不发起任何网络请求。开启「远程功能」后，可连接
            GitHub 等 Git 仓库，一键安装 Skill 到中心仓库并同步到各 Agent。
          </p>
          <p className="text-xs text-tertiary">
            前往「偏好设置 → 远程功能」开启。开启后所有联网操作仍会显式提示。
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col p-6">
      <div className="mb-4 flex items-center justify-between">
        <h1 className="text-2xl font-bold">发现</h1>
        <div className="flex gap-1 rounded-lg bg-secondary p-1">
          <TabButton active={tab === "search"} onClick={() => setTab("search")}>
            <Search className="inline h-4 w-4 mr-1.5" />搜索
          </TabButton>
          <TabButton active={tab === "sources"} onClick={() => setTab("sources")}>
            <Globe className="inline h-4 w-4 mr-1.5" />远程源 ({sources.length})
          </TabButton>
          <TabButton active={tab === "updates"} onClick={() => setTab("updates")}>
            ⟳ 同步源
          </TabButton>
        </div>
      </div>

      <div className="flex-1 overflow-auto">
        {tab === "search" && <SearchTab sources={sources} />}
        {tab === "sources" && <SourcesTab sources={sources} onChange={refreshSources} />}
        {tab === "updates" && <UpdatesTab sources={sources} onChange={refreshSources} />}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded-md px-4 py-1.5 text-sm transition-colors ${
        active ? "bg-accent font-medium text-primary" : "text-primary hover:text-white"
      }`}
    >
      {children}
    </button>
  );
}

// =============================================================================
// Tab 1: Unified search
// =============================================================================

function SearchTab({ sources }: { sources: Source[] }) {
  const discoverSearchTerm = useAppStore((state) => state.discoverSearchTerm);
  const setDiscoverSearchTerm = useAppStore((state) => state.setDiscoverSearchTerm);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [previewing, setPreviewing] = useState<SearchResult | null>(null);
  const [previewBody, setPreviewBody] = useState("");
  const [installTarget, setInstallTarget] = useState<SearchResult | null>(null);

  // SPEC-I5: carry a search term from the inbox capability-gap card.
  useEffect(() => {
    if (discoverSearchTerm) {
      setQuery(discoverSearchTerm);
      setDiscoverSearchTerm(null);
    }
  }, [discoverSearchTerm, setDiscoverSearchTerm]);

  const runSearch = useCallback(async () => {
    setLoading(true);
    try {
      const r = await invoke<SearchResult[]>("search_all", { query });
      setResults(r);
    } catch (err) {
      showError(err, { context: "搜索" });
    } finally {
      setLoading(false);
    }
  }, [query]);

  // PRD-07 §3.3c: debounced auto-search (200ms after the user stops typing),
  // plus an immediate search on mount.
  useEffect(() => {
    const handle = setTimeout(() => {
      runSearch();
    }, 200);
    return () => clearTimeout(handle);
  }, [runSearch]);

  const onSearch = (e: React.FormEvent) => {
    e.preventDefault();
    runSearch();
  };

  const doPreview = async (r: SearchResult) => {
    if (r.origin !== "remote" || !r.source_id) return;
    setPreviewing(r);
    setPreviewBody("加载中...");
    try {
      const body = await invoke<string>("preview_remote_skill", {
        sourceId: r.source_id,
        skillPath: r.skill_path,
      });
      setPreviewBody(body);
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      setPreviewBody(`预览失败：${msg}`);
    }
  };

  const local = results.filter((r) => r.origin === "local");
  const remote = results.filter((r) => r.origin === "remote");

  return (
    <div>
      <form onSubmit={onSearch} className="mb-4 flex gap-2">
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索 Skill 名称、描述、标签或正文..."
          className="flex-1 rounded-lg border border-[var(--border-subtle)] bg-secondary px-4 py-2 text-sm outline-none placeholder:text-tertiary focus:border-accent"
        />
        <button
          type="submit"
          disabled={loading}
          className="rounded-lg bg-accent px-5 py-2 text-sm font-medium text-primary disabled:opacity-50"
        >
          {loading ? "搜索中..." : "搜索"}
        </button>
      </form>

      {sources.length === 0 && (
        <div className="mb-4 rounded-lg border border-[var(--border-subtle)] bg-secondary p-4 text-sm text-secondary">
          还没有连接远程源。切换到「远程源」Tab 添加一个 GitHub 仓库，即可搜索远程 Skill。
        </div>
      )}

      {local.length > 0 && (
        <Section title={<><Package className="inline h-4 w-4 mr-1.5" />本地匹配 ({local.length})</>}>
          <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
            {local.map((r) => (
              <SkillCard key={`local-${r.skill_name}`} r={r} onPreview={doPreview} onInstall={setInstallTarget} />
            ))}
          </div>
        </Section>
      )}

      {remote.length > 0 && (
        <Section title={<><Globe className="inline h-4 w-4 mr-1.5" />远程匹配 ({remote.length})</>}>
          <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
            {remote.map((r) => (
              <SkillCard key={`remote-${r.source_id}-${r.skill_name}`} r={r} onPreview={doPreview} onInstall={setInstallTarget} />
            ))}
          </div>
        </Section>
      )}

      {!loading && local.length === 0 && remote.length === 0 && (
        <div className="py-12 text-center text-sm text-tertiary">
          未找到匹配的 Skill。试试换个关键词，或连接更多远程源。
        </div>
      )}

      {previewing && (
        <PreviewDrawer
          title={previewing.skill_name}
          body={previewBody}
          onClose={() => setPreviewing(null)}
        />
      )}

      {installTarget && (
        <InstallDialog
          target={installTarget}
          onClose={() => setInstallTarget(null)}
        />
      )}
    </div>
  );
}

function Section({ title, children }: { title: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="mb-6">
      <h3 className="mb-2 text-sm font-semibold text-primary">{title}</h3>
      {children}
    </div>
  );
}

function SkillCard({
  r,
  onPreview,
  onInstall,
}: {
  r: SearchResult;
  onPreview: (r: SearchResult) => void;
  onInstall: (r: SearchResult) => void;
}) {
  return (
    <div className="rounded-lg border border-[var(--border-subtle)] bg-secondary p-3">
      <div className="mb-1 flex items-center justify-between">
        <span className="font-medium">{r.skill_name}</span>
        <div className="flex items-center gap-1.5">
          {r.match_field && <MatchBadge field={r.match_field} />}
          {r.installed_locally ? (
            <span className="rounded bg-tertiary px-2 py-0.5 text-xs text-primary">已装</span>
          ) : r.origin === "remote" ? (
            <span className="rounded bg-accent/20 px-2 py-0.5 text-xs text-accent">远程</span>
          ) : null}
        </div>
      </div>
      {r.snippet ? (
        <HighlightedSnippet text={r.snippet} />
      ) : r.description ? (
        <p className="mb-2 line-clamp-2 text-xs text-secondary">{r.description}</p>
      ) : null}
      <div className="mb-2 flex items-center gap-3 text-xs text-tertiary">
        {r.origin === "remote" && r.source_name && <span><Globe className="inline h-3 w-3 mr-1" />{r.source_name}</span>}
        {r.usage_count > 0 && <span><BarChart3 className="inline h-3 w-3 mr-1" />本周 {r.usage_count} 次</span>}
        {r.correction_count > 0 && <span>纠偏 {r.correction_count} 次</span>}
      </div>
      <div className="flex gap-2">
        {!r.installed_locally && (
          <button
            onClick={() => onInstall(r)}
            title="安装到中心仓库，并可选择同步到 Agent"
            className="rounded bg-accent px-3 py-1 text-xs font-medium text-primary hover:opacity-90"
          >
            安装
          </button>
        )}
        {r.origin === "remote" && (
          <button
            onClick={() => onPreview(r)}
            className="rounded border border-[var(--border-prominent)] px-3 py-1 text-xs text-primary hover:bg-tertiary"
          >
            预览
          </button>
        )}
      </div>
    </div>
  );
}

function MatchBadge({ field }: { field: string }) {
  const label =
    field === "name"
      ? "名称命中"
      : field === "description"
      ? "描述命中"
      : field === "tags"
      ? "标签命中"
      : field === "body"
      ? "正文命中"
      : field;
  return (
    <span className="rounded border border-accent/30 bg-accent/10 px-1.5 py-0.5 text-2xs text-accent">
      {label}
    </span>
  );
}

function HighlightedSnippet({ text }: { text: string }) {
  const parts = text.split(/(«[^»]+»)/g);
  return (
    <p className="mb-2 line-clamp-2 text-xs text-secondary">
      {parts.map((part, i) => {
        if (part.startsWith("«") && part.endsWith("»")) {
          return (
            <mark
              key={i}
              className="rounded bg-accent/30 px-0.5 font-medium text-accent"
            >
              {part.slice(1, -1)}
            </mark>
          );
        }
        return <span key={i}>{part}</span>;
      })}
    </p>
  );
}

function PreviewDrawer({ title, body, onClose }: { title: string; body: string; onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-50 flex justify-end bg-black/50" onClick={onClose}>
      <div
        className="h-full w-[480px] overflow-auto bg-primary p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-lg font-bold">{title}</h3>
          <button onClick={onClose} className="text-secondary hover:text-white">
            ✕
          </button>
        </div>
        <pre className="whitespace-pre-wrap break-words font-mono text-xs text-primary">{body}</pre>
      </div>
    </div>
  );
}

// =============================================================================
// Install dialog (with safety scan + agent selection)
// =============================================================================

function InstallDialog({ target, onClose }: { target: SearchResult; onClose: () => void }) {
  const { loadData } = useAppStore();
  const [agents, setAgents] = useState<Agent[]>([]);
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set());
  const [mode, setMode] = useState<"symlink" | "copy">("symlink");
  const [scan, setScan] = useState<SafetyScanResult | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [acknowledgedRisk, setAcknowledgedRisk] = useState(false);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    invoke<Agent[]>("get_agents")
      .then((a) => {
        setAgents(a);
        // Default-select enabled agents.
        setSelectedAgents(new Set(a.filter((x) => x.is_enabled).map((x) => x.id)));
      })
      .catch((err) => {
        const msg = typeof err === "string" ? err : String(err);
        showError(`加载 Agent 列表失败：${msg}`);
      });
    // Safety scan only applies to remote skills.
    setScanError(null);
    if (target.origin === "remote" && target.source_id) {
      setScanning(true);
      invoke<SafetyScanResult>("scan_skill_safety", {
        sourceId: target.source_id,
        skillPath: target.skill_path,
      })
        .then(setScan)
        .catch((err) => {
          const msg = typeof err === "string" ? err : String(err);
          setScanError(msg);
          setScan(null);
        })
        .finally(() => setScanning(false));
    } else {
      setScan({ clean: true, findings: [] });
    }
  }, [target]);

  const toggleAgent = (id: string) => {
    setSelectedAgents((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const doInstall = async () => {
    if (!target.source_id) {
      showError("缺少源信息，无法安装");
      return;
    }
    setInstalling(true);
    try {
      const confirmedRisks = scan && !scan.clean ? scan.findings.map((f) => f.rule) : [];
      const res = await invoke<{ synced_agents: string[]; skipped_agents?: string[] }>(
        "install_remote_skill",
        {
          sourceId: target.source_id,
          skillPath: target.skill_path,
          agentIds: Array.from(selectedAgents),
          mode,
          confirmedRisks,
        }
      );
      await loadData();
      const skipped = res.skipped_agents ?? [];
      const msg =
        `已安装「${target.skill_name}」并同步到 ${res.synced_agents.length} 个 Agent` +
        (res.synced_agents.length === 0 ? "（未选 Agent，仅入库）" : "") +
        (skipped.length > 0 ? `；${skipped.length} 个 Agent 已存在同名，已跳过：${skipped.join("、")}` : "");
      showSuccess(msg);
      onClose();
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`安装失败：${msg}`);
    } finally {
      setInstalling(false);
    }
  };

  const hasRisk = scan && !scan.clean;
  const canInstall = !scanning && ((!hasRisk && !scanError) || acknowledgedRisk) && !installing;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div
        className="max-h-[85vh] w-[560px] overflow-auto rounded-xl bg-primary p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-lg font-bold">安装 Skill：{target.skill_name}</h3>
          <button onClick={onClose} className="text-secondary hover:text-white">
            ✕
          </button>
        </div>

        {target.description && (
          <p className="mb-3 text-sm text-secondary">{target.description}</p>
        )}

        {/* Safety scan */}
        <div className="mb-4">
          <div className="mb-1 text-xs font-semibold text-secondary">⚠ 安全扫描</div>
          {scanning ? (
            <div className="text-sm text-tertiary">扫描中...</div>
          ) : scanError ? (
            <>
              <ErrorState
                title="安全扫描失败"
                message="无法确认该 Skill 的风险，请确认远程源可访问后重试。"
                detail={scanError}
                className="text-left"
              />
              <label className="mt-2 flex items-center gap-2 text-xs text-primary">
                <input
                  type="checkbox"
                  checked={acknowledgedRisk}
                  onChange={(e) => setAcknowledgedRisk(e.target.checked)}
                />
                我已知晓风险，仍要安装
              </label>
            </>
          ) : scan && scan.clean ? (
            <div className="rounded border border-green-700/50 bg-green-900/20 p-2 text-xs text-green-300">
              <CheckCircle2 className="inline h-4 w-4 mr-1.5" />未检测到高危指令
            </div>
          ) : scan ? (
            <div className="rounded border border-red-700/50 bg-red-900/20 p-2">
              <div className="mb-1 text-xs text-red-300">
                检测到 {scan.findings.length} 处潜在风险：
              </div>
              <ul className="space-y-1 text-xs text-red-200">
                {scan.findings.slice(0, 5).map((f, i) => (
                  <li key={i}>
                    第 {f.line} 行 · <span className="font-mono">{f.rule}</span>：{f.excerpt}
                  </li>
                ))}
              </ul>
              <label className="mt-2 flex items-center gap-2 text-xs text-primary">
                <input
                  type="checkbox"
                  checked={acknowledgedRisk}
                  onChange={(e) => setAcknowledgedRisk(e.target.checked)}
                />
                我已知晓风险，仍要安装
              </label>
            </div>
          ) : null}
        </div>

        {/* Agent selection */}
        <div className="mb-4">
          <div className="mb-1 text-xs font-semibold text-secondary">同步目标（勾选）</div>
          <div className="space-y-1">
            {agents.map((a) => (
              <label key={a.id} className="flex items-center gap-2 rounded px-2 py-1 hover:bg-secondary">
                <input
                  type="checkbox"
                  checked={selectedAgents.has(a.id)}
                  onChange={() => toggleAgent(a.id)}
                />
                <span className="text-sm">{a.name}</span>
                <span className="text-xs text-tertiary">{a.skill_directory}</span>
              </label>
            ))}
          </div>
        </div>

        {/* Sync mode */}
        <div className="mb-4">
          <div className="mb-1 text-xs font-semibold text-secondary">同步模式</div>
          <div className="flex gap-3 text-sm">
            <label className="flex items-center gap-1">
              <input type="radio" checked={mode === "symlink"} onChange={() => setMode("symlink")} />
              软链接（推荐）
            </label>
            <label className="flex items-center gap-1">
              <input type="radio" checked={mode === "copy"} onChange={() => setMode("copy")} />
              复制
            </label>
          </div>
        </div>

        <div className="flex justify-end gap-2">
          <button onClick={onClose} className="rounded border border-[var(--border-prominent)] px-4 py-1.5 text-sm text-primary hover:bg-secondary">
            取消
          </button>
          <button
            onClick={doInstall}
            disabled={!canInstall}
            className="rounded bg-accent px-4 py-1.5 text-sm font-medium text-primary disabled:opacity-50"
          >
            {installing ? "安装中..." : "确认安装"}
          </button>
        </div>
      </div>
    </div>
  );
}

// =============================================================================
// Tab 2: Sources management
// =============================================================================

function SourcesTab({
  sources,
  onChange,
}: {
  sources: Source[];
  onChange: () => void;
}) {
  const [adding, setAdding] = useState(false);
  const [selected, setSelected] = useState<Source | null>(null);

  const handleRemove = async (s: Source) => {
    if (!confirm(`确定移除远程源「${s.name}」？缓存将被删除。已安装的 Skill 不受影响。`)) return;
    try {
      await invoke("remove_source", { id: s.id });
      showSuccess("已移除源");
      onChange();
      setSelected(null);
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`移除失败：${msg}`);
    }
  };

  const handleRefresh = async (s: Source) => {
    try {
      await invoke("refresh_source", { id: s.id });
      showSuccess(`已刷新「${s.name}」`);
      onChange();
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`刷新失败：${msg}`);
    }
  };

  if (selected) {
    return (
      <SourceDetail
        source={selected}
        onBack={() => setSelected(null)}
        onRefresh={() => handleRefresh(selected)}
        onRemove={() => handleRemove(selected)}
      />
    );
  }

  return (
    <div>
      <div className="mb-4 flex justify-end">
        <button
          onClick={() => setAdding(true)}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-primary"
        >
          + 添加远程源
        </button>
      </div>

      {sources.length === 0 ? (
        <div className="rounded-lg border border-[var(--border-subtle)] bg-secondary p-8 text-center text-sm text-secondary">
          还没有远程源。连接一个 GitHub 仓库（如 <code className="text-accent">vercel-labs/agent-skills</code>），
          <br />
          开始发现并安装新 Skill。
        </div>
      ) : (
        <div className="space-y-2">
          {sources.map((s) => (
            <SourceRow key={s.id} source={s} onClick={() => setSelected(s)} />
          ))}
        </div>
      )}

      {adding && (
        <AddSourceDialog
          onClose={() => setAdding(false)}
          onAdded={() => {
            setAdding(false);
            onChange();
          }}
        />
      )}
    </div>
  );
}

function SourceRow({ source, onClick }: { source: Source; onClick: () => void }) {
  const [count, setCount] = useState<number | null>(null);
  const [countError, setCountError] = useState(false);
  useEffect(() => {
    setCountError(false);
    invoke<SkillRemoteMeta[]>("list_source_skills", { sourceId: source.id })
      .then((m) => setCount(m.length))
      .catch(() => {
        setCountError(true);
        setCount(null);
      });
  }, [source.id, source.last_fetched_at]);

  return (
    <button
      onClick={onClick}
      className="flex w-full items-center justify-between rounded-lg border border-[var(--border-subtle)] bg-secondary p-3 text-left hover:border-accent"
    >
      <div>
        <div className="font-medium">
          {source.source_type === "github" ? (
            <GitBranch className="inline h-4 w-4 mr-1" />
          ) : source.source_type === "local" ? (
            <Folder className="inline h-4 w-4 mr-1" />
          ) : (
            <LinkIcon className="inline h-4 w-4 mr-1" />
          )}{" "}
          {source.name}
        </div>
        <div className="text-xs text-tertiary">
          {source.url}@{source.ref_spec}
          {source.subpath && ` · 子目录 ${source.subpath}`}
        </div>
      </div>
      <div className="text-right text-xs text-secondary">
        <div>
          {countError
            ? "加载失败"
            : count === null
              ? "..."
              : `${count} skills`}
        </div>
        <div>
          {source.last_fetched_at
            ? `更新于 ${new Date(source.last_fetched_at * 1000).toLocaleString()}`
            : "未拉取"}
        </div>
      </div>
    </button>
  );
}

function SourceDetail({
  source,
  onBack,
  onRefresh,
  onRemove,
}: {
  source: Source;
  onBack: () => void;
  onRefresh: () => void;
  onRemove: () => void;
}) {
  const [metas, setMetas] = useState<SkillRemoteMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [installTarget, setInstallTarget] = useState<SkillRemoteMeta | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const m = await invoke<SkillRemoteMeta[]>("list_source_skills", { sourceId: source.id });
      setMetas(m);
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`加载源 Skill 失败：${msg}`);
    } finally {
      setLoading(false);
    }
  }, [source.id]);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <div>
      <div className="mb-4 flex items-center justify-between">
        <div>
          <button onClick={onBack} className="mb-1 text-xs text-secondary hover:text-white">
            ← 返回源列表
          </button>
          <h2 className="text-xl font-bold">{source.name}</h2>
          <div className="text-xs text-tertiary">
            {source.source_type}:{source.url}@{source.ref_spec} · commit {source.commit_sha.slice(0, 7)}
          </div>
        </div>
        <div className="flex gap-2">
          <button onClick={onRefresh} className="rounded border border-[var(--border-prominent)] px-3 py-1.5 text-sm text-primary hover:bg-secondary">
            立即刷新
          </button>
          <button onClick={onRemove} className="rounded border border-red-700/50 px-3 py-1.5 text-sm text-red-300 hover:bg-red-900/20">
            移除源
          </button>
        </div>
      </div>

      {loading ? (
        <SkeletonList count={6} />
      ) : metas.length === 0 ? (
        <div className="py-12 text-center text-sm text-tertiary">
          未在此源发现 SKILL.md。请检查子目录路径设置。
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
          {metas.map((m) => (
            <div key={m.skill_path} className="rounded-lg border border-[var(--border-subtle)] bg-secondary p-3">
              <div className="mb-1 flex items-center justify-between">
                <span className="font-medium">{m.skill_name}</span>
                {m.installed_locally ? (
                  <span className="rounded bg-tertiary px-2 py-0.5 text-xs text-primary">已装</span>
                ) : (
                  <button
                    onClick={() => setInstallTarget(m)}
                    title="安装到中心仓库，并可选择同步到 Agent"
                    className="rounded bg-accent px-3 py-0.5 text-xs font-medium text-primary hover:opacity-90"
                  >
                    安装
                  </button>
                )}
              </div>
              {m.description && <p className="text-xs text-secondary">{m.description}</p>}
            </div>
          ))}
        </div>
      )}

      {installTarget && (
        <InstallMetaDialog
          sourceId={source.id}
          meta={installTarget}
          onClose={() => setInstallTarget(null)}
        />
      )}
    </div>
  );
}

function InstallMetaDialog({
  sourceId,
  meta,
  onClose,
}: {
  sourceId: string;
  meta: SkillRemoteMeta;
  onClose: () => void;
}) {
  const { loadData } = useAppStore();
  const [agents, setAgents] = useState<Agent[]>([]);
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set());
  const [mode, setMode] = useState<"symlink" | "copy">("symlink");
  const [scan, setScan] = useState<SafetyScanResult | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [acknowledgedRisk, setAcknowledgedRisk] = useState(false);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    invoke<Agent[]>("get_agents")
      .then((a) => {
        setAgents(a);
        setSelectedAgents(new Set(a.filter((x) => x.is_enabled).map((x) => x.id)));
      })
      .catch((err) => {
        const msg = typeof err === "string" ? err : String(err);
        showError(`加载 Agent 列表失败：${msg}`);
      });
    setScanError(null);
    setScanning(true);
    invoke<SafetyScanResult>("scan_skill_safety", {
      sourceId,
      skillPath: meta.skill_path,
    })
      .then(setScan)
      .catch((err) => {
        const msg = typeof err === "string" ? err : String(err);
        setScanError(msg);
        setScan(null);
      })
      .finally(() => setScanning(false));
  }, [sourceId, meta]);

  const doInstall = async () => {
    setInstalling(true);
    try {
      const confirmedRisks = scan && !scan.clean ? scan.findings.map((f) => f.rule) : [];
      const res = await invoke<{ synced_agents: string[]; skipped_agents?: string[] }>(
        "install_remote_skill",
        {
          sourceId,
          skillPath: meta.skill_path,
          agentIds: Array.from(selectedAgents),
          mode,
          confirmedRisks,
        }
      );
      await loadData();
      const skipped = res.skipped_agents ?? [];
      const msg =
        `已安装「${meta.skill_name}」并同步到 ${res.synced_agents.length} 个 Agent` +
        (skipped.length > 0 ? `；${skipped.length} 个 Agent 已存在同名，已跳过：${skipped.join("、")}` : "");
      showSuccess(msg);
      onClose();
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`安装失败：${msg}`);
    } finally {
      setInstalling(false);
    }
  };

  const hasRisk = scan && !scan.clean;
  const canInstall = !scanning && ((!hasRisk && !scanError) || acknowledgedRisk) && !installing;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div className="max-h-[85vh] w-[520px] overflow-auto rounded-xl bg-primary p-6 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-lg font-bold">安装 Skill：{meta.skill_name}</h3>
          <button onClick={onClose} className="text-secondary hover:text-white">✕</button>
        </div>
        {meta.description && <p className="mb-3 text-sm text-secondary">{meta.description}</p>}

        <div className="mb-4">
          <div className="mb-1 text-xs font-semibold text-secondary">⚠ 安全扫描</div>
          {scanning ? (
            <div className="text-sm text-tertiary">扫描中...</div>
          ) : scanError ? (
            <>
              <ErrorState
                title="安全扫描失败"
                message="无法确认该 Skill 的风险，请确认远程源可访问后重试。"
                detail={scanError}
                className="text-left"
              />
              <label className="mt-2 flex items-center gap-2 text-xs text-primary">
                <input type="checkbox" checked={acknowledgedRisk} onChange={(e) => setAcknowledgedRisk(e.target.checked)} />
                我已知晓风险，仍要安装
              </label>
            </>
          ) : scan && scan.clean ? (
            <div className="rounded border border-green-700/50 bg-green-900/20 p-2 text-xs text-green-300">
              <CheckCircle2 className="inline h-4 w-4 mr-1.5" />未检测到高危指令
            </div>
          ) : scan ? (
            <div className="rounded border border-red-700/50 bg-red-900/20 p-2">
              <div className="mb-1 text-xs text-red-300">检测到 {scan.findings.length} 处潜在风险：</div>
              <ul className="space-y-1 text-xs text-red-200">
                {scan.findings.slice(0, 5).map((f, i) => (
                  <li key={i}>
                    第 {f.line} 行 · <span className="font-mono">{f.rule}</span>：{f.excerpt}
                  </li>
                ))}
              </ul>
              <label className="mt-2 flex items-center gap-2 text-xs text-primary">
                <input type="checkbox" checked={acknowledgedRisk} onChange={(e) => setAcknowledgedRisk(e.target.checked)} />
                我已知晓风险，仍要安装
              </label>
            </div>
          ) : null}
        </div>

        <div className="mb-4">
          <div className="mb-1 text-xs font-semibold text-secondary">同步目标（勾选）</div>
          <div className="space-y-1">
            {agents.map((a) => (
              <label key={a.id} className="flex items-center gap-2 rounded px-2 py-1 hover:bg-secondary">
                <input type="checkbox" checked={selectedAgents.has(a.id)} onChange={() => {
                  setSelectedAgents((prev) => {
                    const next = new Set(prev);
                    if (next.has(a.id)) next.delete(a.id); else next.add(a.id);
                    return next;
                  });
                }} />
                <span className="text-sm">{a.name}</span>
                <span className="text-xs text-tertiary">{a.skill_directory}</span>
              </label>
            ))}
          </div>
        </div>

        <div className="mb-4">
          <div className="mb-1 text-xs font-semibold text-secondary">同步模式</div>
          <div className="flex gap-3 text-sm">
            <label className="flex items-center gap-1">
              <input type="radio" checked={mode === "symlink"} onChange={() => setMode("symlink")} />
              软链接（推荐）
            </label>
            <label className="flex items-center gap-1">
              <input type="radio" checked={mode === "copy"} onChange={() => setMode("copy")} />
              复制
            </label>
          </div>
        </div>

        <div className="flex justify-end gap-2">
          <button onClick={onClose} className="rounded border border-[var(--border-prominent)] px-4 py-1.5 text-sm text-primary hover:bg-secondary">
            取消
          </button>
          <button onClick={doInstall} disabled={!canInstall} className="rounded bg-accent px-4 py-1.5 text-sm font-medium text-primary disabled:opacity-50">
            {installing ? "安装中..." : "确认安装"}
          </button>
        </div>
      </div>
    </div>
  );
}

function AddSourceDialog({ onClose, onAdded }: { onClose: () => void; onAdded: () => void }) {
  const [name, setName] = useState("");
  const [sourceType, setSourceType] = useState<SourceType>("github");
  const [url, setUrl] = useState("");
  const [refSpec, setRefSpec] = useState("main");
  const [subpath, setSubpath] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !url.trim()) {
      showError("请填写名称和来源");
      return;
    }
    setSubmitting(true);
    try {
      await invoke("add_source", {
        name: name.trim(),
        sourceType,
        url: url.trim(),
        refSpec: refSpec.trim() || "main",
        subpath: subpath.trim(),
      });
      showSuccess(`已连接「${name}」`);
      onAdded();
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`连接失败：${msg}`);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <form
        onSubmit={submit}
        className="w-[520px] rounded-xl bg-primary p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-lg font-bold">添加远程源</h3>
          <button type="button" onClick={onClose} className="text-secondary hover:text-white">
            ✕
          </button>
        </div>

        <div className="space-y-3">
          <Field label="类型">
            <select
              value={sourceType}
              onChange={(e) => setSourceType(e.target.value as SourceType)}
              className="w-full rounded border border-[var(--border-subtle)] bg-secondary px-3 py-2 text-sm"
            >
              <option value="github">GitHub 仓库</option>
              <option value="git">Git URL（当前仅支持 GitHub）</option>
              <option value="local">本地路径</option>
            </select>
          </Field>

          <Field label="名称（便于识别）">
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如：Vercel Agent Skills"
              className="w-full rounded border border-[var(--border-subtle)] bg-secondary px-3 py-2 text-sm"
            />
          </Field>

          <Field label={sourceType === "local" ? "本地路径" : "GitHub owner/repo 或 URL"}>
            <input
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder={sourceType === "local" ? "~/my-skills" : "vercel-labs/agent-skills"}
              className="w-full rounded border border-[var(--border-subtle)] bg-secondary px-3 py-2 text-sm"
            />
          </Field>

          {sourceType !== "local" && (
            <div className="flex gap-3">
              <Field label="分支/Tag" className="flex-1">
                <input
                  value={refSpec}
                  onChange={(e) => setRefSpec(e.target.value)}
                  placeholder="main"
                  className="w-full rounded border border-[var(--border-subtle)] bg-secondary px-3 py-2 text-sm"
                />
              </Field>
              <Field label="子目录（可选）" className="flex-1">
                <input
                  value={subpath}
                  onChange={(e) => setSubpath(e.target.value)}
                  placeholder="skills"
                  className="w-full rounded border border-[var(--border-subtle)] bg-secondary px-3 py-2 text-sm"
                />
              </Field>
            </div>
          )}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded border border-[var(--border-prominent)] px-4 py-1.5 text-sm text-primary hover:bg-secondary">
            取消
          </button>
          <button type="submit" disabled={submitting} className="rounded bg-accent px-4 py-1.5 text-sm font-medium text-primary disabled:opacity-50">
            {submitting ? "连接中..." : "连接并拉取"}
          </button>
        </div>
      </form>
    </div>
  );
}

function Field({
  label,
  children,
  className,
}: {
  label: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <label className={`block ${className || ""}`}>
      <div className="mb-1 text-xs font-semibold text-secondary">{label}</div>
      {children}
    </label>
  );
}

// =============================================================================
// Tab 3: Updates (skills whose source has a newer revision)
// =============================================================================

function UpdatesTab({ sources, onChange }: { sources: Source[]; onChange: () => void }) {
  const [checking, setChecking] = useState(false);
  const [failed, setFailed] = useState<Source[]>([]);

  const refreshAll = useCallback(async () => {
    setChecking(true);
    const errored: Source[] = [];
    for (const s of sources) {
      if (s.source_type === "local") continue;
      try {
        await invoke<Source>("refresh_source", { id: s.id });
      } catch {
        errored.push(s);
      }
    }
    setFailed(errored);
    setChecking(false);
    onChange();
    if (errored.length === 0) {
      showSuccess("所有源已刷新到最新");
    } else {
      showError(`${errored.length} 个源刷新失败，详见下方列表`);
    }
  }, [sources, onChange]);

  if (sources.filter((s) => s.source_type !== "local").length === 0) {
    return (
      <div className="py-12 text-center text-sm text-tertiary">
        暂无远程源。连接 Git/GitHub 源后，可在此一键批量刷新缓存。
      </div>
    );
  }

  return (
    <div>
      <div className="mb-4 flex items-center justify-between">
        <p className="max-w-xl text-sm text-secondary">
          一键重新拉取所有已连接远程源的缓存。刷新后，上游最新的 Skill 会出现在「源详情」与「搜索」结果中。
        </p>
        <button
          onClick={refreshAll}
          disabled={checking}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-primary disabled:opacity-50"
        >
          {checking ? "刷新中..." : "刷新所有源"}
        </button>
      </div>

      {failed.length > 0 && (
        <div className="rounded-lg border border-yellow-700/50 bg-yellow-900/20 p-3 text-sm text-yellow-200">
          {failed.length} 个源刷新失败，可能是网络或权限问题：
          <ul className="mt-1 list-inside list-disc">
            {failed.map((s) => (
              <li key={s.id}>{s.name}（{s.url}）</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
