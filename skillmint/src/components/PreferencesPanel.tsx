import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../stores/appStore";
import { showError, showSuccess } from "../stores/toastStore";
import { Button } from "./ui/Button";
import { getNpxEnv, type NodeEnvInfo } from "../lib/npxskills";
import type { AppSettings } from "../types";

export default function PreferencesPanel() {
  const { settings, setSettings } = useAppStore();
  const [form, setForm] = useState<AppSettings>(settings);
  const [saving, setSaving] = useState(false);
  const [nodeEnv, setNodeEnv] = useState<NodeEnvInfo | null>(null);

  useEffect(() => {
    setForm(settings);
  }, [settings]);

  const refreshNodeEnv = useCallback(() => {
    getNpxEnv()
      .then(setNodeEnv)
      .catch(() => setNodeEnv(null));
  }, []);

  useEffect(() => {
    refreshNodeEnv();
  }, [refreshNodeEnv]);

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
          <div className="font-medium">Node 运行时（npx skills 依赖，需 ≥ v22.20）</div>
          {nodeEnv ? (
            nodeEnv.problem ? (
              <div className="mt-1 text-xs text-danger">{nodeEnv.problem}</div>
            ) : (
              <div className="mt-1 text-xs text-tertiary">
                node {nodeEnv.node_version} · {nodeEnv.bin_dir}
              </div>
            )
          ) : (
            <div className="mt-1 text-xs text-tertiary">检测中…</div>
          )}
        </div>

        <div>
          <label className="mb-2 block text-sm text-secondary">skills CLI 版本（npm 包规格）</label>
          <input
            type="text"
            value={form.npx_package ?? "skills@latest"}
            onChange={(e) => setForm({ ...form, npx_package: e.target.value })}
            placeholder="skills@latest 或固定版本 skills@1.5.23"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-4 py-2 text-primary focus:border-accent focus:outline-none"
          />
          <p className="mt-1 text-xs text-tertiary">app 的安装/更新/卸载都通过 `npx -y &lt;此规格&gt;` 调用真实 CLI。</p>
        </div>

        <div>
          <label className="mb-2 block text-sm text-secondary">skills 搜索源镜像（SKILLS_API_URL）</label>
          <input
            type="text"
            value={form.skills_api_url ?? ""}
            onChange={(e) => setForm({ ...form, skills_api_url: e.target.value })}
            placeholder="留空使用 https://skills.sh；国内可填可达镜像"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-4 py-2 text-primary focus:border-accent focus:outline-none"
          />
          <p className="mt-1 text-xs text-tertiary">
            影响「发现」页搜索。安装下载走 git，受下方代理影响；api.github.com 不可达时 CLI 会自动回退 git clone。
          </p>
        </div>

        <div>
          <label className="mb-2 block text-sm text-secondary">网络代理（注入 HTTPS_PROXY/HTTP_PROXY/ALL_PROXY）</label>
          <input
            type="text"
            value={form.proxy_env ?? ""}
            onChange={(e) => setForm({ ...form, proxy_env: e.target.value })}
            placeholder="例如 http://127.0.0.1:7890"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-4 py-2 text-primary focus:border-accent focus:outline-none"
          />
        </div>

        <div>
          <label className="mb-2 block text-sm text-secondary">node 路径覆盖（可选）</label>
          <input
            type="text"
            value={form.node_path_override ?? ""}
            onChange={(e) => setForm({ ...form, node_path_override: e.target.value })}
            placeholder="GUI 启动时 PATH 受限；可填 node bin 目录，如 ~/.nvm/versions/node/v22.20.0/bin"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-4 py-2 text-primary focus:border-accent focus:outline-none"
          />
        </div>

        <div className="flex items-center justify-between rounded-lg border border-[var(--border-prominent)] p-4">
          <div>
            <div className="font-medium">允许 npx skills 遥测</div>
            <div className="text-xs text-tertiary">默认关闭（注入 DISABLE_TELEMETRY=1），符合本地优先理念</div>
          </div>
          <input
            type="checkbox"
            checked={!form.disable_telemetry}
            onChange={(e) => setForm({ ...form, disable_telemetry: !e.target.checked })}
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

    </div>
  );
}

