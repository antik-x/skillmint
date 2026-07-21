import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { GitBranch } from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { showError, showInfo, showSuccess } from "../stores/toastStore";
import { Button } from "./ui/Button";
import type { AppSettings, RestoreSummary, SnapshotInfo, GitCommit } from "../types";

export default function PreferencesPanel() {
  const { settings, setSettings } = useAppStore();
  const [form, setForm] = useState<AppSettings>(settings);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setForm(settings);
  }, [settings]);

  const handleSave = async () => {
    if (saving) return;
    setSaving(true);
    try {
      const saved = await invoke<AppSettings>("save_settings", { newSettings: form });
      setSettings(saved);
      showSuccess("设置已保存");
    } catch (err) {
      console.error("[SkillMint] save_settings failed:", err);
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`保存失败：${message}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="space-y-6 rounded-xl border border-[var(--border-subtle)] bg-secondary p-6">
        <h2 className="text-lg font-semibold">常规</h2>
        <div>
          <label className="mb-2 block text-sm text-secondary">中心仓库路径</label>
          <input
            type="text"
            value={form.center_repo}
            onChange={(e) => setForm({ ...form, center_repo: e.target.value })}
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-4 py-2 text-primary focus:border-accent focus:outline-none"
          />
          <p className="mt-1 text-xs text-tertiary">所有 Skill 的唯一信源目录。</p>
        </div>

        <div>
          <label className="mb-2 block text-sm text-secondary">默认同步模式</label>
          <select
            value={form.default_sync_mode}
            onChange={(e) =>
              setForm({ ...form, default_sync_mode: e.target.value as AppSettings["default_sync_mode"] })
            }
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-4 py-2 text-primary focus:border-accent focus:outline-none"
          >
            <option value="symlink">软链接（推荐）</option>
            <option value="copy">复制</option>
          </select>
        </div>

        <div>
          <label className="mb-2 block text-sm text-secondary">自动同步间隔（分钟，0 为关闭）</label>
          <input
            type="number"
            min={0}
            max={120}
            value={form.auto_sync_interval_minutes}
            onChange={(e) =>
              setForm({ ...form, auto_sync_interval_minutes: parseInt(e.target.value) || 0 })
            }
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-4 py-2 text-primary focus:border-accent focus:outline-none"
          />
        </div>

        <div className="flex items-center justify-between rounded-lg border border-[var(--border-prominent)] p-4">
          <div>
            <div className="font-medium">开机自启</div>
            <div className="text-xs text-tertiary">登录后自动在菜单栏启动 SkillMint</div>
          </div>
          <input
            type="checkbox"
            checked={form.launch_at_login}
            onChange={(e) => setForm({ ...form, launch_at_login: e.target.checked })}
            className="h-5 w-5 accent-accent"
          />
        </div>

        <div className="flex items-center justify-between rounded-lg border border-[var(--border-prominent)] p-4">
          <div>
            <div className="font-medium">显示 Dock 图标</div>
            <div className="text-xs text-tertiary">关闭后只在菜单栏显示</div>
          </div>
          <input
            type="checkbox"
            checked={form.show_dock_icon}
            onChange={(e) => setForm({ ...form, show_dock_icon: e.target.checked })}
            className="h-5 w-5 accent-accent"
          />
        </div>

        <div className="rounded-lg border border-[var(--border-prominent)] p-4">
          <div className="font-medium">设备 ID</div>
          <code className="mt-1 block break-all rounded bg-primary px-3 py-2 text-xs text-primary">
            {settings.device_id || "未生成"}
          </code>
          <div className="mt-1 text-xs text-tertiary">
            用于多设备数据隔离，不含任何硬件或个人信息。设备内稳定，仅在重置时变化。
          </div>
        </div>

        <div className="flex items-center justify-between rounded-lg border border-[var(--border-prominent)] p-4">
          <div>
            <div className="font-medium">外观</div>
            <div className="text-xs text-tertiary">选择 App 使用的明暗主题。</div>
          </div>
          <select
            aria-label="外观"
            value={form.theme}
            onChange={(e) => {
              const theme = e.target.value as AppSettings["theme"];
              setForm({ ...form, theme });
              if (theme === "light") {
                document.documentElement.dataset.theme = "light";
              } else if (theme === "dark") {
                document.documentElement.dataset.theme = "dark";
              } else {
                delete document.documentElement.dataset.theme;
              }
            }}
            className="rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-2 text-sm text-primary focus:border-accent focus:outline-none"
          >
            <option value="system">跟随系统</option>
            <option value="light">浅色</option>
            <option value="dark">深色</option>
          </select>
        </div>

        <div className="flex items-center justify-between rounded-lg border border-[var(--border-prominent)] p-4">
          <div>
            <div className="font-medium">Skill 管理范围</div>
            <div className="text-xs text-tertiary">
              {form.skill_scope_mode === "project"
                ? "项目模式：全局同步已暂停，Skill 可按项目隔离"
                : "全局模式：所有 Skill 来自中心仓库，同步到所有启用 Agent"}
            </div>
          </div>
          <select
            value={form.skill_scope_mode}
            onChange={(e) =>
              setForm({ ...form, skill_scope_mode: e.target.value as AppSettings["skill_scope_mode"] })
            }
            className="rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-2 text-sm text-primary focus:border-accent focus:outline-none"
          >
            <option value="global">全局模式</option>
            <option value="project">项目模式</option>
          </select>
        </div>

        <div className="flex items-center justify-between rounded-lg border border-[var(--border-prominent)] p-4">
          <div>
            <div className="font-medium">远程功能</div>
            <div className="text-xs text-tertiary">
              开启后可连接 GitHub 等 Git 仓库，从「发现」页面浏览并安装远程 Skill。
              <br />
              关闭时应用完全离线运行，不发起任何网络请求（本地优先）。
            </div>
          </div>
          <label className="relative inline-flex cursor-pointer items-center">
            <input
              type="checkbox"
              checked={form.remote_enabled}
              onChange={(e) => setForm({ ...form, remote_enabled: e.target.checked })}
              className="peer sr-only"
            />
            <div className="peer h-6 w-11 rounded-full bg-tertiary after:absolute after:left-[2px] after:top-[2px] after:h-5 after:w-5 after:rounded-full after:bg-primary after:transition-all peer-checked:bg-accent peer-checked:after:translate-x-full" />
          </label>
        </div>

        <div className="rounded-lg border border-[var(--border-prominent)] p-4">
          <div className="font-medium">AI 分析（可选）</div>
          <div className="mt-1 text-xs text-tertiary">
            多模型、Embedding 与 ACP 连接已迁移到左侧「AI 分析（可选）」菜单。
          </div>
        </div>

        <div className="border-t border-[var(--divider)] pt-4">
          <Button variant="primary" size="md" onClick={handleSave} loading={saving} disabled={saving}>
            {saving ? "保存中…" : "保存设置"}
          </Button>
        </div>
      </div>

      <BackupRestoreBlock />
      <VersionControlBlock />
    </div>
  );
}

function VersionControlBlock() {
  const { settings, setSettings } = useAppStore();
  const [snapshots, setSnapshots] = useState<SnapshotInfo[]>([]);
  const [commits, setCommits] = useState<GitCommit[]>([]);
  const [gitInited, setGitInited] = useState(false);
  const [note, setNote] = useState("");
  const [commitMsg, setCommitMsg] = useState("");
  const [creating, setCreating] = useState(false);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [gitting, setGitting] = useState<string | null>(null);
  const [togglingAutoCommit, setTogglingAutoCommit] = useState(false);

  // P2-1: 导入后自动提交开关，立即落盘（不经「保存设置」按钮）。
  const toggleAutoCommit = async (checked: boolean) => {
    if (togglingAutoCommit) return;
    setTogglingAutoCommit(true);
    try {
      const saved = await invoke<AppSettings>("save_settings", {
        newSettings: { ...settings, auto_commit_after_import: checked },
      });
      setSettings(saved);
      showSuccess(checked ? "已开启导入后自动提交" : "已关闭导入后自动提交");
    } catch (err) {
      const m = typeof err === "string" ? err : String(err);
      showError(`保存失败：${m}`);
    } finally {
      setTogglingAutoCommit(false);
    }
  };

  const load = useCallback(async () => {
    try {
      const [s, c] = await Promise.all([
        invoke<SnapshotInfo[]>("list_snapshots"),
        invoke<GitCommit[]>("git_versions"),
      ]);
      setSnapshots(s ?? []);
      setCommits(c ?? []);
      setGitInited(c?.length > 0 || (await invoke<boolean>("git_status_inited").catch(() => false)));
    } catch {
      // ignore
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const createSnapshot = async () => {
    if (creating) return;
    setCreating(true);
    try {
      await invoke<SnapshotInfo>("create_snapshot", { note: note.trim() || undefined });
      setNote("");
      showSuccess("快照已创建");
      await load();
    } catch (err) {
      const m = typeof err === "string" ? err : String(err);
      showError(`创建快照失败：${m}`);
    } finally {
      setCreating(false);
    }
  };

  const restoreSnapshot = async (id: string) => {
    if (restoring) return;
    if (!confirm(`确定要恢复到备份「${id.slice(0, 16)}」吗？当前中心仓库会被先备份。`)) return;
    setRestoring(id);
    try {
      await invoke("restore_snapshot", { id });
      showSuccess("已恢复到选定备份");
      await load();
    } catch (err) {
      const m = typeof err === "string" ? err : String(err);
      showError(`恢复失败：${m}`);
    } finally {
      setRestoring(null);
    }
  };

  const initGit = async () => {
    setGitting("init");
    try {
      await invoke("git_init_repo");
      showSuccess("Git 仓库已初始化");
      setGitInited(true);
      await load();
    } catch (err) {
      const m = typeof err === "string" ? err : String(err);
      showError(`初始化失败：${m}`);
    } finally {
      setGitting(null);
    }
  };

  const commit = async () => {
    if (!commitMsg.trim()) return;
    setGitting("commit");
    try {
      // P2-1: 返回被跳过的内嵌 .git 目录（已写入 .gitignore，未形成 gitlink）。
      const ignored = await invoke<string[]>("git_commit", { message: commitMsg.trim() });
      setCommitMsg("");
      if (ignored && ignored.length > 0) {
        showError(`已跳过含内嵌 .git 的目录（已加入 .gitignore）：${ignored.join("、")}。如需纳入版本管理，请使用 git submodule。`);
      } else {
        showSuccess("已提交");
      }
      await load();
    } catch (err) {
      const m = typeof err === "string" ? err : String(err);
      showError(`提交失败：${m}`);
    } finally {
      setGitting(null);
    }
  };

  const runGitAction = async (action: "push" | "pull") => {
    setGitting(action);
    try {
      await invoke(action === "push" ? "git_push" : "git_pull", {});
      showSuccess(action === "push" ? "已推送" : "已拉取");
      await load();
    } catch (err) {
      const m = typeof err === "string" ? err : String(err);
      showError(`${action === "push" ? "推送" : "拉取"}失败：${m}`);
    } finally {
      setGitting(null);
    }
  };

  return (
    <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary p-6">
      <h2 className="mb-2 text-lg font-semibold">协作与备份</h2>
      <p className="mb-4 text-xs text-tertiary">
        本地备份让你随时回滚整个中心仓库；Git 化后可将 Skill 资产推送到远程仓库或与他人协作。
      </p>

      <div className="mb-6 rounded-lg border border-[var(--border-prominent)] p-4">
        <div className="mb-2 text-sm font-medium">本地备份</div>
        <div className="mb-3 flex gap-2">
          <input
            value={note}
            onChange={(e) => setNote(e.target.value)}
            placeholder="备注（可选）"
            className="flex-1 rounded border border-[var(--border-subtle)] bg-primary px-3 py-1.5 text-sm"
          />
          <Button variant="primary" size="sm" onClick={createSnapshot} loading={creating} disabled={creating}>
            {creating ? "创建中…" : "创建备份"}
          </Button>
        </div>
        {(!Array.isArray(snapshots) || snapshots.length === 0) ? (
          <div className="text-xs text-tertiary">暂无备份</div>
        ) : (
          <div className="space-y-1">
            {snapshots.map((s) => (
              <div key={s.id} className="flex items-center justify-between rounded bg-primary px-3 py-2 text-sm">
                <div>
                  <div className="font-medium">{s.note || "未命名备份"}</div>
                  <div className="text-xs text-tertiary">
                    {new Date(s.created_at * 1000).toLocaleString()} · {s.skill_count} 个 Skill
                  </div>
                </div>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => restoreSnapshot(s.id)}
                  loading={restoring === s.id}
                  disabled={restoring !== null}
                >
                  恢复
                </Button>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="rounded-lg border border-[var(--border-prominent)] p-4">
        <div className="mb-2 flex items-center gap-2 text-sm font-medium">
          <GitBranch className="h-4 w-4" />
          Git 版本控制
          {gitInited && <span className="text-xs font-normal text-green-400">已初始化</span>}
        </div>
        {!gitInited ? (
          <div className="space-y-2">
            <p className="text-xs text-tertiary">
              中心仓库当前不是 Git 仓库。初始化后即可提交版本、推送到远程。
            </p>
            <Button variant="primary" size="sm" onClick={initGit} loading={gitting === "init"} disabled={gitting !== null}>
              {gitting === "init" ? "初始化中…" : "初始化 Git"}
            </Button>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex items-center justify-between rounded border border-[var(--border-subtle)] px-3 py-2">
              <div className="text-xs text-secondary">
                导入后自动提交
                <div className="text-[11px] text-tertiary">
                  导入 Skill 后自动 git 提交，提交信息包含导入清单；含内嵌 .git 的目录会被跳过并写入 .gitignore
                </div>
              </div>
              <input
                type="checkbox"
                aria-label="导入后自动提交"
                checked={settings.auto_commit_after_import}
                disabled={togglingAutoCommit}
                onChange={(e) => toggleAutoCommit(e.target.checked)}
                className="h-4 w-4 accent-accent disabled:opacity-50"
              />
            </div>
            <div className="flex gap-2">
              <input
                value={commitMsg}
                onChange={(e) => setCommitMsg(e.target.value)}
                placeholder="提交信息，如：更新 frontend-design Skill"
                className="flex-1 rounded border border-[var(--border-subtle)] bg-primary px-3 py-1.5 text-sm"
                onKeyDown={(e) => e.key === "Enter" && commit()}
              />
              <Button variant="primary" size="sm" onClick={commit} loading={gitting === "commit"} disabled={gitting !== null || !commitMsg.trim()}>
                提交
              </Button>
            </div>
            <div className="flex gap-2">
              <Button variant="secondary" size="sm" onClick={() => runGitAction("push")} loading={gitting === "push"} disabled={gitting !== null}>
                推送
              </Button>
              <Button variant="secondary" size="sm" onClick={() => runGitAction("pull")} loading={gitting === "pull"} disabled={gitting !== null}>
                拉取
              </Button>
            </div>
            {(!Array.isArray(commits) || commits.length === 0) ? (
              <div className="text-xs text-tertiary">暂无提交记录</div>
            ) : (
              <div className="max-h-48 space-y-1 overflow-auto">
                {commits.map((c) => (
                  <div key={c.sha} className="rounded bg-primary px-3 py-2 text-sm">
                    <div className="flex items-center justify-between">
                      <span className="font-medium">{c.message}</span>
                      <span className="text-xs text-tertiary">{c.sha.slice(0, 7)}</span>
                    </div>
                    <div className="text-xs text-tertiary">
                      {c.author} · {new Date(c.timestamp * 1000).toLocaleString()}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function BackupRestoreBlock() {
  const [backing, setBacking] = useState(false);
  const [restoring, setRestoring] = useState(false);

  const handleBackup = async () => {
    if (backing) return;
    setBacking(true);
    try {
      const today = new Date().toISOString().slice(0, 10);
      const path = await save({
        defaultPath: `skillmint-backup-${today}.zip`,
        filters: [{ name: "Zip", extensions: ["zip"] }],
      });
      if (!path) return;
      await invoke("backup_center_repo", { path });
      showSuccess(`已备份到：${path}`);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`备份失败：${msg}`);
    } finally {
      setBacking(false);
    }
  };

  const handleRestore = async () => {
    if (restoring) return;
    setRestoring(true);
    try {
      const path = await open({
        multiple: false,
        filters: [{ name: "Zip", extensions: ["zip"] }],
      });
      if (!path || typeof path !== "string") return;
      const result = await invoke<RestoreSummary>("restore_center_repo", { path });
      const { imported, skipped } = result;
      if (imported.length === 0 && skipped.length === 0) {
        showInfo("备份文件中未发现可导入的 Skill");
      } else if (skipped.length === 0) {
        showSuccess(`恢复完成，导入 ${imported.length} 个 Skill`);
      } else {
        showInfo(
          `导入 ${imported.length} 个，跳过 ${skipped.length} 个已存在的 Skill（${skipped.join("、") || "无"}）`
        );
      }
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`恢复失败：${msg}`);
    } finally {
      setRestoring(false);
    }
  };

  return (
    <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary p-6">
      <h2 className="mb-2 text-lg font-semibold">备份与恢复</h2>
      <p className="mb-4 text-xs text-tertiary">
        把中心仓库打包为 zip 便于备份或迁移到其他设备。恢复时采用智能合并：已存在的同名 Skill 不会被覆盖。
      </p>
      <div className="flex flex-wrap gap-3">
        <Button variant="primary" size="sm" onClick={handleBackup} loading={backing} disabled={backing}>
          {backing ? "备份中…" : "备份 Center Repo"}
        </Button>
        <Button variant="secondary" size="sm" onClick={handleRestore} loading={restoring} disabled={restoring}>
          {restoring ? "恢复中…" : "从 zip 恢复"}
        </Button>
      </div>
    </div>
  );
}
