import { useEffect } from "react";
import { Database, RefreshCw } from "lucide-react";
import { useCollectionStore } from "../stores/collectionStore";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";
import { Badge } from "./ui/Badge";
import { SkeletonList } from "./ui/Skeleton";
import { EmptyState } from "./ui/EmptyState";
import { HelpTip } from "./ui/HelpTip";

const STATUS_META: Record<string, { variant: "success" | "default" | "danger" | "warning"; label: string }> = {
  ok: { variant: "success", label: "已采集" },
  not_found: { variant: "default", label: "未发现" },
  error: { variant: "danger", label: "采集出错" },
  schema_incompatible: { variant: "warning", label: "格式不兼容" },
  unsupported: { variant: "default", label: "不支持" },
};

function statusOf(status: string) {
  return STATUS_META[status] ?? { variant: "default" as const, label: status };
}

function timeAgo(ts?: number): string {
  if (!ts) return "—";
  const diff = Math.max(0, Date.now() / 1000 - ts);
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

export interface DataCollectionPanelProps {
  /** When true, renders a compact card suitable for embedding in Usage/Settings. */
  compact?: boolean;
  /** Hide the path column in compact mode. */
  showPath?: boolean;
  /** Custom title. */
  title?: string;
}

export default function DataCollectionPanel({
  compact = false,
  showPath = true,
  title = "使用数据采集",
}: DataCollectionPanelProps) {
  const sources = useCollectionStore((state) => state.sources);
  const loading = useCollectionStore((state) => state.loading);
  const collecting = useCollectionStore((state) => state.collecting);
  const loadStatus = useCollectionStore((state) => state.loadStatus);
  const startCollect = useCollectionStore((state) => state.startCollect);

  useEffect(() => {
    loadStatus();
  }, [loadStatus]);

  const handleCollect = async () => {
    await startCollect();
  };

  const tableContent = (
    <div className="overflow-hidden">
      <div className="overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-[var(--divider)] text-secondary">
              <th className="px-4 py-3 font-medium">状态</th>
              <th className="px-4 py-3 font-medium">来源</th>
              {showPath && <th className="px-4 py-3 font-medium">采集路径</th>}
              <th className="px-4 py-3 font-medium">状态</th>
              <th className="px-4 py-3 font-medium text-right">会话数</th>
              <th className="px-4 py-3 font-medium text-right">最近采集</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[var(--divider)]">
            {sources.map((s) => {
              const meta = statusOf(s.status);
              return (
                <tr key={s.source} className="group transition-colors hover:bg-tertiary/30">
                  <td className="px-4 py-3">
                    <span
                      className={`inline-block h-2.5 w-2.5 rounded-full ${
                        s.status === "ok"
                          ? "bg-success"
                          : s.status === "error"
                          ? "bg-danger"
                          : s.status === "schema_incompatible"
                          ? "bg-warning"
                          : "bg-tertiary"
                      }`}
                    />
                  </td>
                  <td className="px-4 py-3">
                    <div className="font-medium text-primary">{s.source}</div>
                    {!showPath && (
                      <div className="mt-0.5 text-xs text-tertiary">{s.data_path}</div>
                    )}
                  </td>
                  {showPath && (
                    <td className="px-4 py-3">
                      <div className="max-w-xs truncate text-tertiary" title={s.data_path}>
                        {s.data_path}
                      </div>
                    </td>
                  )}
                  <td className="px-4 py-3">
                    <Badge variant={meta.variant} size="sm">
                      {meta.label}
                    </Badge>
                  </td>
                  <td className="px-4 py-3 text-right font-medium text-primary">
                    {s.record_count} 会话
                  </td>
                  <td className="px-4 py-3 text-right text-secondary">
                    {timeAgo(s.last_collected_at)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );

  const actionButton = (
    <Button
      variant="primary"
      size={compact ? "sm" : "md"}
      onClick={handleCollect}
      loading={collecting}
      disabled={collecting || loading}
      title="增量读取各 Agent 本地使用数据，未变化文件自动跳过"
    >
      <RefreshCw className={`h-4 w-4 ${collecting ? "animate-spin" : ""}`} />
      {collecting ? "采集中…" : "立即采集"}
    </Button>
  );

  if (compact) {
    return (
      <Card padding="none">
        <div className="flex items-center justify-between border-b border-[var(--divider)] px-5 py-4">
          <div className="flex items-center gap-2">
            <Database className="h-5 w-5 text-secondary" />
            <h2 className="text-base font-semibold text-primary">{title}</h2>
          </div>
          {actionButton}
        </div>
        {loading && sources.length === 0 ? (
          <SkeletonList count={4} />
        ) : sources.length === 0 ? (
          <div className="p-6 text-sm text-secondary">
            还没有数据源。安装并使用过 Agent 后点击「立即采集」即可发现数据。
          </div>
        ) : (
          tableContent
        )}
      </Card>
    );
  }

  return (
    <div className="flex h-full flex-col p-8">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="flex items-center gap-1.5 text-3xl font-semibold tracking-tight text-primary">
            {title}
            {/* SPEC-F6 T2: 就地解释「采集」。 */}
            <HelpTip
              ariaLabel="什么是数据采集"
              text="只读读取各 Agent 落盘的使用记录（会话、Token），不会修改 Agent 的任何文件。"
            />
          </h1>
          <p className="mt-1 text-sm text-secondary">管理各 Agent 的使用数据采集状态</p>
        </div>
        {actionButton}
      </div>

      {loading && sources.length === 0 ? (
        <SkeletonList count={6} />
      ) : sources.length === 0 ? (
        <EmptyState
          icon={Database}
          title="还没有数据源"
          description="安装并使用过 Agent 后点击「立即采集」即可发现数据。"
          action={actionButton}
        />
      ) : (
        <Card className="flex-1 overflow-hidden p-0" padding="none">
          {tableContent}
        </Card>
      )}
    </div>
  );
}
