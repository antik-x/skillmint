import { memo, useEffect, useState } from "react";
import {
  CalendarDays,
  FileText,
  FolderGit,
  Globe,
  Inbox,
  Library,
  Bot,
  Puzzle,
  Settings,
  Share2,
  Sparkles,
  TrendingUp,
  type LucideIcon,
} from "lucide-react";
import { invoke } from "../lib/invoke";
import { useAppStore, type AppTab } from "../stores/appStore";
import TaskCenter from "./TaskCenter";
import { cn } from "./ui/utils";

interface MenuItem {
  id: AppTab;
  label: string;
  icon: LucideIcon;
  badge?: number;
}

interface Group {
  eyebrow?: string;
  items: MenuItem[];
}

const groups: Group[] = [
  {
    items: [
      { id: "today", label: "今天", icon: Sparkles },
      { id: "inbox", label: "收件箱", icon: Inbox },
      { id: "growthAssets", label: "成长资产", icon: TrendingUp },
    ],
  },
  {
    eyebrow: "资产库",
    items: [
      { id: "skillLibrary", label: "Skill 库", icon: Library },
      { id: "knowledgeGraph", label: "知识图谱", icon: Share2 },
      { id: "discover", label: "发现", icon: Globe },
    ],
  },
  {
    eyebrow: "工作区",
    items: [
      { id: "projects", label: "项目", icon: FolderGit },
      { id: "agents", label: "Agent", icon: Bot },
    ],
  },
  {
    eyebrow: "档案",
    items: [
      { id: "usage", label: "洞察档案", icon: Puzzle },
      { id: "weeklyReport", label: "周报", icon: CalendarDays },
      { id: "dailySummaries", label: "每日摘要", icon: FileText },
    ],
  },
  {
    eyebrow: "系统",
    items: [{ id: "settings", label: "设置", icon: Settings }],
  },
];

function SidebarComponent() {
  const activeTab = useAppStore((state) => state.activeTab);
  const skillLibrarySubTab = useAppStore((state) => state.skillLibrarySubTab);
  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const sidebarVisible = useAppStore((state) => state.sidebarVisible);
  const inboxRefreshKey = useAppStore((state) => state.inboxRefreshKey);
  const [pendingCount, setPendingCount] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<{ status: string }[]>("list_discoveries", { status: "pending" })
      .then((rows) => {
        if (!cancelled) setPendingCount(rows.filter((r) => r.status === "pending").length);
      })
      .catch(() => {
        if (!cancelled) setPendingCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, [activeTab, inboxRefreshKey]);

  if (!sidebarVisible) return null;

  return (
    <aside className="surface-glass flex w-56 flex-col border-r">
      <div className="flex items-center gap-3 px-5 py-4">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-accent shadow-sm shadow-accent/20">
          <span className="text-sm font-bold text-white">S</span>
        </div>
        <span className="text-base font-semibold tracking-tight text-primary">
          SkillMint
        </span>
      </div>

      <nav className="flex-1 space-y-5 px-3 py-2">
        {groups.map((group, idx) => (
          <div key={group.eyebrow ?? `group-${idx}`}>
            {group.eyebrow && (
              <div className="mb-1.5 px-3 text-2xs font-semibold uppercase tracking-wider text-tertiary font-mono">
                {group.eyebrow}
              </div>
            )}
            <div className="space-y-1">
              {group.items.map((item) => {
                const Icon = item.icon;
                const isActive =
                  activeTab === item.id ||
                  (item.id === "knowledgeGraph" && activeTab === "skillLibrary" && skillLibrarySubTab === "graph") ||
                  (item.id === "discover" && activeTab === "skillLibrary" && skillLibrarySubTab === "discover");
                const showAmberBadge = item.id === "inbox" && typeof pendingCount === "number" && pendingCount > 0;
                return (
                  <button
                    key={item.id}
                    onClick={() => setActiveTab(item.id)}
                    className={cn(
                      "group relative flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm font-medium transition-all duration-200",
                      isActive
                        ? "bg-accent-dim text-primary"
                        : "text-secondary hover:bg-tertiary/60 hover:text-primary"
                    )}
                  >
                    <Icon
                      className={cn(
                        "h-[18px] w-[18px] shrink-0 transition-colors",
                        isActive ? "text-accent" : "text-tertiary group-hover:text-secondary"
                      )}
                    />
                    <span className="flex-1 truncate">{item.label}</span>
                    {showAmberBadge && (
                      <span className="ml-auto flex h-5 min-w-5 items-center justify-center rounded-full bg-amber-dim px-1.5 text-2xs font-medium text-amber font-mono">
                        {pendingCount > 99 ? "99+" : pendingCount}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </nav>

      <div className="border-t border-[var(--border-subtle)] px-3 py-3">
        <TaskCenter />
      </div>
    </aside>
  );
}

export default memo(SidebarComponent);
