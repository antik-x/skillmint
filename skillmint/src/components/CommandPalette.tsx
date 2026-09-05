import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "../lib/invoke";
import { rebuildSkillIndex } from "../lib/npxskills";
import {
  BarChart3,
  Bot,
  CalendarDays,
  Copy,
  FileText,
  FolderGit,
  FolderOpen,
  Gauge,
  Globe,
  Inbox,
  Keyboard,
  Library,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Sparkles,
  Terminal,
  TrendingUp,
} from "lucide-react";
import { useAppStore, type AppTab, type SkillLibrarySubTab, type SettingsSubTab } from "../stores/appStore";
import { useCollectionStore } from "../stores/collectionStore";
import { showError, showInfo, showSuccess } from "../stores/toastStore";
import { useHotkeyScope } from "../hooks/useHotkeys";
import { useListNavigation } from "../hooks/useListNavigation";
import type { SearchResult } from "../types";
import { Dialog } from "./ui/Dialog";
import { Input } from "./ui/Input";
import { cn } from "./ui/utils";

type CommandItemType = "page" | "skill" | "action";

interface CommandItem {
  id: string;
  type: CommandItemType;
  title: string;
  subtitle?: string;
  icon: React.ElementType;
  keywords?: string;
  shortcut?: string;
  action: () => void;
}

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  onNewSkill?: () => void;
}

const STATIC_PAGES: Omit<CommandItem, "id" | "action">[] = [
  { type: "page", title: "今天", icon: Sparkles, keywords: "today" },
  { type: "page", title: "收件箱", icon: Inbox, keywords: "inbox" },
  { type: "page", title: "Skill 库", subtitle: "技能、技能集、知识图谱、发现", icon: Library, keywords: "skill skills" },
  { type: "page", title: "成长资产", icon: TrendingUp, keywords: "growth assets" },
  { type: "page", title: "洞察", subtitle: "洞察档案", icon: BarChart3, keywords: "usage insights" },
  { type: "page", title: "Agent", icon: Bot, keywords: "agents" },
  { type: "page", title: "项目", icon: FolderGit, keywords: "projects" },
  { type: "page", title: "工作记忆", subtitle: "周报 · 每日摘要", icon: CalendarDays, keywords: "work memory weekly daily report 周报 摘要" },
  { type: "page", title: "设置", icon: Settings, keywords: "settings" },
];

function useCommandItems(
  query: string,
  onNewSkill: () => void,
  onOpenShortcuts: () => void,
  onClose: () => void
): { items: CommandItem[]; loading: boolean } {
  const skills = useAppStore((state) => state.skills) ?? [];
  const selectedSkillId = useAppStore((state) => state.selectedSkillId);
  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const setSkillLibrarySubTab = useAppStore((state) => state.setSkillLibrarySubTab);
  const setSettingsSubTab = useAppStore((state) => state.setSettingsSubTab);
  const navigateToSkill = useAppStore((state) => state.navigateToSkill);
  const loadData = useAppStore((state) => state.loadData);
  const startCollect = useCollectionStore((state) => state.startCollect);
  const [remoteResults, setRemoteResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);

  const navigateTo = useCallback(
    (tab: AppTab, subTab?: SkillLibrarySubTab | SettingsSubTab) => {
      setActiveTab(tab);
      if (tab === "skillLibrary" && subTab) {
        setSkillLibrarySubTab(subTab as SkillLibrarySubTab);
      }
      if (tab === "settings" && subTab) {
        setSettingsSubTab(subTab as SettingsSubTab);
      }
      onClose();
    },
    [onClose, setActiveTab, setSettingsSubTab, setSkillLibrarySubTab]
  );

  // Debounced remote skill search (200ms).
  useEffect(() => {
    if (!query.trim()) {
      setRemoteResults([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    const handle = setTimeout(async () => {
      try {
        const results = await invoke<SearchResult[]>("search_all", { query });
        setRemoteResults(results.filter((r) => r.origin === "remote" && !r.installed_locally));
      } catch (err) {
        // Silent fail for remote search; local skills still available.
        setRemoteResults([]);
      } finally {
        setLoading(false);
      }
    }, 200);
    return () => clearTimeout(handle);
  }, [query]);

  const pageItems: CommandItem[] = useMemo(
    () =>
      STATIC_PAGES.map((page, index) => ({
        ...page,
        id: `page-${index}`,
        action: () => {
          const title = page.title;
          if (title === "今天") navigateTo("today");
          else if (title === "收件箱") navigateTo("inbox");
          else if (title === "成长资产") navigateTo("growthAssets");
          else if (title === "Skill 库") navigateTo("skillLibrary", "skills");
          else if (title === "洞察") navigateTo("usage");
          else if (title === "Agent") navigateTo("agents");
          else if (title === "项目") navigateTo("projects");
          else if (title === "工作记忆") navigateTo("workMemory");
          else if (title === "设置") navigateTo("settings", "preferences");
        },
      })),
    [navigateTo]
  );

  const localSkillItems: CommandItem[] = useMemo(
    () =>
      skills.map((skill) => ({
        id: `skill-local-${skill.id}`,
        type: "skill",
        title: skill.name,
        subtitle: "本地 Skill",
        icon: Library,
        keywords: skill.name,
        action: () => {
          navigateToSkill(skill.id);
          onClose();
        },
      })),
    [navigateToSkill, onClose, skills]
  );

  const remoteSkillItems: CommandItem[] = useMemo(
    () =>
      remoteResults.map((result, index) => ({
        id: `skill-remote-${result.source_id}-${result.skill_name}-${index}`,
        type: "skill",
        title: result.skill_name,
        subtitle: result.source_name ? `远程 · ${result.source_name}` : "远程 Skill",
        icon: Globe,
        keywords: `${result.skill_name} ${result.description ?? ""}`,
        action: () => {
          // Best-effort: navigate to discover and let the user install from there.
          setActiveTab("skillLibrary");
          setSkillLibrarySubTab("discover");
          onClose();
        },
      })),
    [remoteResults, setActiveTab, setSkillLibrarySubTab, onClose]
  );

  const actionItems: CommandItem[] = useMemo(
    () => [
      {
        id: "action-sync-all",
        type: "action",
        title: "刷新技能索引",
        subtitle: "从 npx locks / agent 目录 / 私有 Hub 重建索引",
        icon: RefreshCw,
        shortcut: "mod+shift+s",
        action: async () => {
          try {
            const summary = await rebuildSkillIndex(null);
            await loadData();
            showSuccess(`索引已刷新：${summary.total} 项`);
          } catch (err) {
            const msg = typeof err === "string" ? err : String(err);
            showError(`刷新失败：${msg}`);
          } finally {
            onClose();
          }
        },
      },
      {
        id: "action-collect-now",
        type: "action",
        title: "立即采集",
        subtitle: "启动数据采集任务",
        icon: Gauge,
        action: async () => {
          await startCollect();
          onClose();
        },
      },
      {
        id: "action-new-skill",
        type: "action",
        title: "新建 Skill",
        subtitle: "创建新的本地 Skill",
        icon: Plus,
        shortcut: "mod+n",
        action: () => {
          onNewSkill();
          onClose();
        },
      },
      {
        id: "action-open-settings",
        type: "action",
        title: "打开设置",
        subtitle: "偏好设置",
        icon: Settings,
        shortcut: "mod+,",
        action: () => navigateTo("settings", "preferences"),
      },
      {
        id: "action-open-daily-summaries",
        type: "action",
        title: "打开每日摘要",
        subtitle: "查看历史日报",
        icon: FileText,
        action: () => navigateTo("dailySummaries"),
      },
      {
        id: "action-copy-skill-path",
        type: "action",
        title: "复制 Skill 路径",
        subtitle: "复制当前聚焦 Skill 的绝对路径",
        icon: Copy,
        action: async () => {
          const skill = skills.find((s) => s.id === selectedSkillId) ?? skills[0];
          if (!skill) {
            showInfo("暂无 Skill 可复制路径");
            onClose();
            return;
          }
          try {
            await navigator.clipboard.writeText(skill.repo_path);
            showSuccess("路径已复制");
          } catch {
            showError("复制失败");
          }
          onClose();
        },
      },
      {
        id: "action-reveal-skill-in-finder",
        type: "action",
        title: "在 Finder 中显示当前 Skill 目录",
        subtitle: "打开当前聚焦 Skill 的目录",
        icon: FolderOpen,
        action: async () => {
          const skill = skills.find((s) => s.id === selectedSkillId) ?? skills[0];
          if (!skill) {
            showInfo("暂无 Skill 可显示");
            onClose();
            return;
          }
          const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
          if (!isTauri) {
            showInfo("需在桌面 App 中使用");
            onClose();
            return;
          }
          try {
            const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
            await revealItemInDir(skill.repo_path);
            showSuccess("已在 Finder 中显示");
          } catch {
            showError("无法在 Finder 中显示");
          }
          onClose();
        },
      },
      {
        id: "action-open-skill-terminal",
        type: "action",
        title: "在终端中打开 Skill 目录",
        subtitle: "打开当前聚焦 Skill 的目录",
        icon: Terminal,
        action: async () => {
          const skill = skills.find((s) => s.id === selectedSkillId) ?? skills[0];
          if (!skill) {
            showInfo("暂无 Skill 可打开");
            onClose();
            return;
          }
          try {
            await invoke("open_path_in_terminal", { path: skill.repo_path });
            showSuccess("已在终端中打开");
          } catch {
            showError("打开终端失败");
          }
          onClose();
        },
      },
      {
        id: "action-refresh",
        type: "action",
        title: "刷新当前页",
        subtitle: "重新加载数据",
        icon: RefreshCw,
        shortcut: "mod+r",
        action: async () => {
          try {
            await loadData();
            showSuccess("已刷新");
          } catch (err) {
            const msg = typeof err === "string" ? err : String(err);
            showError(`刷新失败：${msg}`);
          } finally {
            onClose();
          }
        },
      },
      {
        id: "action-keyboard-shortcuts",
        type: "action",
        title: "键盘快捷键",
        subtitle: "查看所有快捷键",
        icon: Keyboard,
        shortcut: "mod+/",
        action: () => {
          onOpenShortcuts();
          onClose();
        },
      },
    ],
    [loadData, navigateTo, onClose, onNewSkill, onOpenShortcuts, startCollect, skills, selectedSkillId]
  );

  const allItems = useMemo(
    () => [...pageItems, ...localSkillItems, ...remoteSkillItems, ...actionItems],
    [actionItems, localSkillItems, pageItems, remoteSkillItems]
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return allItems;
    return allItems
      .map((item) => {
        const haystack = `${item.title} ${item.subtitle ?? ""} ${item.keywords ?? ""}`.toLowerCase();
        let score = 0;
        if (item.title.toLowerCase().startsWith(q)) score += 100;
        else if (haystack.includes(q)) score += 50;
        else {
          const words = q.split(/\s+/).filter(Boolean);
          const matches = words.filter((w) => haystack.includes(w)).length;
          if (matches === 0) return null;
          score += matches * 20;
        }
        // Boost pages and actions slightly over skills for empty-ish queries.
        if (item.type === "page") score += 5;
        if (item.type === "action") score += 3;
        return { item, score };
      })
      .filter((x): x is { item: CommandItem; score: number } => x !== null)
      .sort((a, b) => b.score - a.score)
      .map((x) => x.item);
  }, [allItems, query]);

  return { items: filtered, loading };
}

export default function CommandPalette({ open, onClose, onNewSkill }: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [showShortcuts, setShowShortcuts] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useHotkeyScope(open ? "command-palette" : undefined);

  const handleNewSkill = useCallback(() => {
    onNewSkill?.();
  }, [onNewSkill]);

  const handleOpenShortcuts = useCallback(() => {
    setShowShortcuts(true);
  }, []);

  const { items, loading } = useCommandItems(
    query,
    handleNewSkill,
    handleOpenShortcuts,
    onClose
  );

  const handleSelect = useCallback(
    (_item: CommandItem, index: number) => {
      items[index]?.action?.();
    },
    [items]
  );

  const { activeIndex, setActiveIndex, getItemProps, listProps, handleKeyDown } = useListNavigation({
    items,
    onSelect: handleSelect,
    listRef,
    initialIndex: items.length > 0 ? 0 : -1,
  });

  // The command palette keeps focus in the input, so we handle navigation at
  // the dialog-content level. We still expose listbox aria attributes on the list.
  const { onKeyDown: _listOnKeyDown, ...listAriaProps } = listProps;

  // Auto-focus input when opened.
  useEffect(() => {
    if (open) {
      setQuery("");
      setShowShortcuts(false);
      const timer = setTimeout(() => inputRef.current?.focus(), 10);
      return () => clearTimeout(timer);
    }
  }, [open]);

  // Reset active index when results change.
  useEffect(() => {
    setActiveIndex(items.length > 0 ? 0 : -1);
  }, [items.length, setActiveIndex]);

  const grouped = useMemo(() => {
    const groups: { type: CommandItemType; label: string; items: CommandItem[] }[] = [
      { type: "page", label: "页面", items: [] },
      { type: "skill", label: "Skill", items: [] },
      { type: "action", label: "动作", items: [] },
    ];
    for (const item of items) {
      groups.find((g) => g.type === item.type)?.items.push(item);
    }
    return groups.filter((g) => g.items.length > 0);
  }, [items]);

  return (
    <>
      <Dialog
        open={open}
        onClose={onClose}
        disableEscape={false}
        className="max-w-2xl p-0"
      >
        <div className="flex flex-col" onKeyDown={handleKeyDown}>
          <div className="flex items-center gap-3 border-b border-[var(--border-subtle)] px-4 py-3">
            <Search className="h-5 w-5 text-tertiary" />
            <Input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索页面、Skill 或动作…"
              className="border-none bg-transparent px-0 py-0 shadow-none focus:ring-0"
            />
            {loading && <RefreshCw className="h-4 w-4 animate-spin text-tertiary" />}
          </div>

          <div
            ref={listRef}
            {...listAriaProps}
            className="max-h-[60vh] min-h-[120px] overflow-auto p-2 outline-none"
          >
            {items.length === 0 ? (
              <div className="py-8 text-center text-sm text-tertiary">
                {query.trim() ? "未找到匹配结果" : "输入关键词开始搜索"}
              </div>
            ) : (
              grouped.map((group) => (
                <div key={group.type} className="mb-2">
                  <div className="px-3 py-1 text-xs font-semibold uppercase tracking-wider text-tertiary">
                    {group.label}
                  </div>
                  <div className="space-y-0.5">
                    {group.items.map((item) => {
                      const globalIndex = items.indexOf(item);
                      const props = getItemProps(globalIndex);
                      const Icon = item.icon;
                      return (
                        <button
                          key={item.id}
                          {...props}
                          onClick={() => item.action()}
                          className={cn(
                            "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors",
                            globalIndex === activeIndex
                              ? "bg-accent text-primary"
                              : "text-secondary hover:bg-tertiary/50 hover:text-primary"
                          )}
                        >
                          <Icon
                            className={cn(
                              "h-4 w-4 shrink-0",
                              globalIndex === activeIndex ? "text-primary" : "text-tertiary"
                            )}
                          />
                          <div className="min-w-0 flex-1">
                            <div className="truncate text-sm font-medium">{item.title}</div>
                            {item.subtitle && (
                              <div
                                className={cn(
                                  "truncate text-xs",
                                  globalIndex === activeIndex ? "text-primary/80" : "text-tertiary"
                                )}
                              >
                                {item.subtitle}
                              </div>
                            )}
                          </div>
                          {item.shortcut && (
                            <kbd
                              className={cn(
                                "rounded border px-1.5 py-0.5 text-2xs font-mono",
                                globalIndex === activeIndex
                                  ? "border-primary/40 text-primary"
                                  : "border-[var(--border-subtle)] text-tertiary"
                              )}
                            >
                              {item.shortcut.replace("mod", isMac() ? "⌘" : "Ctrl")}
                            </kbd>
                          )}
                        </button>
                      );
                    })}
                  </div>
                </div>
              ))
            )}
          </div>

          <div className="flex items-center justify-between border-t border-[var(--border-subtle)] px-4 py-2 text-xs text-tertiary">
            <div className="flex gap-3">
              <span>↑↓ 选择</span>
              <span>↵ 执行</span>
              <span>Esc 关闭</span>
            </div>
            <span>{items.length} 个结果</span>
          </div>
        </div>
      </Dialog>

      <ShortcutsHelpDialog open={showShortcuts} onClose={() => setShowShortcuts(false)} />
    </>
  );
}

function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return navigator.platform.toLowerCase().includes("mac");
}

function ShortcutsHelpDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const shortcuts = [
    { combo: "mod+k", title: "命令面板" },
    { combo: "mod+s", title: "保存当前 Skill（编辑器内）" },
    { combo: "mod+shift+s", title: "同步全部" },
    { combo: "mod+r", title: "刷新当前页" },
    { combo: "mod+,", title: "打开设置" },
    { combo: "mod+n", title: "新建 Skill" },
    { combo: "mod+b", title: "切换侧边栏" },
    { combo: "mod+/", title: "键盘快捷键帮助" },
  ];

  return (
    <Dialog open={open} onClose={onClose} title="键盘快捷键">
      <div className="space-y-2">
        {shortcuts.map((s) => (
          <div key={s.combo} className="flex items-center justify-between py-1">
            <span className="text-sm text-secondary">{s.title}</span>
            <kbd className="rounded border border-[var(--border-subtle)] px-2 py-0.5 text-xs font-mono text-primary">
              {s.combo.replace("mod", isMac() ? "⌘" : "Ctrl")}
            </kbd>
          </div>
        ))}
      </div>
    </Dialog>
  );
}
