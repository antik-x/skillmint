import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { MoreHorizontal, Plus, Save, Tag, Undo, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import { showError, showInfoWithAction, showSuccess } from "../stores/toastStore";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";
import { EmptyState } from "./ui/EmptyState";
import { Input } from "./ui/Input";
import type { RollbackResult, SkillVersion } from "../types";

interface VersionPanelProps {
  skillId: string;
}

interface PreviewState {
  version: string;
  body: string;
}

export default function VersionPanel({ skillId }: VersionPanelProps) {
  const [versions, setVersions] = useState<SkillVersion[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [rollingBack, setRollingBack] = useState<string | null>(null);
  const [noteVersion, setNoteVersion] = useState<string | null>(null);
  const [noteDraft, setNoteDraft] = useState("");
  const [preview, setPreview] = useState<PreviewState | null>(null);
  const [openMenu, setOpenMenu] = useState<string | null>(null);

  const load = useCallback(async (signal?: AbortSignal) => {
    try {
      const list = await invoke<SkillVersion[]>("list_skill_versions_command", { skillId });
      if (signal?.aborted) return;
      setVersions(list);
    } catch (err) {
      if (signal?.aborted) return;
      const msg = typeof err === "string" ? err : String(err);
      showError(`加载版本失败：${msg}`);
    } finally {
      if (!signal?.aborted) {
        setLoading(false);
      }
    }
  }, [skillId]);

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    load(controller.signal);
    return () => {
      controller.abort();
    };
  }, [load]);

  const hasHistory = useMemo(
    () => versions.some((v) => v.version !== "latest"),
    [versions]
  );

  const handleSaveVersion = useCallback(async () => {
    if (saving) return;
    const note = window.prompt("版本备注（可选）");
    if (note === null) return; // cancelled
    setSaving(true);
    try {
      const label = await invoke<string>("save_version", {
        skillId,
        note: note.trim() || undefined,
      });
      showSuccess(`已留存为 ${label}`);
      await load();
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`留版本失败：${msg}`);
    } finally {
      setSaving(false);
    }
  }, [skillId, saving, load]);

  const handleRollback = useCallback(
    async (version: string) => {
      if (rollingBack) return;
      const confirmed = window.confirm(
        `将 latest 回退到 ${version} 的内容？\n当前 latest 会先留存为新版本，可再次回滚。`
      );
      if (!confirmed) return;
      setRollingBack(version);
      try {
        const res = await invoke<RollbackResult>("rollback_to_version", {
          skillId,
          targetVersion: version,
        });
        showSuccess(`已回滚到 ${res.rolled_to}（原 latest 存为 ${res.saved_as}）`);
        await load();
      } catch (err) {
        const msg = typeof err === "string" ? err : String(err);
        showError(`回滚失败：${msg}`);
      } finally {
        setRollingBack(null);
        setOpenMenu(null);
      }
    },
    [skillId, rollingBack, load]
  );

  const startEditNote = useCallback((version: string, current?: string) => {
    setNoteVersion(version);
    setNoteDraft(current ?? "");
    setOpenMenu(null);
  }, []);

  const MAX_NOTE_LENGTH = 200;

  const saveNote = useCallback(async () => {
    if (!noteVersion) return;
    const trimmed = noteDraft.trim();
    if (trimmed.length > MAX_NOTE_LENGTH) {
      showError(`备注不能超过 ${MAX_NOTE_LENGTH} 个字符`);
      return;
    }
    try {
      await invoke("set_version_note", {
        skillId,
        version: noteVersion,
        note: trimmed || undefined,
      });
      showSuccess("备注已更新");
      setNoteVersion(null);
      await load();
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      showError(`更新备注失败：${msg}`);
    }
  }, [noteVersion, noteDraft, skillId, load]);

  const previewVersion = useCallback(
    async (version: string) => {
      try {
        const content = await invoke<{ body: string }>("read_skill_content", {
          skillId,
          version: version === "latest" ? undefined : version,
        });
        setPreview({ version, body: content.body || "*无内容*" });
        setOpenMenu(null);
      } catch (err) {
        const msg = typeof err === "string" ? err : String(err);
        showError(`预览失败：${msg}`);
      }
    },
    [skillId]
  );

  const formatTime = (ts: number) => {
    const d = new Date(ts * 1000);
    const now = new Date();
    const diff = Math.floor((now.getTime() - d.getTime()) / 1000);
    if (diff < 60) return "刚刚";
    if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
    if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
    if (diff < 604800) return `${Math.floor(diff / 86400)} 天前`;
    return d.toLocaleDateString("zh-CN");
  };

  if (loading) {
    return (
      <Card className="h-full" padding="lg">
        <div className="text-sm text-secondary">加载版本…</div>
      </Card>
    );
  }

  return (
    <>
      <Card className="h-full" padding="none">
        <div className="flex items-center justify-between border-b border-[var(--divider)] px-5 py-3">
          <h3 className="text-sm font-semibold text-primary">版本</h3>
          <Button variant="secondary" size="sm" onClick={handleSaveVersion} loading={saving}>
            <Plus className="h-3.5 w-3.5" />
            留版本
          </Button>
        </div>

        {!hasHistory ? (
          <div className="p-5">
            <EmptyState
              icon={Tag}
              title="还没有历史版本"
              description="留一个版本，以后就能随时回退。"
              action={
                <Button variant="secondary" size="sm" onClick={handleSaveVersion} loading={saving}>
                  <Plus className="h-3.5 w-3.5" />
                  留版本
                </Button>
              }
            />
          </div>
        ) : (
          <ul className="divide-y divide-[var(--divider)]">
            {versions.map((v) => (
              <li
                key={v.version}
                className="group flex items-start justify-between px-5 py-3 hover:bg-tertiary/40"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-primary">
                      {v.version === "latest" ? "latest（当前）" : v.version}
                    </span>
                    {v.note && (
                      <span className="truncate text-xs text-secondary">「{v.note}」</span>
                    )}
                  </div>
                  <div className="mt-0.5 text-xs text-tertiary">{formatTime(v.created_at)}</div>
                  {v.pinned_by && v.pinned_by.length > 0 && (
                    <div className="mt-1 text-xs text-accent">
                      被 {v.pinned_by.join("、")} 固定
                    </div>
                  )}
                </div>

                {v.version !== "latest" && (
                  <div className="relative ml-3 shrink-0">
                    <button
                      onClick={() => setOpenMenu(openMenu === v.version ? null : v.version)}
                      className="rounded-md p-1.5 text-secondary hover:bg-tertiary/60 hover:text-primary"
                      aria-label="操作"
                    >
                      <MoreHorizontal className="h-4 w-4" />
                    </button>
                    {openMenu === v.version && (
                      <div className="absolute right-0 top-full z-20 mt-1 w-32 rounded-lg border border-[var(--border-subtle)] bg-primary py-1 shadow-lg">
                        <button
                          onClick={() => previewVersion(v.version)}
                          className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-secondary hover:bg-tertiary/60 hover:text-primary"
                        >
                          <Tag className="h-3.5 w-3.5" />
                          预览
                        </button>
                        <button
                          onClick={() => startEditNote(v.version, v.note)}
                          className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-secondary hover:bg-tertiary/60 hover:text-primary"
                        >
                          <Save className="h-3.5 w-3.5" />
                          编辑备注
                        </button>
                        <button
                          onClick={() => handleRollback(v.version)}
                          disabled={rollingBack === v.version}
                          className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-secondary hover:bg-tertiary/60 hover:text-primary disabled:opacity-50"
                        >
                          <Undo className="h-3.5 w-3.5" />
                          {rollingBack === v.version ? "回滚中…" : "回滚到此版"}
                        </button>
                      </div>
                    )}
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>

      {/* Inline note editor modal */}
      {noteVersion && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <Card className="w-full max-w-sm" padding="lg">
            <div className="mb-3 flex items-center justify-between">
              <h3 className="text-sm font-semibold text-primary">
                编辑 {noteVersion} 备注
              </h3>
              <button
                onClick={() => setNoteVersion(null)}
                className="rounded-md p-1 text-tertiary hover:bg-tertiary/60 hover:text-primary"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <Input
              value={noteDraft}
              onChange={(e) => setNoteDraft(e.target.value)}
              placeholder="版本备注（可选）"
              className="mb-4"
            />
            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={() => setNoteVersion(null)}>
                取消
              </Button>
              <Button variant="primary" size="sm" onClick={saveNote}>
                保存
              </Button>
            </div>
          </Card>
        </div>
      )}

      {/* Preview modal */}
      {preview && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <Card className="flex h-[80vh] w-full max-w-2xl flex-col" padding="none">
            <div className="flex items-center justify-between border-b border-[var(--divider)] px-5 py-3">
              <h3 className="text-sm font-semibold text-primary">
                {preview.version === "latest" ? "latest 预览" : `${preview.version} 预览`}
              </h3>
              <button
                onClick={() => setPreview(null)}
                className="rounded-md p-1 text-tertiary hover:bg-tertiary/60 hover:text-primary"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="flex-1 overflow-auto p-5">
              <div className="prose prose-sm max-w-none text-sm text-primary">
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
                  {preview.body}
                </ReactMarkdown>
              </div>
            </div>
          </Card>
        </div>
      )}
    </>
  );
}

export function useVersionToast(skillId: string, onVersionSaved?: () => void) {
  return useCallback(
    (onSave?: () => void) => {
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
              onSave?.();
              onVersionSaved?.();
            } catch (err) {
              const msg = typeof err === "string" ? err : String(err);
              showError(`留版本失败：${msg}`);
            }
          },
        },
        6000
      );
    },
    [skillId, onVersionSaved]
  );
}
