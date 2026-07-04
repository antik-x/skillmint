import { Suspense, lazy } from "react";
import { Globe, Layers, Puzzle, Share2 } from "lucide-react";
import { useAppStore, type SkillLibrarySubTab } from "../stores/appStore";
import { SkeletonCard } from "../components/ui/Skeleton";
import { cn } from "../components/ui/utils";

const Skills = lazy(() => import("./Skills"));
const SkillBundles = lazy(() => import("./SkillBundles"));
const KnowledgeGraph = lazy(() => import("./KnowledgeGraph"));
const Discover = lazy(() => import("./Discover"));

interface SubTabItem {
  id: SkillLibrarySubTab;
  label: string;
  icon: React.ElementType;
}

const subTabs: SubTabItem[] = [
  { id: "skills", label: "技能", icon: Puzzle },
  { id: "bundles", label: "技能集", icon: Layers },
  { id: "graph", label: "知识图谱", icon: Share2 },
  { id: "discover", label: "发现", icon: Globe },
];

export default function SkillLibrary() {
  const skillLibrarySubTab = useAppStore((state) => state.skillLibrarySubTab);
  const setSkillLibrarySubTab = useAppStore((state) => state.setSkillLibrarySubTab);

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-[var(--border-subtle)] px-8 pt-6 pb-0">
        <div className="mb-4 flex items-end justify-between">
          <div>
            <h1 className="text-2xl font-bold text-primary">技能</h1>
            <p className="mt-1 text-sm text-secondary">管理技能、查看关系图谱、发现新技能</p>
          </div>
        </div>

        <nav className="flex gap-1">
          {subTabs.map((item) => {
            const Icon = item.icon;
            const isActive = skillLibrarySubTab === item.id;
            return (
              <button
                key={item.id}
                onClick={() => setSkillLibrarySubTab(item.id)}
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

      <div className="flex-1 overflow-hidden">
        <Suspense fallback={<SkillLibrarySkeleton />}>
          {skillLibrarySubTab === "skills" && <Skills />}
          {skillLibrarySubTab === "bundles" && <SkillBundles />}
          {skillLibrarySubTab === "graph" && <KnowledgeGraph />}
          {skillLibrarySubTab === "discover" && <Discover />}
        </Suspense>
      </div>
    </div>
  );
}

function SkillLibrarySkeleton() {
  return (
    <div className="h-full overflow-auto p-8">
      <SkeletonCard className="mb-4 h-12" />
      <SkeletonCard className="h-48" />
    </div>
  );
}
