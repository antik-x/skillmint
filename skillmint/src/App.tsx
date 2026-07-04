import { Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "./lib/invoke";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Sidebar from "./components/Sidebar";
import ToastContainer from "./components/ToastContainer";
import { PageTransition } from "./components/PageTransition";
import { SkeletonCard } from "./components/ui/Skeleton";
import { useAppStore } from "./stores/appStore";
import { humanizeError, showError, showSuccess } from "./stores/toastStore";
import type { RepoIntegrity, SyncAllResult } from "./types";
import { GlobalShortcuts } from "./components/GlobalShortcuts";
import CommandPalette from "./components/CommandPalette";
import { ShortcutsHelp } from "./components/ShortcutsHelp";
import { ErrorBoundary } from "./components/ErrorBoundary";

const Today = lazy(() => import("./pages/Today"));
const Inbox = lazy(() => import("./pages/Inbox"));
const GrowthAssets = lazy(() => import("./pages/GrowthAssets"));
const WeeklyReport = lazy(() => import("./pages/WeeklyReport"));
const SkillLibrary = lazy(() => import("./pages/SkillLibrary"));
const Agents = lazy(() => import("./pages/Agents"));
const Projects = lazy(() => import("./pages/Projects"));
const Usage = lazy(() => import("./pages/Usage"));
const DailySummaries = lazy(() => import("./pages/DailySummaries"));
const Dashboard = lazy(() => import("./pages/Dashboard"));
const Settings = lazy(() => import("./pages/Settings"));
const Onboarding = lazy(() => import("./pages/Onboarding"));

function App() {
  const initialized = useAppStore((state) => state.initialized);
  const activeTab = useAppStore((state) => state.activeTab);
  const settings = useAppStore((state) => state.settings);
  const setInitialized = useAppStore((state) => state.setInitialized);
  const setSettings = useAppStore((state) => state.setSettings);
  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const setSelectedSkillName = useAppStore((state) => state.setSelectedSkillName);
  const loadData = useAppStore((state) => state.loadData);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // I3: command palette + shortcuts help overlay.
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);

  // F6: recovery banner when the center repo is missing but DB still has skills.
  const [repoIntegrity, setRepoIntegrity] = useState<RepoIntegrity | null>(null);
  // F7: track consecutive auto-sync failures and avoid spam toasts.
  const [syncFailures, setSyncFailures] = useState(0);

  const togglePalette = useCallback(() => setPaletteOpen((v) => !v), []);
  const openHelp = useCallback(() => setHelpOpen(true), []);

  // M1: apply persisted theme on startup.
  useEffect(() => {
    if (settings.theme === "light") {
      document.documentElement.dataset.theme = "light";
    } else if (settings.theme === "dark") {
      document.documentElement.dataset.theme = "dark";
    } else {
      delete document.documentElement.dataset.theme;
    }
  }, [settings.theme]);

  useEffect(() => {
    invoke("init_app")
      .then(() => invoke<typeof settings>("get_settings"))
      .then((savedSettings) => {
        setSettings(savedSettings);
        return loadData();
      })
      .then(() => setInitialized(true))
      .catch((err) => {
        console.error("[SkillMint] init failed:", err);
        showError(humanizeError(err, { context: "初始化" }));
      });
  }, [loadData, setInitialized, setSettings]);

  // F6: check repo integrity on mount and listen for backend events.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    const check = async () => {
      try {
        const integrity = await invoke<RepoIntegrity>("check_repo_integrity");
        setRepoIntegrity(integrity);
      } catch (err) {
        console.error("[SkillMint] check_repo_integrity failed:", err);
      }
    };

    check();
    listen("repo-missing-with-records", () => {
      setRepoIntegrity("missing_with_records");
    }).then((u) => {
      unlisten = u;
    });

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    if (!initialized) return;

    if (intervalRef.current) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }

    const minutes = settings.auto_sync_interval_minutes;
    if (minutes > 0) {
      intervalRef.current = setInterval(() => {
        invoke<SyncAllResult>("sync_all_command")
          .then((result) => {
            setSyncFailures(0);
            if (result.failure_count > 0) {
              showError(
                `自动同步完成：${result.success_count} 成功，${result.failure_count} 失败（${result.failures[0]?.agent_name ?? ""}）`
              );
            }
            return loadData();
          })
          .catch((err) => {
            setSyncFailures((n) => n + 1);
            console.error("[SkillMint] auto-sync failed:", err);
            // Only toast after 3 consecutive failures to avoid noise.
            if (syncFailures + 1 >= 3) {
              showError(`自动同步连续失败：${humanizeError(err, { context: "自动同步" }).message}`);
            }
          });
      }, minutes * 60 * 1000);
    }

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    };
  }, [initialized, settings.auto_sync_interval_minutes, loadData, syncFailures]);

  // SPEC-F3: handle skillmint:// deep links.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let unlistenSync: UnlistenFn | undefined;
    let unlistenOpen: UnlistenFn | undefined;

    const setup = async () => {
      unlisten = await listen<string>("deep-link", (e) => {
        console.log("[deep-link]", e.payload);
      });
      unlistenSync = await listen("deep-link-sync", () => {
        invoke<SyncAllResult>("sync_all_command")
          .then((result) => {
            if (result.failure_count > 0) {
              showError(`同步完成：${result.success_count} 成功，${result.failure_count} 失败`);
            } else {
              showSuccess(`同步完成：${result.success_count} 个目标成功`);
            }
            return loadData();
          })
          .catch((err) => showError(humanizeError(err, { context: "同步" })));
      });
      unlistenOpen = await listen<string>("deep-link-open-skill", (e) => {
        setActiveTab("skillLibrary");
        setSelectedSkillName(e.payload);
      });
    };
    setup();

    return () => {
      unlisten?.();
      unlistenSync?.();
      unlistenOpen?.();
    };
  }, [loadData, setActiveTab, setSelectedSkillName]);

  if (!initialized) {
    return (
      <div className="flex h-full items-center justify-center bg-primary">
        <div className="w-64 space-y-4 text-center">
          <div className="mb-4 text-2xl font-bold tracking-tight text-primary">SkillMint</div>
          <SkeletonCard className="animate-pulse" />
          <p className="text-sm text-secondary">正在初始化...</p>
        </div>
      </div>
    );
  }

  if (!settings.onboarding_completed) {
    return (
      <div className="h-full">
        <Suspense fallback={<FullPageSkeleton />}>
          <Onboarding />
        </Suspense>
        <ToastContainer />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col bg-primary text-primary">
      {repoIntegrity === "missing_with_records" && (
        <div className="flex items-center justify-between border-b border-warning/20 bg-warning/10 px-4 py-2 text-xs text-warning">
          <span>
            ⚠ 中心仓库缺失或为空，但数据库仍有 Skill 记录。请从备份恢复，或在设置中确认重建。
          </span>
          <div className="flex gap-2">
            <button
              onClick={() => setActiveTab("settings")}
              className="rounded border border-warning/20 px-2 py-0.5 hover:bg-warning/20"
            >
              去设置
            </button>
            <button
              onClick={() => setRepoIntegrity("healthy")}
              className="rounded border border-warning/20 px-2 py-0.5 hover:bg-warning/20"
            >
              忽略
            </button>
          </div>
        </div>
      )}
      <div className="flex flex-1 overflow-hidden">
        <Sidebar />
        <main className="relative flex-1 overflow-hidden">
          <Suspense fallback={<FullPageSkeleton />}>
            <PageTransition trigger={activeTab}>
              {activeTab === "today" && <Today />}
              {activeTab === "inbox" && <Inbox />}
              {activeTab === "growthAssets" && <GrowthAssets />}
              {(activeTab === "skillLibrary" || activeTab === "knowledgeGraph" || activeTab === "discover") && <SkillLibrary />}
              {activeTab === "agents" && <Agents />}
              {activeTab === "projects" && <Projects />}
              {activeTab === "usage" && <Usage />}
              {activeTab === "weeklyReport" && <WeeklyReport />}
              {activeTab === "settings" && (
                <ErrorBoundary>
                  <Settings />
                </ErrorBoundary>
              )}
              {/* Legacy routes kept reachable for redirects / bookmarks. */}
              {activeTab === "dashboard" && <Dashboard />}
              {activeTab === "dailySummaries" && <DailySummaries />}
            </PageTransition>
          </Suspense>
        </main>
        <ToastContainer />
      </div>

      {/* I3: global keyboard shortcuts, command palette and help overlay. */}
      <GlobalShortcuts onTogglePalette={togglePalette} onOpenHelp={openHelp} />
      <CommandPalette open={paletteOpen} onClose={() => setPaletteOpen(false)} />
      <ShortcutsHelp open={helpOpen} onClose={() => setHelpOpen(false)} />
    </div>
  );
}

function FullPageSkeleton() {
  return (
    <div className="h-full overflow-auto p-8">
      <div className="mb-8 grid grid-cols-1 gap-4 sm:grid-cols-3">
        <SkeletonCard />
        <SkeletonCard />
        <SkeletonCard />
      </div>
      <SkeletonCard className="h-48" />
    </div>
  );
}

export default App;
