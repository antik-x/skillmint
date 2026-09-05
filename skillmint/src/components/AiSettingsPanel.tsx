import { useEffect, useMemo, useState } from "react";
import { invoke } from "../lib/invoke";
import {
  Bot,
  Brain,
  Check,
  ChevronDown,
  Database,
  FlaskConical,
  Key,
  Link,
  Plus,
  Search,
  Server,
  Sparkles,
  Trash2,
} from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { showError, showSuccess } from "../stores/toastStore";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";
import { HelpTip } from "./ui/HelpTip";
import { Input } from "./ui/Input";
import type { AiConfig, AiModelConfig, AcpConnectionConfig, AcpTransport, DetectedAgent, LlmRequestLog, OpenVikingConfig } from "../types";

/** P0 探针返回结构（与 Rust `openviking::ProbeResult` 对应）。 */
interface OvProbeResult {
  state: "disabled" | "available" | "auth_failed" | "incompatible" | "unreachable";
  detail: string;
  capabilities: { path: string; ok: boolean }[];
}

const PROVIDER_LABELS: Record<string, string> = {
  openai: "OpenAI 兼容",
  anthropic: "Anthropic",
};

function newId(prefix: string) {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
}

function defaultModel(): AiModelConfig {
  return {
    id: newId("model"),
    name: "",
    provider: "openai",
    model: "",
    base_url: "",
    api_key: "",
    capabilities: ["chat"],
  };
}

function defaultAcpConnection(): AcpConnectionConfig {
  return {
    id: newId("acp"),
    name: "",
    enabled: true,
    transport: { kind: "stdio", command: "", args: [], env: {} },
  };
}

export default function AiSettingsPanel() {
  const { settings, setSettings } = useAppStore();
  // Always read the latest settings at save time. The destructured `settings`
  // above is a render snapshot and can be stale if the store updates between
  // renders; using it in `handleSave` previously meant a stale `...settings`
  // could clobber other fields (issue #2). `getSettings` reads live.
  const getSettings = useAppStore((s) => s.settings);
  const [form, setForm] = useState<AiConfig>(() => {
    // SPEC-F2 T3: guard against missing or partial AI settings to prevent white-screen.
    const ai = settings.ai;
    if (ai && Array.isArray(ai.models) && Array.isArray(ai.acp_connections)) {
      return ai;
    }
    return {
      models: ai?.models ?? [],
      acp_connections: ai?.acp_connections ?? [],
      prefer_acp: ai?.prefer_acp ?? false,
      strict_local_mode: ai?.strict_local_mode ?? false,
      default_chat_model_id: ai?.default_chat_model_id,
      default_embedding_model_id: ai?.default_embedding_model_id,
    };
  });
  const [saving, setSaving] = useState(false);
  const [testingModelId, setTestingModelId] = useState<string | null>(null);
  const [testingAcpId, setTestingAcpId] = useState<string | null>(null);
  const [detectedAgents, setDetectedAgents] = useState<DetectedAgent[]>([]);
  const [detectingAgents, setDetectingAgents] = useState(false);

  // P4: OpenViking integration (root-level settings, separate save path).
  const [ov, setOv] = useState<OpenVikingConfig>(() =>
    settings.openviking ?? { enabled: false, base_url: "http://localhost:1933", api_key: "" }
  );
  const [ovSaving, setOvSaving] = useState(false);
  const [ovProbing, setOvProbing] = useState(false);
  const [ovProbe, setOvProbe] = useState<OvProbeResult | null>(null);

  useEffect(() => {
    detectAgents();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const embeddingModels = useMemo(
    () => form.models.filter((m) => m.capabilities.includes("embedding")),
    [form.models]
  );

  const setAi = (patch: Partial<AiConfig>) => {
    setForm((prev) => ({ ...prev, ...patch }));
  };

  const updateModel = (id: string, patch: Partial<AiModelConfig>) => {
    setForm((prev) => ({
      ...prev,
      models: prev.models.map((m) => (m.id === id ? { ...m, ...patch } : m)),
    }));
  };

  const addModel = (capability: "chat" | "embedding") => {
    const model = defaultModel();
    model.capabilities = [capability];
    if (capability === "chat" && !form.default_chat_model_id && form.models.length === 0) {
      setAi({ models: [...form.models, model], default_chat_model_id: model.id });
    } else if (
      capability === "embedding" &&
      !form.default_embedding_model_id &&
      embeddingModels.length === 0
    ) {
      setAi({ models: [...form.models, model], default_embedding_model_id: model.id });
    } else {
      setAi({ models: [...form.models, model] });
    }
  };

  const removeModel = (id: string) => {
    setForm((prev) => ({
      ...prev,
      models: prev.models.filter((m) => m.id !== id),
      default_chat_model_id:
        prev.default_chat_model_id === id ? undefined : prev.default_chat_model_id,
      default_embedding_model_id:
        prev.default_embedding_model_id === id ? undefined : prev.default_embedding_model_id,
    }));
  };

  const updateAcp = (id: string, patch: Partial<AcpConnectionConfig>) => {
    setForm((prev) => ({
      ...prev,
      acp_connections: prev.acp_connections.map((c) => (c.id === id ? { ...c, ...patch } : c)),
    }));
  };

  const updateAcpTransport = (id: string, patch: Partial<AcpTransport>) => {
    setForm((prev) => ({
      ...prev,
      acp_connections: prev.acp_connections.map((c) =>
        c.id === id ? { ...c, transport: { ...c.transport, ...patch } as AcpTransport } : c
      ),
    }));
  };

  const addAcp = () => {
    setAi({ acp_connections: [...form.acp_connections, defaultAcpConnection()] });
  };

  const addDetectedAcp = (agent: DetectedAgent) => {
    const conn: AcpConnectionConfig = {
      ...defaultAcpConnection(),
      name: agent.display_name,
      transport: { kind: "stdio", command: agent.command, args: agent.args, env: {} },
    };
    setAi({ acp_connections: [...form.acp_connections, conn] });
  };

  const detectAgents = async () => {
    if (detectingAgents) return;
    setDetectingAgents(true);
    try {
      const agents = await invoke<DetectedAgent[]>("detect_local_agents");
      setDetectedAgents(agents);
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`检测失败：${message}`);
    } finally {
      setDetectingAgents(false);
    }
  };

  const removeAcp = (id: string) => {
    setAi({ acp_connections: form.acp_connections.filter((c) => c.id !== id) });
  };

  const handleSave = async () => {
    if (saving) return;
    setSaving(true);
    try {
      // Read the freshest settings from the store instead of the render
      // snapshot, so we don't accidentally persist a stale `ai` block or drop
      // sibling fields. Only `ai` is what this panel owns.
      const saved = await invoke<typeof settings>("save_settings", {
        newSettings: { ...getSettings, ai: form },
      });
      setSettings(saved);
      showSuccess("AI 设置已保存");
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`保存失败：${message}`);
    } finally {
      setSaving(false);
    }
  };

  const testModel = async (model: AiModelConfig) => {
    if (!model.api_key || testingModelId) return;
    setTestingModelId(model.id);
    try {
      // Simple non-invasive test via a short chat request.
      await invoke<string>("test_ai_model", { modelId: model.id });
      showSuccess(`模型「${model.name || model.model}」连接正常`);
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`模型测试失败：${message}`);
    } finally {
      setTestingModelId(null);
    }
  };

  const testAcp = async (id: string) => {
    if (testingAcpId) return;
    setTestingAcpId(id);
    try {
      const msg = await invoke<string>("test_acp_transport", { id });
      showSuccess(`ACP 连接成功：${msg.slice(0, 200)}`);
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`ACP 连接失败：${message}`);
    } finally {
      setTestingAcpId(null);
    }
  };

  // P4 OpenViking: persist root-level settings, then run the read-only probe.
  // Probing after save guarantees the backend uses exactly what's on screen.
  const saveOv = async (): Promise<boolean> => {
    setOvSaving(true);
    try {
      const saved = await invoke<typeof settings>("save_settings", {
        newSettings: { ...getSettings, openviking: ov },
      });
      setSettings(saved);
      return true;
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`OpenViking 设置保存失败：${message}`);
      return false;
    } finally {
      setOvSaving(false);
    }
  };

  const handleOvSave = async () => {
    if (await saveOv()) showSuccess("OpenViking 设置已保存（API key 存入钥匙串）");
  };

  const handleOvProbe = async () => {
    if (ovProbing) return;
    if (!(await saveOv())) return;
    setOvProbing(true);
    try {
      setOvProbe(await invoke<OvProbeResult>("openviking_probe"));
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`探测失败：${message}`);
    } finally {
      setOvProbing(false);
    }
  };

  return (
    <div className="space-y-6">
      <div className="rounded-xl border border-[var(--border-subtle)] bg-secondary p-6">
        <div className="flex items-start gap-3">
          <Brain className="mt-0.5 h-5 w-5 text-accent" />
          <div>
            <h2 className="text-lg font-semibold text-primary">AI 分析（可选）</h2>
            <p className="text-sm text-secondary">
              配置后可用于 Prompt 语义分类、每日 AI 工作摘要等分析任务。留空则跳过这些功能，其它分析不受影响。
            </p>
          </div>
        </div>
      </div>

      <Card className="p-0 overflow-hidden">
        <div className="border-b border-[var(--border-subtle)] px-6 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Sparkles className="h-4 w-4 text-accent" />
              <h3 className="font-semibold text-primary">模型配置</h3>
            </div>
            <div className="flex gap-2">
              <Button variant="secondary" size="sm" onClick={() => addModel("chat")}>
                <Plus className="mr-1 h-4 w-4" />
                添加对话模型
              </Button>
              <Button variant="secondary" size="sm" onClick={() => addModel("embedding")}>
                <Plus className="mr-1 h-4 w-4" />
                添加 Embedding
              </Button>
            </div>
          </div>
        </div>

        {form.models.length === 0 ? (
          <div className="p-10 text-center text-sm text-secondary">
            尚未配置任何模型。点击上方按钮添加第一个模型。
          </div>
        ) : (
          <div className="divide-y divide-[var(--border-subtle)]">
            {form.models.map((model) => (
              <ModelRow
                key={model.id}
                model={model}
                isDefaultChat={form.default_chat_model_id === model.id}
                isDefaultEmbedding={form.default_embedding_model_id === model.id}
                onChange={(patch) => updateModel(model.id, patch)}
                onTest={() => testModel(model)}
                onRemove={() => removeModel(model.id)}
                onSetDefaultChat={() => setAi({ default_chat_model_id: model.id })}
                onSetDefaultEmbedding={() => setAi({ default_embedding_model_id: model.id })}
                testing={testingModelId === model.id}
              />
            ))}
          </div>
        )}
      </Card>

      <Card className="p-0 overflow-hidden">
        <div className="border-b border-[var(--border-subtle)] px-6 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Bot className="h-4 w-4 text-accent" />
              <h3 className="flex items-center gap-1.5 font-semibold text-primary">
                本地 Agent 分析（ACP）
                {/* SPEC-F6 T2: 就地解释「ACP」。 */}
                <HelpTip
                  ariaLabel="什么是 ACP"
                  text="Agent Client Protocol——让本机的 AI Agent 充当分析引擎，数据不出本机。"
                />
              </h3>
            </div>
            <Button variant="secondary" size="sm" onClick={addAcp}>
              <Plus className="mr-1 h-4 w-4" />
              添加连接
            </Button>
          </div>
          <p className="mt-1 text-xs text-secondary">
            启用后，语义分类、每日摘要等任务会优先调用本地智能体（如 Claude Code / Kimi Code CLI），无 API Key 时也能运行。
          </p>
        </div>

        <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-6 py-3">
          <span className="text-sm text-secondary">优先使用 ACP</span>
          <label className="relative inline-flex cursor-pointer items-center">
            <input
              type="checkbox"
              checked={form.prefer_acp}
              onChange={(e) => setAi({ prefer_acp: e.target.checked })}
              className="peer sr-only"
            />
            <div className="peer h-6 w-11 rounded-full bg-tertiary after:absolute after:left-[2px] after:top-[2px] after:h-5 after:w-5 after:rounded-full after:bg-primary after:transition-all peer-checked:bg-accent peer-checked:after:translate-x-full" />
          </label>
        </div>

        <AcpAutoDetect
          agents={detectedAgents}
          detecting={detectingAgents}
          existing={form.acp_connections}
          onDetect={detectAgents}
          onAdd={addDetectedAcp}
        />

        {form.acp_connections.length === 0 ? (
          <div className="p-10 text-center text-sm text-secondary">
            尚未配置 ACP 连接。点击上方按钮添加，或使用自动检测。
          </div>
        ) : (
          <div className="divide-y divide-[var(--border-subtle)]">
            {form.acp_connections.map((conn) => (
              <AcpRow
                key={conn.id}
                connection={conn}
                onChange={(patch) => updateAcp(conn.id, patch)}
                onTransportChange={(patch) => updateAcpTransport(conn.id, patch)}
                onTest={() => testAcp(conn.id)}
                onRemove={() => removeAcp(conn.id)}
                testing={testingAcpId === conn.id}
              />
            ))}
          </div>
        )}
      </Card>

      <Card className="p-0 overflow-hidden" >
        <div className="flex items-start gap-3 border-b border-[var(--border-subtle)] px-6 py-5">
          <Database className="mt-0.5 h-5 w-5 text-accent" />
          <div className="flex-1">
            <h2 className="text-lg font-semibold text-primary">
              OpenViking · 外置上下文引擎
              <span className="ml-2 align-middle text-xs font-normal text-tertiary">可选增强 · P0 只读探针</span>
            </h2>
            <p className="mt-1 text-sm text-secondary">
              连接本机运行的 OpenViking 上下文数据库，为技能查找、关系发现与记忆召回提供语义增强。只增强、不替代——关闭后一切功能照常。
            </p>
          </div>
        </div>

        <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-6 py-3">
          <span className="text-sm text-secondary">启用 OpenViking 集成</span>
          <label className="relative inline-flex cursor-pointer items-center">
            <input
              type="checkbox"
              checked={ov.enabled}
              onChange={(e) => setOv((p) => ({ ...p, enabled: e.target.checked }))}
              className="peer sr-only"
            />
            <div className="peer h-6 w-11 rounded-full bg-tertiary after:absolute after:left-[2px] after:top-[2px] after:h-5 after:w-5 after:rounded-full after:bg-primary after:transition-all peer-checked:bg-accent peer-checked:after:translate-x-full" />
          </label>
        </div>

        <div className="space-y-4 px-6 py-4">
          <div>
            <label className="mb-1 block text-sm text-secondary">服务地址</label>
            <Input
              value={ov.base_url}
              onChange={(e) => setOv((p) => ({ ...p, base_url: e.target.value }))}
              placeholder="http://localhost:1933"
            />
          </div>
          <div>
            <label className="mb-1 block text-sm text-secondary">API Key（Bearer，存入系统钥匙串）</label>
            <Input
              type="password"
              value={ov.api_key ?? ""}
              onChange={(e) => setOv((p) => ({ ...p, api_key: e.target.value }))}
              placeholder="ov-…"
            />
          </div>
          <p className="text-xs text-tertiary">
            门控为双开关：需同时打开「偏好设置 → 远程功能」与上方开关，才会发出任何请求（包括 localhost）。
            注意：OpenViking 本地存储 ≠ 完全离线——其 embedding/VLM 可能经火山引擎 Ark 出网，P0 探针仅访问只读接口。
          </p>
          <div className="flex items-center gap-3">
            <Button variant="secondary" size="sm" onClick={handleOvSave} loading={ovSaving} disabled={ovSaving}>
              保存 OpenViking 设置
            </Button>
            <Button variant="primary" size="sm" onClick={handleOvProbe} loading={ovProbing} disabled={ovProbing}>
              探测连通性
            </Button>
          </div>
          {ovProbe && (
            <div
              className="rounded-lg border border-[var(--border-subtle)] bg-secondary p-4 text-sm"
              data-testid="ov-probe-result"
            >
              <p className="font-medium">
                状态：
                {ovProbe.state === "available" && <span className="text-green-600">可用</span>}
                {ovProbe.state === "disabled" && <span className="text-secondary">未启用</span>}
                {ovProbe.state === "auth_failed" && <span className="text-red-600">认证失败</span>}
                {ovProbe.state === "incompatible" && <span className="text-amber">版本不兼容</span>}
                {ovProbe.state === "unreachable" && <span className="text-amber">无法连接</span>}
              </p>
              <p className="mt-1 text-xs text-secondary">{ovProbe.detail}</p>
              {ovProbe.capabilities.length > 0 && (
                <ul className="mt-2 space-y-1 text-xs text-tertiary font-mono">
                  {ovProbe.capabilities.map((c) => (
                    <li key={c.path}>
                      {c.ok ? "✓" : "✗"} {c.path}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </div>
      </Card>

      <PrivacyCard
        strictLocalMode={form.strict_local_mode}
        onChangeStrict={(v) => setAi({ strict_local_mode: v })}
      />

      <div className="pt-2">
        <Button variant="primary" size="md" onClick={handleSave} loading={saving} disabled={saving}>
          {saving ? "保存中…" : "保存 AI 设置"}
        </Button>
      </div>
    </div>
  );
}

function PrivacyCard({
  strictLocalMode,
  onChangeStrict,
}: {
  strictLocalMode: boolean;
  onChangeStrict: (v: boolean) => void;
}) {
  const [logs, setLogs] = useState<LlmRequestLog[]>([]);
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);

  const loadLogs = async () => {
    setLoading(true);
    try {
      const data = await invoke<LlmRequestLog[]>("list_llm_request_logs", { limit: 50 });
      setLogs(data);
    } catch (err) {
      const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`读取日志失败：${message}`);
    } finally {
      setLoading(false);
    }
  };

  const openLogs = async () => {
    setOpen(true);
    await loadLogs();
  };

  return (
    <Card className="p-0 overflow-hidden">
      <div className="border-b border-[var(--border-subtle)] px-6 py-4">
        <div className="flex items-center gap-2">
          <Key className="h-4 w-4 text-accent" />
          <h3 className="font-semibold text-primary">隐私与路由</h3>
        </div>
        <p className="mt-1 text-xs text-secondary">
          控制分析任务是否允许离开本机，并审计最近的 LLM/ACP 调用路由。
        </p>
      </div>

      <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-6 py-3">
        <div>
          <div className="text-sm text-secondary">严格本地模式</div>
          <div className="text-xs text-tertiary">本地 Agent 失败时禁止回退到云端 LLM</div>
        </div>
        <label className="relative inline-flex cursor-pointer items-center">
          <input
            type="checkbox"
            checked={strictLocalMode}
            onChange={(e) => onChangeStrict(e.target.checked)}
            className="peer sr-only"
          />
          <div className="peer h-6 w-11 rounded-full bg-tertiary after:absolute after:left-[2px] after:top-[2px] after:h-5 after:w-5 after:rounded-full after:bg-primary after:transition-all peer-checked:bg-accent peer-checked:after:translate-x-full" />
        </label>
      </div>

      <div className="flex items-center justify-between px-6 py-3">
        <div className="text-sm text-secondary">请求路由日志</div>
        <Button variant="secondary" size="sm" onClick={openLogs} loading={loading}>
          查看最近 50 条
        </Button>
      </div>

      {open && (
        <div className="border-t border-[var(--border-subtle)] bg-primary/50 px-6 py-4">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-xs text-secondary">时间 / 路由 / 结果</span>
            <button onClick={() => setOpen(false)} className="text-xs text-tertiary hover:text-primary">
              收起
            </button>
          </div>
          {logs.length === 0 ? (
            <div className="text-xs text-tertiary">暂无记录</div>
          ) : (
            <div className="max-h-64 space-y-2 overflow-auto">
              {logs.map((log) => (
                <div key={log.id} className="rounded-md border border-[var(--border-subtle)] p-2 text-xs">
                  <div className="flex items-center gap-2">
                    <span className="text-tertiary">{new Date(log.requested_at * 1000).toLocaleString()}</span>
                    <span
                      className={`rounded px-1.5 py-0.5 ${
                        log.provider.startsWith("acp")
                          ? "bg-success/10 text-success"
                          : log.provider.startsWith("cloud")
                            ? "bg-warning/10 text-warning"
                            : "bg-tertiary/50 text-secondary"
                      }`}
                    >
                      {log.provider}
                    </span>
                    {log.fallback && <span className="text-warning">fallback</span>}
                  </div>
                  {log.error && <div className="mt-1 text-danger">{log.error}</div>}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </Card>
  );
}

function AcpAutoDetect({
  agents,
  detecting,
  existing,
  onDetect,
  onAdd,
}: {
  agents: DetectedAgent[];
  detecting: boolean;
  existing: AcpConnectionConfig[];
  onDetect: () => void;
  onAdd: (agent: DetectedAgent) => void;
}) {
  const isAdded = (agent: DetectedAgent) =>
    existing.some(
      (c) =>
        c.transport.kind === "stdio" &&
        c.transport.command === agent.command &&
        c.transport.args.join(" ") === agent.args.join(" ")
    );

  return (
    <div className="border-b border-[var(--border-subtle)] px-6 py-4">
      <div className="flex items-center justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Search className="h-4 w-4 text-accent" />
            <h4 className="text-sm font-medium text-primary">本地智能体自动检测</h4>
          </div>
          <p className="mt-1 text-xs text-secondary">
            扫描 PATH 中已安装的 Claude Code、Kimi Code、Codex 等 ACP 兼容 CLI。
          </p>
        </div>
        <Button variant="secondary" size="sm" onClick={onDetect} loading={detecting}>
          <Search className="mr-1 h-4 w-4" />
          {detecting ? "扫描中…" : "重新检测"}
        </Button>
      </div>

      {!detecting && agents.length === 0 && (
        <div className="mt-3 rounded-lg border border-dashed border-[var(--border-subtle)] bg-primary/50 px-4 py-3 text-xs text-secondary">
          未检测到本地智能体。请确认已安装 CLI（如 <code>claude</code>、<code>kimi</code>）且已在 PATH 中，或手动添加连接。
        </div>
      )}

      {agents.length > 0 && (
        <div className="mt-3 space-y-2">
          {agents.map((agent) => {
            const added = isAdded(agent);
            return (
              <div
                key={agent.command}
                className="flex items-center justify-between rounded-lg border border-[var(--border-subtle)] bg-primary/50 px-4 py-2"
              >
                <div>
                  <div className="text-sm font-medium text-primary">{agent.display_name}</div>
                  <div className="font-mono text-xs text-tertiary">
                    {agent.command} {agent.args.join(" ")}
                  </div>
                </div>
                {added ? (
                  <span className="text-xs text-secondary">已添加</span>
                ) : (
                  <Button variant="secondary" size="sm" onClick={() => onAdd(agent)}>
                    添加连接
                  </Button>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function ModelRow({
  model,
  isDefaultChat,
  isDefaultEmbedding,
  onChange,
  onTest,
  onRemove,
  onSetDefaultChat,
  onSetDefaultEmbedding,
  testing,
}: {
  model: AiModelConfig;
  isDefaultChat: boolean;
  isDefaultEmbedding: boolean;
  onChange: (patch: Partial<AiModelConfig>) => void;
  onTest: () => void;
  onRemove: () => void;
  onSetDefaultChat: () => void;
  onSetDefaultEmbedding: () => void;
  testing: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasChat = model.capabilities.includes("chat");
  const hasEmbedding = model.capabilities.includes("embedding");

  const toggleCapability = (cap: "chat" | "embedding") => {
    const next = model.capabilities.includes(cap)
      ? model.capabilities.filter((c) => c !== cap)
      : [...model.capabilities, cap];
    onChange({ capabilities: next.length ? next : ["chat"] });
  };

  return (
    <div className="px-6 py-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Server className="h-4 w-4 text-tertiary" />
            <input
              type="text"
              value={model.name}
              onChange={(e) => onChange({ name: e.target.value })}
              placeholder="模型名称"
              className="min-w-[8rem] flex-1 rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1 text-sm text-primary focus:border-accent focus:outline-none"
            />
            {isDefaultChat && hasChat && (
              <span className="shrink-0 rounded-full bg-accent/10 px-2 py-0.5 text-xs font-medium text-accent">
                默认对话
              </span>
            )}
            {isDefaultEmbedding && hasEmbedding && (
              <span className="shrink-0 rounded-full bg-success/10 px-2 py-0.5 text-xs font-medium text-success">
                默认 Embedding
              </span>
            )}
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-tertiary">
            <span>{PROVIDER_LABELS[model.provider] || model.provider}</span>
            <span>模型：{model.model || "—"}</span>
            <span>Base URL：{model.base_url || "默认"}</span>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => setExpanded((v) => !v)}
            className="rounded p-1.5 text-tertiary hover:bg-tertiary hover:text-primary"
          >
            <ChevronDown className={`h-4 w-4 transition-transform ${expanded ? "rotate-180" : ""}`} />
          </button>
          <button
            onClick={onTest}
            disabled={testing || !model.api_key}
            className="rounded p-1.5 text-tertiary hover:bg-tertiary hover:text-primary disabled:opacity-50"
            title="测试连接"
          >
            <FlaskConical className={`h-4 w-4 ${testing ? "animate-pulse" : ""}`} />
          </button>
          <button
            onClick={onRemove}
            className="rounded p-1.5 text-tertiary hover:bg-danger/10 hover:text-danger"
            title="删除"
          >
            <Trash2 className="h-4 w-4" />
          </button>
        </div>
      </div>

      {expanded && (
        <div className="mt-4 grid grid-cols-1 gap-3 rounded-lg border border-[var(--border-prominent)] bg-primary p-4 md:grid-cols-2">
          <div>
            <label className="mb-1 block text-xs text-secondary">服务商</label>
            <select
              value={model.provider}
              onChange={(e) => onChange({ provider: e.target.value })}
              className="w-full rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
            >
              <option value="openai">OpenAI 兼容</option>
              <option value="anthropic">Anthropic</option>
            </select>
          </div>
          <div>
            <label className="mb-1 block text-xs text-secondary">模型 ID</label>
            <input
              type="text"
              value={model.model}
              onChange={(e) => onChange({ model: e.target.value })}
              placeholder="gpt-4o-mini"
              className="w-full rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
            />
          </div>
          <div className="md:col-span-2">
            <label className="mb-1 block text-xs text-secondary">Base URL（可选）</label>
            <input
              type="text"
              value={model.base_url}
              onChange={(e) => onChange({ base_url: e.target.value })}
              placeholder="https://api.openai.com/v1"
              className="w-full rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
            />
          </div>
          <div className="md:col-span-2">
            <label className="mb-1 block flex items-center gap-1 text-xs text-secondary">
              <Key className="h-3 w-3" /> API Key
            </label>
            <input
              type="password"
              value={model.api_key}
              onChange={(e) => onChange({ api_key: e.target.value })}
              placeholder="sk-..."
              className="w-full rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
            />
          </div>
          <div className="md:col-span-2">
            <label className="mb-2 block text-xs text-secondary">能力</label>
            <div className="flex flex-wrap gap-3">
              <label className="flex items-center gap-1.5 text-sm text-secondary">
                <input
                  type="checkbox"
                  checked={hasChat}
                  onChange={() => toggleCapability("chat")}
                  className="accent-accent"
                />
                对话
              </label>
              <label className="flex items-center gap-1.5 text-sm text-secondary">
                <input
                  type="checkbox"
                  checked={hasEmbedding}
                  onChange={() => toggleCapability("embedding")}
                  className="accent-accent"
                />
                Embedding
              </label>
            </div>
          </div>
          <div className="flex flex-wrap gap-2 md:col-span-2">
            {hasChat && (
              <button
                onClick={onSetDefaultChat}
                disabled={isDefaultChat}
                className="flex items-center gap-1 rounded-md bg-accent/10 px-2 py-1 text-xs font-medium text-accent disabled:opacity-50"
              >
                {isDefaultChat && <Check className="h-3 w-3" />}
                {isDefaultChat ? "已设为默认对话" : "设为默认对话"}
              </button>
            )}
            {hasEmbedding && (
              <button
                onClick={onSetDefaultEmbedding}
                disabled={isDefaultEmbedding}
                className="flex items-center gap-1 rounded-md bg-success/10 px-2 py-1 text-xs font-medium text-success disabled:opacity-50"
              >
                {isDefaultEmbedding && <Check className="h-3 w-3" />}
                {isDefaultEmbedding ? "已设为默认 Embedding" : "设为默认 Embedding"}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function AcpRow({
  connection,
  onChange,
  onTransportChange,
  onTest,
  onRemove,
  testing,
}: {
  connection: AcpConnectionConfig;
  onChange: (patch: Partial<AcpConnectionConfig>) => void;
  onTransportChange: (patch: Partial<AcpTransport>) => void;
  onTest: () => void;
  onRemove: () => void;
  testing: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const transport = connection.transport;

  return (
    <div className="px-6 py-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Link className="h-4 w-4 text-tertiary" />
            <input
              type="text"
              value={connection.name}
              onChange={(e) => onChange({ name: e.target.value })}
              placeholder="ACP 连接名称"
              className="min-w-[8rem] flex-1 rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1 text-sm text-primary focus:border-accent focus:outline-none"
            />
            {!connection.enabled && (
              <span className="shrink-0 rounded-full bg-tertiary px-2 py-0.5 text-xs text-secondary">
                已禁用
              </span>
            )}
          </div>
          <div className="mt-2 text-xs text-tertiary">
            {transport.kind === "stdio"
              ? `stdio: ${transport.command} ${transport.args.join(" ")}`
              : `sse: ${transport.url}`}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <label className="relative inline-flex cursor-pointer items-center">
            <input
              type="checkbox"
              checked={connection.enabled}
              onChange={(e) => onChange({ enabled: e.target.checked })}
              className="peer sr-only"
            />
            <div className="peer h-5 w-9 rounded-full bg-tertiary after:absolute after:left-[2px] after:top-[2px] after:h-4 after:w-4 after:rounded-full after:bg-primary after:transition-all peer-checked:bg-accent peer-checked:after:translate-x-full" />
          </label>
          <button
            onClick={() => setExpanded((v) => !v)}
            className="rounded p-1.5 text-tertiary hover:bg-tertiary hover:text-primary"
          >
            <ChevronDown className={`h-4 w-4 transition-transform ${expanded ? "rotate-180" : ""}`} />
          </button>
          <button
            onClick={onTest}
            disabled={testing}
            className="rounded p-1.5 text-tertiary hover:bg-tertiary hover:text-primary disabled:opacity-50"
            title="测试连接"
          >
            <FlaskConical className={`h-4 w-4 ${testing ? "animate-pulse" : ""}`} />
          </button>
          <button
            onClick={onRemove}
            className="rounded p-1.5 text-tertiary hover:bg-danger/10 hover:text-danger"
            title="删除"
          >
            <Trash2 className="h-4 w-4" />
          </button>
        </div>
      </div>

      {expanded && (
        <div className="mt-4 space-y-3 rounded-lg border border-[var(--border-prominent)] bg-primary p-4">
          {transport.kind === "stdio" ? (
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
              <div className="md:col-span-2">
                <label className="mb-1 block text-xs text-secondary">命令</label>
                <input
                  type="text"
                  value={transport.command}
                  onChange={(e) =>
                    onTransportChange({ kind: "stdio", command: e.target.value })
                  }
                  placeholder="例如：claude"
                  className="w-full rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
                />
              </div>
              <div className="md:col-span-2">
                <label className="mb-1 block text-xs text-secondary">参数（空格分隔）</label>
                <input
                  type="text"
                  value={transport.args.join(" ")}
                  onChange={(e) =>
                    onTransportChange({
                      kind: "stdio",
                      args: e.target.value.split(/\s+/).filter(Boolean),
                    })
                  }
                  placeholder="code --mcp"
                  className="w-full rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
                />
              </div>
            </div>
          ) : (
            <div>
              <label className="mb-1 block text-xs text-secondary">SSE URL</label>
              <input
                type="text"
                value={transport.url}
                onChange={(e) => onTransportChange({ kind: "sse", url: e.target.value })}
                placeholder="http://localhost:3000/sse"
                className="w-full rounded-md border border-[var(--border-prominent)] bg-primary px-2 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
              />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
