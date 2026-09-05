import { Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "./lib/invoke";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import Sidebar from "./components/Sidebar";
import ToastContainer from "./components/ToastContainer";
import { PageTransition } from "./components/PageTransition";
import { Sprout } from "lucide-react";
import { SkeletonCard } from "./components/ui/Skeleton";
import { useAppStore } from "./stores/appStore";
import { humanizeError, showError, showSuccess } from "./stores/toastStore";
import { rebuildSkillIndex } from "./lib/npxskills";
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
  // TEMP DEBUG: surface init failures visibly (remove after diagnosis).
  const [initError, setInitError] = useState<string | null>(null);

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
    // P3 startup hardening: a failure in ANY init step must never leave the
    // window on the splash forever. Core settings still gate the stores; a
    // loadData failure degrades to toasts inside the main UI instead.
    invoke("init_app")
      .then(() => invoke<typeof settings>("get_settings"))
      .then((savedSettings) => {
        setSettings(savedSettings);
      })
      .catch((err) => {
        console.error("[SkillMint] init failed:", err);
        const raw = err as Error & { detail?: string };
        setInitError([raw.message, raw.detail].filter(Boolean).join(" | ") || String(err));
        showError(humanizeError(err, { context: "初始化" }));
      })
      .finally(() => {
        loadData()
          .catch((err) => {
            console.error("[SkillMint] initial load failed:", err);
            showError(humanizeError(err, { context: "初始加载" }));
          })
          .finally(() => setInitialized(true));
      });
  }, [loadData, setInitialized, setSettings]);

  useEffect(() => {
    if (!initialized) return;

    if (intervalRef.current) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }

    // P3-3: the frontend tick is a light "refresh if stale" probe; the real
    // fingerprint check also runs on the backend scheduler. Npx-managed state
    // (locks / agent dirs / hubs) is the only thing that can change here.
    const minutes = settings.auto_sync_interval_minutes;
    if (minutes > 0) {
      intervalRef.current = setInterval(() => {
        rebuildSkillIndex(null)
          .then(() => loadData())
          .catch((err) => {
            console.error("[SkillMint] index refresh failed:", err);
          });
      }, minutes * 60 * 1000);
    }

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    };
  }, [initialized, settings.auto_sync_interval_minutes, loadData]);

  // SPEC-F3: handle skillmint:// deep links.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let unlistenSync: UnlistenFn | undefined;
    let unlistenOpen: UnlistenFn | undefined;
    let unlistenNav: UnlistenFn | undefined;

    const setup = async () => {
      unlisten = await listen<string>("deep-link", (e) => {
        console.log("[deep-link]", e.payload);
      });
      // P3-3: deep-link sync now means "rebuild the skill index from disk".
      unlistenSync = await listen("deep-link-sync", () => {
        rebuildSkillIndex(null)
          .then((summary) => {
            showSuccess(`已刷新技能索引：${summary.total} 项`);
            return loadData();
          })
          .catch((err) => showError(humanizeError(err, { context: "刷新索引" })));
      });
      unlistenOpen = await listen<string>("deep-link-open-skill", (e) => {
        setActiveTab("skillLibrary");
        setSelectedSkillName(e.payload);
      });
      // P1-2: tray 直达菜单（Skill 库 / 同步健康）——Rust 侧 show+focus 后
      // 经 deep-link 可靠性通道重发，这里只做页面路由。
      unlistenNav = await listen<string>("tray-navigate", (e) => {
        if (e.payload === "skillLibrary" || e.payload === "today") {
          setActiveTab(e.payload);
        }
      });
      // P1-1: listeners are registered — tell the backend to replay any
      // deep links that arrived before the WebView was ready.
      emit("app-ready");
    };
    setup();

    return () => {
      unlisten?.();
      unlistenSync?.();
      unlistenOpen?.();
      unlistenNav?.();
    };
  }, [loadData, setActiveTab, setSelectedSkillName]);

  if (!initialized) {
    return <BootSplash initError={initError} />;
  }

  if (!settings.onboarding_completed) {
    return (
      <div className="h-full">
        {initError && (
          <div className="m-4 rounded border border-danger/30 bg-danger/10 p-3 text-xs text-danger">
            初始化出现问题（功能受限，可继续使用）：{initError}
          </div>
        )}
        <Suspense fallback={<FullPageSkeleton />}>
          <Onboarding />
        </Suspense>
        <ToastContainer />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col bg-primary text-primary">
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

function BootSplash({ initError }: { initError: string | null }) {
  return (
    <div className="flex h-full select-none flex-col items-center justify-center gap-7 bg-primary">
      <div className="flex items-center gap-3">
        <div className="flex h-11 w-11 items-center justify-center rounded-2xl bg-accent shadow-lg shadow-accent/30">
          <Sprout className="h-6 w-6 text-white" strokeWidth={2.2} />
        </div>
        <span className="text-3xl font-bold tracking-tight text-primary">SkillMint</span>
      </div>
      <div className="h-1 w-52 overflow-hidden rounded-full bg-tertiary">
        <div className="animate-boot-bar h-full w-1/3 rounded-full bg-accent" />
      </div>
      <p className="text-xs tracking-wide text-tertiary">正在初始化…</p>
      {initError && (
        <p className="max-w-xl rounded border border-danger/30 bg-danger/10 p-2 text-xs text-danger">{initError}</p>
      )}
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
