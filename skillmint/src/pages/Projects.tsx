/**
 * P3-6b: Projects is now a pure usage-analytics page (project ↔ agent activity).
 * The legacy center-repo install / version / binding UI was retired with the
 * center repo — project-level skills live in `<project>/.skillmint/hub`
 * (authoring) and `.agents/skills` + per-agent links (npx project installs).
 */
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FolderGit, RefreshCw } from "lucide-react";
import { showError, showSuccess } from "../stores/toastStore";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/ui/EmptyState";
import { Skeleton, SkeletonList } from "../components/ui/Skeleton";
import type { ProjectDetail, ProjectUsageSummary } from "../types";

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
  const [loading, setLoading] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const projs = await invoke<ProjectUsageSummary[]>("get_projects");
      setProjects(projs);
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
          <p className="mt-1 text-sm text-secondary">项目维度的 Agent 使用洞察</p>
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
    </div>
  );
}

// =============================================================================
// Project detail view — usage analytics only (P3-6b: legacy skill install /
// version / binding UI retired with the center repo).
// =============================================================================

function ProjectDetailView({ projectId, onBack }: { projectId: string; onBack: () => void }) {
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [loading, setLoading] = useState(true);

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
          <button
            onClick={() => invoke("open_project_in_finder", { path: detail.path }).catch(() => {})}
            className="rounded-lg border border-[var(--border-prominent)] px-3 py-1.5 text-sm text-primary hover:bg-tertiary"
          >
            打开 Finder
          </button>
        </div>
        <div className="mt-2 break-all text-sm text-tertiary">{detail.path}</div>
        <div className="mt-3 flex flex-wrap gap-x-6 gap-y-1 text-sm text-primary">
          <span>最近活跃：{timeAgo(detail.last_active_at)}</span>
          <span>{detail.session_count} 会话</span>
          <span>{formatTokens(detail.total_tokens)} tokens</span>
          <span>Agents：{detail.agents.length}</span>
        </div>
      </div>

      {/* Per-agent activity */}
      {detail.agents.length === 0 ? (
        <section className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-secondary p-8 text-center text-sm text-secondary">
          该项目尚未发现任何 Agent 活动。前往「使用洞察」点「立即采集」后，重新扫描项目即可看到关联 Agent。
        </section>
      ) : (
        <div className="space-y-3">
          {detail.agents.map((a) => (
            <div
              key={a.agent_id}
              className="flex items-center justify-between rounded-xl border border-[var(--border-subtle)] bg-secondary px-5 py-4"
            >
              <div>
                <div className="font-medium">{a.agent_name}</div>
                <div className="mt-0.5 text-xs text-tertiary">
                  最近活跃：{timeAgo(a.last_session_at)}
                </div>
              </div>
              <div className="flex items-center gap-6 text-sm text-primary">
                <span>{a.session_count} 会话</span>
                <span>{formatTokens(a.total_tokens)} tokens</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
