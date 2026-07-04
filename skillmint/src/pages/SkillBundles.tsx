import { useState } from "react";
import { Plus } from "lucide-react";
import BundleManager from "../components/BundleManager";
import { Button } from "../components/ui/Button";

export default function SkillBundles() {
  const [showCreate, setShowCreate] = useState(false);

  return (
    <div className="flex h-full flex-col p-8">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-semibold tracking-tight text-primary">技能集</h1>
          <p className="mt-1 text-sm text-secondary">把常用 Skill 打包成可复用组合，一键应用到项目或导出分享。</p>
        </div>
        <div className="flex gap-2">
          <Button variant="primary" size="sm" onClick={() => setShowCreate(true)}>
            <Plus className="h-4 w-4" />
            新建技能集
          </Button>
        </div>
      </div>

      <div className="flex-1 overflow-hidden">
        <BundleManager showHeader={false} showCreate={showCreate} onShowCreateChange={setShowCreate} />
      </div>
    </div>
  );
}
