import { CalendarDays, FileText, type LucideIcon } from "lucide-react";
import { useAppStore, type WorkMemorySubTab } from "../stores/appStore";
import WeeklyReport from "./WeeklyReport";
import DailySummaries from "./DailySummaries";
import { cn } from "../components/ui/utils";

const subTabs: { id: WorkMemorySubTab; label: string; icon: LucideIcon }[] = [
  { id: "weekly", label: "周报", icon: CalendarDays },
  { id: "daily", label: "每日摘要", icon: FileText },
];

/** 「工作记忆」：周报 + 每日摘要合并页（E2 工作记忆报告，见 docs/product-epics.md）。 */
export default function WorkMemory() {
  const subTab = useAppStore((state) => state.workMemorySubTab);
  const setSubTab = useAppStore((state) => state.setWorkMemorySubTab);

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-[var(--border-subtle)] px-8 pt-6 pb-0">
        <div className="mb-4">
          <h1 className="text-2xl font-bold text-primary">工作记忆</h1>
          <p className="mt-1 text-sm text-secondary">
            把每一次与 Agent 的协作沉淀为可回顾的记忆
          </p>
        </div>

        <nav className="flex gap-1">
          {subTabs.map((item) => {
            const Icon = item.icon;
            const isActive = subTab === item.id;
            return (
              <button
                key={item.id}
                onClick={() => setSubTab(item.id)}
                className={cn(
                  "flex items-center gap-2 border-b-2 px-4 py-2.5 text-sm font-medium transition-colors",
                  isActive
                    ? "border-accent text-accent"
                    : "border-transparent text-secondary hover:text-primary"
                )}
              >
                <Icon className="h-4 w-4" />
                {item.label}
              </button>
            );
          })}
        </nav>
      </div>

      <div className="min-h-0 flex-1">
        {subTab === "weekly" ? <WeeklyReport /> : <DailySummaries />}
      </div>
    </div>
  );
}
