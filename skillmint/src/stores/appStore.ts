import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Skill, Agent, AppSettings } from "../types";

/** 重构后的顶层入口，符合 SPEC-I4 信息架构（E1/E2/E3 对齐 docs/product-epics.md）。 */
export type AppTab =
  | "today"
  | "inbox"
  | "growthAssets"
  | "skillLibrary"
  | "knowledgeGraph"
  | "discover"
  | "agents"
  | "projects"
  | "usage"
  | "weeklyReport"
  | "settings"
  | "dashboard"
  | "dailySummaries"
  | "workMemory";

export type SkillLibrarySubTab = "skills" | "bundles" | "graph" | "discover";
export type WorkMemorySubTab = "weekly" | "daily";
export type SettingsSubTab = "preferences" | "dataCollection" | "scheduledTasks" | "dataManagement" | "aiAnalysis" | "about";
export type UsageSubTab = "overview" | "profile" | "sediment";

/** 旧版 tab key，用于状态迁移或外部持久化值的兼容映射。 */
type LegacyTab =
  | "dashboard"
  | "dailySummaries"
  | "skills"
  | "agents"
  | "projects"
  | "usage"
  | "dataCollection"
  | "scheduledTasks"
  | "graph"
  | "discover"
  | "conflicts"
  | "settings";

interface AppState {
  initialized: boolean;
  activeTab: AppTab;
  skillLibrarySubTab: SkillLibrarySubTab;
  workMemorySubTab: WorkMemorySubTab;
  settingsSubTab: SettingsSubTab;
  usageSubTab: UsageSubTab;
  selectedSkillId: string | null;
  selectedSkillName: string | null;
  sidebarVisible: boolean;
  newSkillRequest: number;
  discoverSearchTerm: string | null;
  skillEditMode: boolean;
  inboxRefreshKey: number;
  skills: Skill[];
  agents: Agent[];
  settings: AppSettings;
  setInitialized: (value: boolean) => void;
  setActiveTab: (tab: AppState["activeTab"]) => void;
  setSkillLibrarySubTab: (subTab: SkillLibrarySubTab) => void;
  setWorkMemorySubTab: (subTab: WorkMemorySubTab) => void;
  setSettingsSubTab: (subTab: SettingsSubTab) => void;
  setUsageSubTab: (subTab: UsageSubTab) => void;
  navigateToSettings: (subTab: SettingsSubTab) => void;
  setSelectedSkillId: (id: string | null) => void;
  setSelectedSkillName: (name: string | null) => void;
  toggleSidebar: () => void;
  setSidebarVisible: (visible: boolean) => void;
  requestNewSkill: () => void;
  navigateToSkill: (skillId: string, subTab?: SkillLibrarySubTab) => void;
  navigateToSkillEditor: (skillId: string) => void;
  setDiscoverSearchTerm: (term: string | null) => void;
  setSkillEditMode: (value: boolean) => void;
  bumpInboxRefresh: () => void;
  setSkills: (skills: Skill[]) => void;
  setAgents: (agents: Agent[]) => void;
  setSettings: (settings: AppSettings) => void;
  loadData: () => Promise<void>;
}

/**
 * 将旧版 tab key 映射到新的顶层 tab + 子导航。
 * 当前 activeTab 未持久化，但保留映射层以防御未来持久化值或 URL 传参。
 */
export function normalizeTab(
  legacy: LegacyTab
): { tab: AppTab; skillLibrarySubTab?: SkillLibrarySubTab; workMemorySubTab?: WorkMemorySubTab; settingsSubTab?: SettingsSubTab } {
  switch (legacy) {
    case "dashboard":
      return { tab: "today" };
    case "dailySummaries":
      return { tab: "workMemory", workMemorySubTab: "daily" };
    case "skills":
      return { tab: "skillLibrary", skillLibrarySubTab: "skills" };
    case "graph":
      return { tab: "skillLibrary", skillLibrarySubTab: "graph" };
    case "discover":
      return { tab: "skillLibrary", skillLibrarySubTab: "discover" };
    case "dataCollection":
      return { tab: "settings", settingsSubTab: "dataCollection" };
    case "scheduledTasks":
      return { tab: "settings", settingsSubTab: "scheduledTasks" };
    case "conflicts":
      return { tab: "today" };
    default:
      return { tab: legacy as AppTab };
  }
}

const defaultSettings: AppSettings = {
  device_id: "",
  auto_sync_interval_minutes: 5,
  launch_at_login: false,
  show_dock_icon: true,
  onboarding_completed: false,
  remote_enabled: false,
  theme: "system",
  openviking: { enabled: false, base_url: "http://localhost:1933", api_key: "" },
  ai: {
    models: [],
    acp_connections: [],
    default_chat_model_id: undefined,
    default_embedding_model_id: undefined,
    prefer_acp: false,
    strict_local_mode: false,
  },
};

const SIDEBAR_STORAGE_KEY = "skillmint:sidebar-visible";

function readSidebarDefault(): boolean {
  try {
    const raw = localStorage.getItem(SIDEBAR_STORAGE_KEY);
    return raw === null ? true : raw === "true";
  } catch {
    return true;
  }
}

function writeSidebarVisible(visible: boolean): void {
  try {
    localStorage.setItem(SIDEBAR_STORAGE_KEY, String(visible));
  } catch {
    // ignore storage errors (e.g. private mode)
  }
}

export const useAppStore = create<AppState>((set, get) => ({
  initialized: false,
  activeTab: "today",
  skillLibrarySubTab: "skills",
  workMemorySubTab: "weekly",
  settingsSubTab: "preferences",
  usageSubTab: "overview",
  selectedSkillId: null,
  selectedSkillName: null,
  sidebarVisible: readSidebarDefault(),
  newSkillRequest: 0,
  discoverSearchTerm: null,
  skillEditMode: false,
  inboxRefreshKey: 0,
  skills: [],
  agents: [],
  settings: defaultSettings,
  setInitialized: (value) => set({ initialized: value }),
  setActiveTab: (tab) => {
    // M1: legacy routes redirect to today.
    if (tab === "dashboard") {
      set({ activeTab: "today" });
      return;
    }
    // M1: knowledge graph / discover are Skill Library subtabs.
    if (tab === "knowledgeGraph") {
      set({ activeTab: "knowledgeGraph", skillLibrarySubTab: "graph" });
      return;
    }
    if (tab === "discover") {
      set({ activeTab: "discover", skillLibrarySubTab: "discover" });
      return;
    }
    // 周报 / 每日摘要已合并为「工作记忆」页（P4 IA 收敛）。
    if (tab === "weeklyReport") {
      set({ activeTab: "workMemory", workMemorySubTab: "weekly" });
      return;
    }
    if (tab === "dailySummaries") {
      set({ activeTab: "workMemory", workMemorySubTab: "daily" });
      return;
    }
    set({ activeTab: tab });
  },
  setSkillLibrarySubTab: (subTab) => set({ skillLibrarySubTab: subTab }),
  setWorkMemorySubTab: (subTab) => set({ workMemorySubTab: subTab }),
  setSettingsSubTab: (subTab) => set({ settingsSubTab: subTab }),
  setUsageSubTab: (subTab) => set({ usageSubTab: subTab }),
  navigateToSettings: (subTab) => set({ activeTab: "settings", settingsSubTab: subTab }),
  setSelectedSkillId: (id) => set({ selectedSkillId: id }),
  setSelectedSkillName: (name) => set({ selectedSkillName: name }),
  toggleSidebar: () => {
    const next = !get().sidebarVisible;
    writeSidebarVisible(next);
    set({ sidebarVisible: next });
  },
  setSidebarVisible: (visible) => {
    writeSidebarVisible(visible);
    set({ sidebarVisible: visible });
  },
  requestNewSkill: () => set((state) => ({ newSkillRequest: state.newSkillRequest + 1 })),
  navigateToSkill: (skillId, subTab = "skills") =>
    set({ activeTab: "skillLibrary", skillLibrarySubTab: subTab, selectedSkillId: skillId }),
  navigateToSkillEditor: (skillId) =>
    set({
      activeTab: "skillLibrary",
      skillLibrarySubTab: "skills",
      selectedSkillId: skillId,
      skillEditMode: true,
    }),
  setDiscoverSearchTerm: (term) => set({ discoverSearchTerm: term }),
  setSkillEditMode: (value) => set({ skillEditMode: value }),
  bumpInboxRefresh: () => set((state) => ({ inboxRefreshKey: state.inboxRefreshKey + 1 })),
  setSkills: (skills) => set({ skills }),
  setAgents: (agents) => set({ agents }),
  setSettings: (settings) => set({ settings }),
  loadData: async () => {
    // P3 startup fix: get_sync_targets no longer exists as a command — calling
    // it here rejected the whole init chain and left the app on the splash.
    const [skills, agents] = await Promise.all([
      invoke<Skill[]>("get_skills"),
      invoke<Agent[]>("get_agents"),
    ]);
    set({ skills, agents });
  },
}));
