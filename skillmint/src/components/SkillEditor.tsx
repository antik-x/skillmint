import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "../lib/invoke";
import { Eye, Columns, Code, Save, X, AlertTriangle } from "lucide-react";
import { Skeleton } from "./ui/Skeleton";
import ReactMarkdown from "react-markdown";
import { showError, showInfoWithAction, showSuccess } from "../stores/toastStore";
import { Button } from "./ui/Button";
import { useHotkey, useHotkeyScope } from "../hooks/useHotkeys";
import type { Skill, SkillVersion } from "../types";

interface SkillContent {
  frontmatter: Record<string, unknown>;
  body: string;
}

type EditorMode = "split" | "source" | "preview";

interface SkillEditorProps {
  skillId: string;
  skillName: string;
  onSaved?: (skill: Skill) => void;
  onCancel?: () => void;
}

export default function SkillEditor({ skillId, skillName, onSaved, onCancel }: SkillEditorProps) {
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [mode, setMode] = useState<EditorMode>("split");
  const [frontmatter, setFrontmatter] = useState<Record<string, unknown>>({});
  const [body, setBody] = useState("");
  const [externalChange, setExternalChange] = useState(false);
  const [conflictOpen, setConflictOpen] = useState(false);
  const editorRef = useRef<HTMLDivElement>(null);

  // I3: editor scope makes mod+s save active only when the editor is visible.
  useHotkeyScope("editor");

  // SPEC-F2 T10: poll for external modifications to SKILL.md.
  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      try {
        const changed = await invoke<boolean>("check_skill_external_change", { skillId });
        if (!cancelled) setExternalChange(changed);
      } catch {
        if (!cancelled) setExternalChange(false);
      }
    };
    check();
    const timer = setInterval(check, 5000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [skillId]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    invoke<SkillContent>("read_skill_content", { skillId })
      .then((content) => {
        if (cancelled) return;
        setFrontmatter(content.frontmatter || {});
        setBody(content.body || "");
      })
      .catch((err) => {
        if (cancelled) return;
        const msg = typeof err === "string" ? err : String(err);
        showError(`读取 Skill 失败：${msg}`);
      })
      .finally(() => setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [skillId]);

  const name = useMemo(() => {
    const n = frontmatter.name;
    return typeof n === "string" ? n : skillName;
  }, [frontmatter.name, skillName]);

  const description = useMemo(() => String(frontmatter.description || ""), [frontmatter.description]);

  const stringArray = useCallback((key: string): string[] => {
    const v = frontmatter[key];
    if (Array.isArray(v)) return v.filter((x): x is string => typeof x === "string");
    if (typeof v === "string" && v.trim()) return v.split(/[,，]/).map((s) => s.trim()).filter(Boolean);
    return [];
  }, [frontmatter]);

  const setFrontmatterField = useCallback((key: string, value: unknown) => {
    setFrontmatter((prev) => ({ ...prev, [key]: value }));
  }, []);

  const setStringArray = useCallback((key: string, raw: string) => {
    const arr = raw.split(/[,，]/).map((s) => s.trim()).filter(Boolean);
    setFrontmatterField(key, arr);
  }, [setFrontmatterField]);

  const shouldSuggestVersion = useCallback(async () => {
    try {
      const versions = await invoke<SkillVersion[]>("list_skill_versions_command", { skillId });
      const latestSnapshot = versions.find((v) => v.version !== "latest");
      if (!latestSnapshot) {
        // No historical snapshot yet; any non-trivial save is worth keeping.
        return body.trim().length > 0;
      }
      const content = await invoke<SkillContent>("read_skill_content", {
        skillId,
        version: latestSnapshot.version,
      });
      return content.body.trim() !== body.trim();
    } catch (err) {
      console.error("Failed to check version suggestion:", err);
      return false;
    }
  }, [skillId, body]);

  const suggestVersion = useCallback(() => {
    showInfoWithAction(
      "已保存，要留个版本吗？",
      {
        label: "留版本",
        onClick: async () => {
          try {
            const note = window.prompt("版本备注（可选）");
            if (note === null) return;
            await invoke("save_version", {
              skillId,
              note: note.trim() || undefined,
            });
            showSuccess("已留存版本");
          } catch (err) {
            const msg = typeof err === "string" ? err : String(err);
            showError(`留版本失败：${msg}`);
          }
        },
      },
      6000
    );
  }, [skillId]);

  const handleSave = useCallback(async (force?: boolean) => {
    if (saving) return;
    if (!name.trim()) {
      showError("Skill 名称不能为空");
      return;
    }
    setSaving(true);
    try {
      const updated = await invoke<Skill>("save_skill_content", {
        skillId,
        frontmatter: { ...frontmatter, name: name.trim() },
        body,
        force,
      });
      setConflictOpen(false);
      setExternalChange(false);
      showSuccess("Skill 已保存");
      onSaved?.(updated);

      // G3-②: one-time, non-blocking prompt to keep a version after meaningful edits.
      const suggest = await shouldSuggestVersion();
      if (suggest) {
        suggestVersion();
      }
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      if (typeof msg === "string" && msg.includes("已被外部编辑器修改")) {
        setConflictOpen(true);
      } else {
        showError(`保存失败：${msg}`);
      }
    } finally {
      setSaving(false);
    }
  }, [body, frontmatter, name, onSaved, saving, shouldSuggestVersion, skillId]);

  // I3: Cmd/Ctrl+S saves the current skill when the editor is focused/visible.
  useHotkey("mod+s", () => handleSave(), { scope: "editor", description: "保存当前 Skill" }, [
    handleSave,
  ]);

  const reloadFromDisk = useCallback(async () => {
    setLoading(true);
    try {
      const content = await invoke<SkillContent>("read_skill_content", { skillId });
      setFrontmatter(content.frontmatter || {});
      setBody(content.body || "");
      setExternalChange(false);
      setConflictOpen(false);
      showSuccess("已加载磁盘版本");
    } catch (err) {
      showError(err, { context: "重新加载" });
    } finally {
      setLoading(false);
    }
  }, [skillId]);

  if (loading) {
    return (
      <div className="h-full space-y-4 p-6">
        <Skeleton className="h-8 w-1/3" />
        <Skeleton className="h-4 w-2/3" />
        <div className="space-y-2">
          <Skeleton className="h-3 w-full" />
          <Skeleton className="h-3 w-5/6" />
          <Skeleton className="h-3 w-4/5" />
          <Skeleton className="h-3 w-full" />
          <Skeleton className="h-3 w-3/4" />
        </div>
      </div>
    );
  }

  return (
    <div ref={editorRef} className="flex h-full flex-col" data-editor-scope>
      {externalChange && (
        <div className="mb-3 flex items-center justify-between rounded-lg border border-warning/20 bg-warning/10 px-4 py-2 text-sm text-warning">
          <span className="flex items-center gap-2">
            <AlertTriangle className="h-4 w-4" />
            SKILL.md 已在外部修改
          </span>
          <button onClick={reloadFromDisk} className="font-medium hover:underline">
            重新加载
          </button>
        </div>
      )}

      {conflictOpen && (
        <div className="mb-3 rounded-lg border border-danger/20 bg-danger/5 p-4">
          <div className="text-sm font-medium text-danger">检测到编辑冲突</div>
          <p className="mt-1 text-xs text-secondary">
            SKILL.md 在磁盘上已被修改。你可以选择保留当前编辑版本（覆盖磁盘），或加载磁盘版本（丢失当前编辑）。
          </p>
          <div className="mt-3 flex justify-end gap-2">
            <Button variant="secondary" size="sm" onClick={reloadFromDisk}>
              加载磁盘版本
            </Button>
            <Button variant="danger" size="sm" onClick={() => handleSave(true)}>
              保留我的版本
            </Button>
          </div>
        </div>
      )}

      {/* Toolbar */}
      <div className="mb-3 flex items-center justify-between">
        <div className="flex items-center gap-1 rounded-lg bg-secondary p-1">
          <ModeButton active={mode === "split"} onClick={() => setMode("split")} icon={Columns} label="双栏" />
          <ModeButton active={mode === "source"} onClick={() => setMode("source")} icon={Code} label="源码" />
          <ModeButton active={mode === "preview"} onClick={() => setMode("preview")} icon={Eye} label="预览" />
        </div>
        <div className="flex gap-2">
          {onCancel && (
            <Button variant="ghost" size="sm" onClick={onCancel}>
              <X className="mr-1 h-4 w-4" />
              取消
            </Button>
          )}
          <Button variant="primary" size="sm" onClick={() => handleSave()} loading={saving} disabled={saving}>
            <Save className="mr-1 h-4 w-4" />
            {saving ? "保存中…" : "保存"}
          </Button>
        </div>
      </div>

      {/* Frontmatter form */}
      <div className="mb-4 grid grid-cols-1 gap-3 rounded-xl border border-[var(--border-subtle)] bg-secondary p-4 md:grid-cols-2">
        <Field label="名称">
          <input
            value={name}
            onChange={(e) => setFrontmatterField("name", e.target.value)}
            placeholder="Skill 名称"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
          />
        </Field>
        <Field label="描述">
          <input
            value={description}
            onChange={(e) => setFrontmatterField("description", e.target.value)}
            placeholder="一句话描述"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
          />
        </Field>
        <Field label="概念（逗号分隔）">
          <input
            value={stringArray("concepts").join(", ")}
            onChange={(e) => setStringArray("concepts", e.target.value)}
            placeholder="例如：JWT, Authentication"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
          />
        </Field>
        <Field label="场景（逗号分隔）">
          <input
            value={stringArray("scenarios").join(", ")}
            onChange={(e) => setStringArray("scenarios", e.target.value)}
            placeholder="例如：backend-api, security"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
          />
        </Field>
        <Field label="标签（逗号分隔）">
          <input
            value={stringArray("tags").join(", ")}
            onChange={(e) => setStringArray("tags", e.target.value)}
            placeholder="例如：nodejs, python"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
          />
        </Field>
        <Field label="相关 Skill（逗号分隔）">
          <input
            value={stringArray("related_skills").join(", ")}
            onChange={(e) => setStringArray("related_skills", e.target.value)}
            placeholder="例如：password-hash"
            className="w-full rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-1.5 text-sm text-primary focus:border-accent focus:outline-none"
          />
        </Field>
      </div>

      {/* Editor body */}
      <div className="min-h-0 flex-1 rounded-xl border border-[var(--border-subtle)] bg-secondary p-0 overflow-hidden">
        <div className="flex h-full">
          {(mode === "split" || mode === "source") && (
            <div className={`h-full ${mode === "split" ? "w-1/2 border-r border-[var(--border-subtle)]" : "w-full"}`}>
              <textarea
                value={body}
                onChange={(e) => setBody(e.target.value)}
                spellCheck={false}
                className="h-full w-full resize-none bg-primary p-4 font-mono text-sm text-primary outline-none"
                placeholder="# 标题\n\n在这里写 SKILL.md 正文…"
              />
            </div>
          )}
          {(mode === "split" || mode === "preview") && (
            <div className={`h-full overflow-auto ${mode === "split" ? "w-1/2" : "w-full"}`}>
              <div className="max-w-none p-4 text-sm text-primary">
                <ReactMarkdown
                  components={{
                    h1: ({ children }) => <h1 className="mb-3 text-2xl font-bold">{children}</h1>,
                    h2: ({ children }) => <h2 className="mb-2 mt-4 text-xl font-semibold">{children}</h2>,
                    h3: ({ children }) => <h3 className="mb-2 mt-3 text-lg font-semibold">{children}</h3>,
                    p: ({ children }) => <p className="mb-3 leading-relaxed">{children}</p>,
                    ul: ({ children }) => <ul className="mb-3 list-inside list-disc space-y-1">{children}</ul>,
                    ol: ({ children }) => <ol className="mb-3 list-inside list-decimal space-y-1">{children}</ol>,
                    li: ({ children }) => <li>{children}</li>,
                    code: ({ children }) => <code className="rounded bg-tertiary/50 px-1 py-0.5 font-mono text-xs">{children}</code>,
                    pre: ({ children }) => <pre className="mb-3 overflow-auto rounded-lg bg-tertiary/50 p-3 font-mono text-xs">{children}</pre>,
                    blockquote: ({ children }) => <blockquote className="mb-3 border-l-2 border-accent/50 pl-3 text-secondary">{children}</blockquote>,
                    a: ({ children, href }) => {
                      const safe = href?.toLowerCase().startsWith("javascript:") ? "#" : href;
                      return <a href={safe} className="text-accent underline" target="_blank" rel="noreferrer">{children}</a>;
                    },
                  }}
                >
                  {body || "*预览区域*"}
                </ReactMarkdown>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ModeButton({
  active,
  onClick,
  icon: Icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-1 rounded-md px-2.5 py-1 text-xs transition-colors ${
        active ? "bg-accent font-medium text-primary" : "text-secondary hover:text-primary"
      }`}
    >
      <Icon className="h-3.5 w-3.5" />
      {label}
    </button>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label>
      <span className="mb-1 block text-xs font-medium text-secondary">{label}</span>
      {children}
    </label>
  );
}
