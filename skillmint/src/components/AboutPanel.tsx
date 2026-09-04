import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Box, Code, FileJson, HardDrive, Info, Network, Shield } from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { Card } from "./ui/Card";

export default function AboutPanel() {
  const settings = useAppStore((state) => state.settings);
  const [appVersion, setAppVersion] = useState<string>("");

  useEffect(() => {
    getVersion()
      .then((v) => setAppVersion(v))
      .catch(() => setAppVersion("0.1.0"));
  }, []);

  const infoItems = [
    { label: "应用名称", value: "积微 · SkillMint", icon: Box },
    { label: "版本号", value: appVersion ? `v${appVersion}` : "v0.1.0", icon: Info },
    { label: "运行模式", value: "Local Mode", icon: Shield },
    { label: "设备 ID", value: settings.device_id || "未生成", icon: HardDrive, mono: true },
  ];

  return (
    <div className="space-y-6">
      <Card className="p-6">
        <div className="flex items-center gap-4">
          <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-accent shadow-sm shadow-accent/20">
            <span className="text-2xl font-bold text-white">M</span>
          </div>
          <div>
            <h2 className="text-xl font-semibold text-primary">积微 · SkillMint</h2>
            <p className="text-sm text-secondary">把每一次重复，铸成会复利的技能资产</p>
            <p className="mt-0.5 text-xs text-tertiary italic">Mint your AI experience into compounding skill assets.</p>
            <p className="mt-1 text-xs text-tertiary">{appVersion ? `v${appVersion}` : "v0.1.0"} · Local Mode</p>
          </div>
        </div>
      </Card>

      <Card className="p-0 overflow-hidden">
        <div className="border-b border-[var(--border-subtle)] px-6 py-4">
          <h3 className="font-semibold text-primary">应用信息</h3>
          <p className="text-xs text-secondary">当前运行实例的基本信息</p>
        </div>
        <div className="divide-y divide-[var(--border-subtle)]">
          {infoItems.map((item) => {
            const Icon = item.icon;
            return (
              <div key={item.label} className="flex items-start gap-4 px-6 py-4">
                <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-secondary">
                  <Icon className="h-4 w-4 text-tertiary" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-sm text-secondary">{item.label}</div>
                  <div
                    className={`mt-0.5 break-all text-sm font-medium text-primary ${
                      item.mono ? "font-mono" : ""
                    }`}
                  >
                    {item.value}
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </Card>

      <Card className="p-6">
        <div className="flex items-start gap-3">
          <Code className="mt-0.5 h-4 w-4 text-tertiary" />
          <div>
            <h3 className="text-sm font-medium text-primary">开源与隐私</h3>
            <p className="mt-1 text-xs text-secondary leading-relaxed">
              SkillMint 默认在本地运行，所有 Skill 资产与使用数据均存储于本机中心仓库。
              远程功能为可选项，开启后才会发起网络请求。
            </p>
          </div>
        </div>
      </Card>

      <Card className="p-0 overflow-hidden">
        <div className="border-b border-[var(--border-subtle)] px-6 py-4">
          <h3 className="font-semibold text-primary">开放协议</h3>
          <p className="text-xs text-secondary">SkillMint 使用开放的存储与通信格式，避免厂商锁定。</p>
        </div>
        <div className="divide-y divide-[var(--border-subtle)]">
          <ProtocolItem
            icon={FileJson}
            title="Skill 存储格式"
            text="Markdown + YAML frontmatter，一个 Skill 一个目录，包含 SKILL.md 与可选资源。"
          />
          <ProtocolItem
            icon={Box}
            title="Bundle 格式"
            text="JSON manifest + skills/ 目录，可直接用文本编辑器查看、Git diff 与分享。"
          />
          <ProtocolItem
            icon={Network}
            title="ACP 协议"
            text="JSON-RPC 2.0 over stdio 或 SSE，调用本地 Agent CLI 的 agent.chat 方法。"
          />
        </div>
      </Card>
    </div>
  );
}

function ProtocolItem({
  icon: Icon,
  title,
  text,
}: {
  icon: React.ElementType;
  title: string;
  text: string;
}) {
  return (
    <div className="flex items-start gap-4 px-6 py-4">
      <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-secondary">
        <Icon className="h-4 w-4 text-tertiary" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-primary">{title}</div>
        <div className="mt-0.5 text-xs text-secondary leading-relaxed">{text}</div>
      </div>
    </div>
  );
}
