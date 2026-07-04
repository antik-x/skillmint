import { useEffect, useState } from "react";
import { invoke } from "../lib/invoke";
import { Database, Download, FileJson, HardDrive, Trash2, AlertTriangle, Wrench } from "lucide-react";
import { showError, showSuccess } from "../stores/toastStore";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";
import { Skeleton, SkeletonList } from "./ui/Skeleton";
import TrashPanel from "./TrashPanel";
import type { DataDictionary, RawDataExport, TableInfo } from "../types";

const DEFAULT_TABLES = ["skills", "collected_sessions", "projects", "skill_bundles"];
const CONFIRM_CODE = "DELETE";

export default function DataManagementPanel() {
  const [dict, setDict] = useState<DataDictionary | null>(null);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set(DEFAULT_TABLES));
  const [exporting, setExporting] = useState(false);
  const [dialogOpen, setDialogOpen] = useState<"clear" | "reset" | null>(null);
  const [confirm, setConfirm] = useState("");
  const [dangerLoading, setDangerLoading] = useState(false);
  const [repairLoading, setRepairLoading] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      const d = await invoke<DataDictionary>("export_data_dictionary");
      setDict(d);
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`加载数据字典失败：${message}`);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const toggleTable = (name: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const handleExportJson = async () => {
    if (selected.size === 0) return;
    setExporting(true);
    try {
      const result = await invoke<RawDataExport>("export_raw_data", {
        tables: Array.from(selected),
      });
      const blob = new Blob([JSON.stringify(result.data, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `skillmint-export-${new Date().toISOString().slice(0, 10)}.json`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      showSuccess(`已导出 ${Array.from(selected).join(", ")}`);
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`导出失败：${message}`);
    } finally {
      setExporting(false);
    }
  };

  const runDangerAction = async () => {
    if (confirm !== CONFIRM_CODE) {
      showError("确认码错误，操作已取消");
      return;
    }
    setDangerLoading(true);
    try {
      if (dialogOpen === "clear") {
        await invoke("clear_collected_data", { confirm });
        showSuccess("已清除采集数据");
      } else {
        await invoke("reset_database", { confirm });
        showSuccess("数据库已重置，请刷新页面");
      }
      setDialogOpen(null);
      setConfirm("");
      load();
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`操作失败：${message}`);
    } finally {
      setDangerLoading(false);
    }
  };

  // SPEC-F5 T5: user-invoked repo-path repair. Diagnostic only — no confirm code.
  const handleRepairPaths = async () => {
    setRepairLoading(true);
    try {
      const result = await invoke<{ migrated: string[]; failed: { skill: string; reason: string }[] }>("repair_skill_paths");
      const migrated = result.migrated.length;
      const failed = result.failed.length;
      if (migrated === 0 && failed === 0) {
        showSuccess("仓库路径检查完成，无需修复。");
      } else if (failed === 0) {
        showSuccess(`已修复 ${migrated} 个 Skill 的仓库路径。`);
      } else {
        const detail = result.failed.map((f) => `${f.skill}（${f.reason}）`).join("、");
        showError(`修复 ${migrated} 个成功，${failed} 个失败：${detail}`);
      }
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`修复失败：${message}`);
    } finally {
      setRepairLoading(false);
    }
  };

  const dangerTitle = dialogOpen === "clear" ? "清除采集数据" : "重置数据库";
  const dangerItems =
    dialogOpen === "clear"
      ? ["所有采集来源状态", "采集到的会话、Prompt、Token 用量", "采集任务记录", "分析缓存"]
      : ["所有 Skill", "所有 Agent 与同步目标", "所有项目与绑定关系", "所有采集数据与设置"];

  return (
    <div className="space-y-6">
      <Card className="p-0 overflow-hidden">
        <div className="border-b border-[var(--border-subtle)] px-6 py-4">
          <div className="flex items-center gap-2">
            <HardDrive className="h-4 w-4 text-accent" />
            <h3 className="font-semibold text-primary">本地数据库</h3>
          </div>
          <p className="mt-1 text-xs text-secondary">所有 Skill、会话、同步状态都保存在这个 SQLite 文件中。</p>
        </div>
        <div className="px-6 py-4">
          {loading || !dict ? (
            <div className="flex items-center gap-3 rounded-lg border border-[var(--border-subtle)] bg-primary/50 px-4 py-3">
              <Skeleton circle className="h-5 w-5" />
              <Skeleton className="h-4 w-2/3" />
            </div>
          ) : (
            <div className="flex items-center gap-3 rounded-lg border border-[var(--border-subtle)] bg-primary/50 px-4 py-3">
              <Database className="h-5 w-5 text-tertiary" />
              <code className="flex-1 break-all text-xs text-primary">{dict.db_path}</code>
            </div>
          )}
        </div>
      </Card>

      <Card className="p-0 overflow-hidden">
        <div className="border-b border-[var(--border-subtle)] px-6 py-4">
          <div className="flex items-center gap-2">
            <FileJson className="h-4 w-4 text-accent" />
            <h3 className="font-semibold text-primary">数据字典</h3>
          </div>
          <p className="mt-1 text-xs text-secondary">核心表说明与字段含义。</p>
        </div>
        <div className="divide-y divide-[var(--border-subtle)]">
          {dict?.tables.map((table: TableInfo) => (
            <div key={table.name} className="px-6 py-4">
              <div className="flex items-center justify-between">
                <div className="font-medium text-primary">{table.name}</div>
                <label className="flex items-center gap-1.5 text-xs text-secondary">
                  <input
                    type="checkbox"
                    checked={selected.has(table.name)}
                    onChange={() => toggleTable(table.name)}
                    className="accent-accent"
                  />
                  导出
                </label>
              </div>
              <div className="mt-1 text-xs text-secondary">{table.description}</div>
              <div className="mt-2 flex flex-wrap gap-1">
                {table.columns.map((col) => (
                  <span key={col} className="rounded bg-tertiary/50 px-1.5 py-0.5 text-2xs text-secondary">
                    {col}
                  </span>
                ))}
              </div>
            </div>
          ))}
          {!dict && (
            <div className="px-4 py-4">
              <SkeletonList count={4} />
            </div>
          )}
        </div>
      </Card>

      <Card className="p-0 overflow-hidden">
        <div className="border-b border-[var(--border-subtle)] px-6 py-4">
          <div className="flex items-center gap-2">
            <Download className="h-4 w-4 text-accent" />
            <h3 className="font-semibold text-primary">导出原始数据</h3>
          </div>
          <p className="mt-1 text-xs text-secondary">将选中的表导出为 JSON，方便备份或二次分析。</p>
        </div>
        <div className="flex items-center justify-between px-6 py-4">
          <div className="text-sm text-secondary">
            已选择 {selected.size} 个表
          </div>
          <Button
            variant="primary"
            size="sm"
            onClick={handleExportJson}
            loading={exporting}
            disabled={selected.size === 0}
          >
            <Download className="mr-1 h-4 w-4" />
            导出 JSON
          </Button>
        </div>
      </Card>

      {/* SPEC-F5 T5: one-click repo-path repair. Diagnostic only — safe to run
          any time, no confirm code. */}
      <Card className="p-0 overflow-hidden">
        <div className="border-b border-[var(--border-subtle)] px-6 py-4">
          <div className="flex items-center gap-2">
            <Wrench className="h-4 w-4 text-accent" />
            <h3 className="font-semibold text-primary">仓库路径修复</h3>
          </div>
          <p className="mt-1 text-xs text-secondary">
            将散落在仓库根目录的历史 Skill 收敛进 <code>repo/</code> 子目录并更新数据库路径。诊断类操作，可安全重复执行。
          </p>
        </div>
        <div className="flex items-center justify-between px-6 py-4">
          <div className="text-sm text-secondary">
            适用于从旧版本升级后 Skill 文件未归位的情况。
          </div>
          <Button variant="secondary" size="sm" onClick={handleRepairPaths} loading={repairLoading}>
            <Wrench className="mr-1 h-4 w-4" />
            修复仓库路径
          </Button>
        </div>
      </Card>

      {/* SPEC-C3 T4: recycle bin sits above the danger zone so users reach it
          before the irreversible operations. */}
      <TrashPanel />

      <Card className="p-0 overflow-hidden border-danger/20">
        <div className="border-b border-danger/10 px-6 py-4">
          <div className="flex items-center gap-2">
            <AlertTriangle className="h-4 w-4 text-danger" />
            <h3 className="font-semibold text-primary">危险操作</h3>
          </div>
          <p className="mt-1 text-xs text-secondary">这些操作不可逆，请谨慎使用。</p>
        </div>
        <div className="flex flex-col gap-3 px-6 py-4 sm:flex-row">
          <Button variant="danger" size="sm" onClick={() => setDialogOpen("clear")}>
            <Trash2 className="mr-1 h-4 w-4" />
            清除采集数据
          </Button>
          <Button variant="danger" size="sm" onClick={() => setDialogOpen("reset")}>
            <Trash2 className="mr-1 h-4 w-4" />
            重置数据库
          </Button>
        </div>
      </Card>

      {dialogOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
          <div className="w-full max-w-md rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-primary)] p-6 shadow-xl">
            <div className="flex items-center gap-2 text-danger">
              <AlertTriangle className="h-5 w-5" />
              <h3 className="text-lg font-semibold">{dangerTitle}</h3>
            </div>
            <p className="mt-2 text-sm text-secondary">
              此操作不可逆，将删除以下内容：
            </p>
            <ul className="mt-2 list-inside list-disc text-sm text-secondary">
              {dangerItems.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
            <p className="mt-4 text-sm text-primary">
              请输入确认码 <code className="rounded bg-tertiary px-1 py-0.5 text-xs">{CONFIRM_CODE}</code> 以继续：
            </p>
            <input
              type="text"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              placeholder={CONFIRM_CODE}
              className="mt-2 w-full rounded-lg border border-[var(--border-subtle)] bg-primary px-3 py-2 text-sm text-primary outline-none focus:border-accent"
            />
            <div className="mt-6 flex justify-end gap-3">
              <Button variant="secondary" size="sm" onClick={() => { setDialogOpen(null); setConfirm(""); }}>
                取消
              </Button>
              <Button variant="danger" size="sm" loading={dangerLoading} onClick={runDangerAction}>
                确认{dangerTitle}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
