import { useCallback, useRef } from "react";
import { invoke } from "../lib/invoke";
import { useAppStore } from "../stores/appStore";
import { useCollectionStore } from "../stores/collectionStore";
import { showError, showInfo, showSuccess } from "../stores/toastStore";
import { useHotkey } from "../hooks/useHotkeys";
import type { SyncAllResult } from "../types";

export interface GlobalShortcutsProps {
  onTogglePalette: () => void;
  onOpenHelp: () => void;
}

export function GlobalShortcuts({ onTogglePalette, onOpenHelp }: GlobalShortcutsProps) {
  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const setSkillLibrarySubTab = useAppStore((state) => state.setSkillLibrarySubTab);
  const setSettingsSubTab = useAppStore((state) => state.setSettingsSubTab);
  const toggleSidebar = useAppStore((state) => state.toggleSidebar);
  const loadData = useAppStore((state) => state.loadData);
  const requestNewSkill = useAppStore((state) => state.requestNewSkill);
  const startCollect = useCollectionStore((state) => state.startCollect);

  const syncDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const doSyncAll = useCallback(async () => {
    if (syncDebounceRef.current) return;
    syncDebounceRef.current = setTimeout(() => {
      syncDebounceRef.current = null;
    }, 1000);
    showInfo("同步已启动…", 2000);
    try {
      const result = await invoke<SyncAllResult>("sync_all_command");
      await loadData();
      if (result.failure_count > 0) {
        showError(
          `同步完成：${result.success_count} 成功，${result.failure_count} 失败`,
          5000
        );
      } else {
        showSuccess(`同步完成：${result.success_count} 个目标成功`);
      }
    } catch (err) {
      showError(err, { context: "同步" });
    }
  }, [loadData]);

  const doRefresh = useCallback(async () => {
    try {
      await loadData();
      showSuccess("已刷新");
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`刷新失败：${msg}`);
    }
  }, [loadData]);

  const openSettings = useCallback(() => {
    setActiveTab("settings");
    setSettingsSubTab("preferences");
  }, [setActiveTab, setSettingsSubTab]);

  const newSkill = useCallback(() => {
    setActiveTab("skillLibrary");
    setSkillLibrarySubTab("skills");
    requestNewSkill();
  }, [requestNewSkill, setActiveTab, setSkillLibrarySubTab]);

  const collectNow = useCallback(async () => {
    showInfo("采集已启动…", 2000);
    try {
      await startCollect();
      showSuccess("采集任务已启动");
    } catch (err) {
      showError(err, { context: "采集" });
    }
  }, [startCollect]);

  useHotkey("mod+k", onTogglePalette, { description: "命令面板", global: true });
  useHotkey("mod+shift+s", doSyncAll, { description: "同步全部" });
  useHotkey("mod+r", doRefresh, { description: "刷新当前页" });
  useHotkey("mod+,", openSettings, { description: "打开设置" });
  useHotkey("mod+n", newSkill, { description: "新建 Skill" });
  useHotkey("mod+b", toggleSidebar, { description: "切换侧边栏" });
  useHotkey("mod+/", onOpenHelp, { description: "键盘快捷键帮助" });
  // Also expose collect now globally for parity with command palette.
  useHotkey("mod+shift+c", collectNow, { description: "立即采集" });

  return null;
}
