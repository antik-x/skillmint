import { BarChart, Brain, Clock, Database, Info, SlidersHorizontal } from "lucide-react";
import { useEffect } from "react";
import { useAppStore, type SettingsSubTab } from "../stores/appStore";
import PreferencesPanel from "../components/PreferencesPanel";
import DataCollectionPanel from "../components/DataCollectionPanel";
import DataManagementPanel from "../components/DataManagementPanel";
import ScheduledTasks from "./ScheduledTasks";
import AboutPanel from "../components/AboutPanel";
import AiSettingsPanel from "../components/AiSettingsPanel";
import { cn } from "../components/ui/utils";

interface SubNavItem {
  id: SettingsSubTab;
  label: string;
  description: string;
  icon: React.ElementType;
}

const VALID_TABS: SettingsSubTab[] = [
  "preferences",
  "dataCollection",
  "scheduledTasks",
  "dataManagement",
  "aiAnalysis",
  "about",
];

function parseTabFromUrl(): SettingsSubTab | null {
  try {
    const params = new URLSearchParams(window.location.search);
    const tab = params.get("tab") as SettingsSubTab | null;
    if (tab && VALID_TABS.includes(tab)) return tab;
  } catch {
    // ignore
  }
  return null;
}

function parseTabFromState(): SettingsSubTab | null {
  try {
    const state = window.history?.state;
    const tab = state?.settingsTab as SettingsSubTab | undefined;
    if (tab && VALID_TABS.includes(tab)) return tab;
  } catch {
    // ignore
  }
  return null;
}

const subNavItems: SubNavItem[] = [
  {
    id: "preferences",
    label: "偏好设置",
    description: "中心仓库、同步模式、开机自启、备份与 Git",
    icon: SlidersHorizontal,
  },
  {
    id: "dataCollection",
    label: "数据采集",
    description: "配置 Agent 使用数据的采集源与规则",
    icon: BarChart,
  },
  {
    id: "scheduledTasks",
    label: "定时任务",
    description: "管理周期性后台同步与扫描任务",
    icon: Clock,
  },
  {
    id: "dataManagement",
    label: "数据管理",
    description: "本地数据库路径、数据字典与导出",
    icon: Database,
  },
  {
    id: "aiAnalysis",
    label: "本地 Agent 分析（ACP）",
    description: "配置本地 Agent、云端模型与隐私边界",
    icon: Brain,
  },
  {
    id: "about",
    label: "关于",
    description: "应用版本、设备 ID 与运行模式",
    icon: Info,
  },
];

export default function Settings() {
  const settingsSubTab = useAppStore((state) => state.settingsSubTab);
  const setSettingsSubTab = useAppStore((state) => state.setSettingsSubTab);

  // SPEC-F4 T6: support deep-linking to a settings sub-tab via ?tab= or nav state.
  useEffect(() => {
    const tab = parseTabFromUrl() ?? parseTabFromState();
    if (tab) {
      setSettingsSubTab(tab);
      // Clean the URL/state so a refresh lands back at the default sub-tab.
      try {
        const url = new URL(window.location.href);
        url.searchParams.delete("tab");
        window.history.replaceState({ ...window.history.state, settingsTab: null }, "", url.toString());
      } catch {
        // ignore
      }
    }
  }, [setSettingsSubTab]);

  return (
    <div className="flex h-full overflow-hidden">
      <aside className="w-56 flex-shrink-0 border-r border-[var(--border-subtle)] bg-secondary/30 p-4">
        <h2 className="mb-1 px-3 text-sm font-semibold text-primary">设置</h2>
        <p className="mb-4 px-3 text-xs text-tertiary">系统级配置与自动化</p>
        <nav className="space-y-1">
          {subNavItems.map((item) => {
            const Icon = item.icon;
            const isActive = settingsSubTab === item.id;
            return (
              <button
                key={item.id}
                onClick={() => setSettingsSubTab(item.id)}
                className={cn(
                  "flex w-full flex-col items-start rounded-lg px-3 py-2.5 text-left text-sm transition-colors",
                  isActive
                    ? "bg-accent/10 text-accent"
                    : "text-secondary hover:bg-tertiary/50 hover:text-primary"
                )}
              >
                <span className="flex items-center gap-2 font-medium">
                  <Icon className="h-4 w-4" />
                  {item.label}
                </span>
                <span className="mt-0.5 pl-6 text-xs text-tertiary">{item.description}</span>
              </button>
            );
          })}
        </nav>
      </aside>

      <main className="flex-1 overflow-auto p-8">
        <h1 className="mb-6 text-2xl font-bold text-primary">
          {subNavItems.find((i) => i.id === settingsSubTab)?.label}
        </h1>
        {settingsSubTab === "preferences" && <PreferencesPanel />}
        {settingsSubTab === "dataCollection" && <DataCollectionPanel title="使用数据采集" />}
        {settingsSubTab === "scheduledTasks" && <ScheduledTasks embedded />}
        {settingsSubTab === "dataManagement" && <DataManagementPanel />}
        {settingsSubTab === "aiAnalysis" && <AiSettingsPanel />}
        {settingsSubTab === "about" && <AboutPanel />}
      </main>
    </div>
  );
}
