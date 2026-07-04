import { useCallback, useEffect, useState } from "react";
import { invoke } from "../lib/invoke";
import { RotateCcw, Trash2, AlertTriangle } from "lucide-react";
import { showError, showSuccess } from "../stores/toastStore";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";
import { Dialog, DialogActions } from "./ui/Dialog";
import { EmptyState } from "./ui/EmptyState";
import type { RestoreResult, TrashItem } from "../types";

const CONFIRM_CODE = "DELETE";
const DAY_MS = 86400 * 1000;

type ConflictState = {
  item: TrashItem;
  conflictName: string;
};

/**
 * SPEC-C3 T4: recycle bin panel. Lists trashed skills with remaining days and
 * offers restore (with three-way conflict resolution) and purge (heavy
 * confirm, confirm-code gated).
 */
export default function TrashPanel() {
  const [items, setItems] = useState<TrashItem[] | null>(null);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [purgeTarget, setPurgeTarget] = useState<TrashItem | null>(null);
  const [purgeConfirm, setPurgeConfirm] = useState("");
  const [conflict, setConflict] = useState<ConflictState | null>(null);

  const load = useCallback(async () => {
    try {
      const list = await invoke<TrashItem[]>("list_trash_items");
      setItems(list);
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`加载回收站失败：${message}`);
      setItems([]);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const remainingDays = (item: TrashItem): number => {
    const now = Date.now();
    return Math.max(0, Math.ceil((item.expires_at * 1000 - now) / DAY_MS));
  };

  const formatDate = (ts: number): string => {
    try {
      return new Date(ts * 1000).toLocaleDateString();
    } catch {
      return "—";
    }
  };

  const handleRestore = useCallback(
    async (item: TrashItem, strategy: string | null) => {
      setBusyId(item.id);
      try {
        const result = await invoke<RestoreResult>("restore_trash_item", {
          id: item.id,
          conflictStrategy: strategy,
        });
        if (result.skipped_bindings.length > 0) {
          showSuccess(
            `已恢复「${result.final_name}」（${result.skipped_bindings.length} 个失效项目绑定已跳过）`,
            6000,
          );
        } else {
          showSuccess(`已恢复「${result.final_name}」，同步状态为待同步，可在详情页同步到 Agent`);
        }
        setConflict(null);
        await load();
      } catch (err) {
        // The invoke wrapper humanizes the message; the raw backend string is
        // preserved in `detail`. Check both so restore_conflict is detected
        // regardless of the wrapper's transformation.
        const raw =
          typeof err === "string"
            ? err
            : (err as Error & { detail?: string })?.detail ??
              (err instanceof Error ? err.message : String(err));
        if (raw.startsWith("restore_conflict")) {
          // Surface the three-way picker.
          setConflict({ item, conflictName: item.original_name });
        } else {
          const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
          showError(`恢复失败：${message}`);
        }
      } finally {
        setBusyId(null);
      }
    },
    [load],
  );

  const handlePurge = useCallback(async () => {
    if (!purgeTarget) return;
    if (purgeConfirm !== CONFIRM_CODE) {
      showError("确认码错误，操作已取消");
      return;
    }
    setBusyId(purgeTarget.id);
    try {
      await invoke("purge_trash_item", { id: purgeTarget.id, confirm: purgeConfirm });
      showSuccess(`已彻底删除「${purgeTarget.original_name}」`);
      setPurgeTarget(null);
      setPurgeConfirm("");
      await load();
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`彻底删除失败：${message}`);
    } finally {
      setBusyId(null);
    }
  }, [purgeTarget, purgeConfirm, load]);

  return (
    <Card className="p-0 overflow-hidden">
      <div className="border-b border-[var(--border-subtle)] px-6 py-4">
        <div className="flex items-center gap-2">
          <Trash2 className="h-4 w-4 text-accent" />
          <h3 className="font-semibold text-primary">回收站</h3>
        </div>
        <p className="mt-1 text-xs text-secondary">
          删除的 Skill 会在这里保留 30 天，之后将被永久清除。重置数据库不会清空回收站快照。
        </p>
      </div>

      {items === null ? (
        <div className="px-6 py-8 text-sm text-secondary">加载中…</div>
      ) : items.length === 0 ? (
        <div className="px-6 py-4">
          <EmptyState icon={Trash2} title="回收站是空的" description="删除的 Skill 会在这里保留 30 天。" />
        </div>
      ) : (
        <div className="divide-y divide-[var(--border-subtle)]">
          {items.map((item) => {
            const days = remainingDays(item);
            const expiring = days <= 3;
            return (
              <div key={item.id} className="flex items-center justify-between px-6 py-4">
                <div className="min-w-0">
                  <div className="font-medium text-primary truncate">{item.original_name}</div>
                  <div className="mt-1 flex items-center gap-2 text-xs text-secondary">
                    <span>删除于 {formatDate(item.deleted_at)}</span>
                    <span>·</span>
                    {expiring ? (
                      <span className="text-danger">剩余 {days} 天</span>
                    ) : (
                      <span>剩余 {days} 天</span>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => handleRestore(item, null)}
                    loading={busyId === item.id}
                    disabled={busyId === item.id}
                  >
                    <RotateCcw className="mr-1 h-3.5 w-3.5" />
                    恢复
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setPurgeTarget(item);
                      setPurgeConfirm("");
                    }}
                    disabled={busyId === item.id}
                  >
                    <Trash2 className="mr-1 h-3.5 w-3.5 text-danger" />
                    彻底删除
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Restore conflict three-way picker. */}
      <Dialog
        open={conflict !== null}
        onClose={() => setConflict(null)}
        title="恢复时遇到重名"
        description={`名为「${conflict?.conflictName ?? ""}」的 Skill 已存在，请选择恢复方式：`}
      >
        <div className="space-y-2 text-sm text-secondary">
          <p>
            <strong className="text-primary">覆盖现有</strong>
            ：现有版本也会被移入回收站（双向可逆）。
          </p>
          <p>
            <strong className="text-primary">重命名恢复</strong>
            ：恢复为「{conflict?.conflictName ?? ""}-restored」。
          </p>
        </div>
        <DialogActions>
          <Button variant="secondary" size="sm" onClick={() => setConflict(null)}>
            取消
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => conflict && handleRestore(conflict.item, "rename")}
            loading={conflict?.item ? busyId === conflict.item.id : false}
          >
            重命名恢复
          </Button>
          <Button
            variant="danger"
            size="sm"
            onClick={() => conflict && handleRestore(conflict.item, "overwrite")}
            loading={conflict?.item ? busyId === conflict.item.id : false}
          >
            覆盖现有
          </Button>
        </DialogActions>
      </Dialog>

      {/* Purge heavy-confirm dialog. */}
      <Dialog
        open={purgeTarget !== null}
        onClose={() => {
          setPurgeTarget(null);
          setPurgeConfirm("");
        }}
        title="彻底删除？"
      >
        <div className="flex items-center gap-2 text-danger">
          <AlertTriangle className="h-5 w-5" />
          <span className="font-medium">此操作不可逆</span>
        </div>
        <p className="mt-2 text-sm text-secondary">
          Skill「{purgeTarget?.original_name}」的快照与记录将被永久删除，无法恢复。
        </p>
        <p className="mt-4 text-sm text-primary">
          请输入确认码 <code className="rounded bg-tertiary px-1 py-0.5 text-xs">{CONFIRM_CODE}</code> 以继续：
        </p>
        <input
          type="text"
          value={purgeConfirm}
          onChange={(e) => setPurgeConfirm(e.target.value)}
          placeholder={CONFIRM_CODE}
          className="mt-2 w-full rounded-lg border border-[var(--border-subtle)] bg-primary px-3 py-2 text-sm text-primary outline-none focus:border-accent"
        />
        <DialogActions>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              setPurgeTarget(null);
              setPurgeConfirm("");
            }}
          >
            取消
          </Button>
          <Button
            variant="danger"
            size="sm"
            onClick={handlePurge}
            loading={purgeTarget ? busyId === purgeTarget.id : false}
          >
            确认彻底删除
          </Button>
        </DialogActions>
      </Dialog>
    </Card>
  );
}
