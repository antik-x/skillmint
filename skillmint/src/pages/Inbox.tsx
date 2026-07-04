import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Inbox as InboxIcon } from "lucide-react";
import { useAppStore } from "../stores/appStore";
import { useDiscoveryDecision } from "../hooks/useDiscoveryDecision";
import { useHotkey, useHotkeyScope } from "../hooks/useHotkeys";
import { EmptyState } from "../components/ui/EmptyState";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { humanizeError, showError, showInfo, showSuccess } from "../stores/toastStore";
import type {
  Discovery,
  DiscoveryEvidence,
  DiscoveryKind,
  DismissReason,
  GateRejection,
  SyncAllResult,
  SyncFailure,
} from "../types";

const EXPIRE_DAYS = 7;
const ACCEPT_ANIMATION_MS = 250;

// SPEC-C2 T1: dismiss reason labels for the inline reason picker.
const REASON_OPTIONS: { value: DismissReason; label: string; key: string }[] = [
  { value: "wrong", label: "内容不对", key: "1" },
  { value: "trivial", label: "太琐碎", key: "2" },
  { value: "duplicate", label: "重复了已有 Skill", key: "3" },
];

function discoveryKindLabel(kind: DiscoveryKind): string {
  switch (kind) {
    case "repeat_pattern":
      return "重复模式";
    case "high_value_prompt":
      return "高价值 Prompt";
    case "skill_feedback":
      return "Skill 效果反馈";
    case "capability_gap":
      return "能力缺口";
    default:
      return "发现";
  }
}

function formatEvidenceTime(ts?: number): string {
  if (!ts) return "--";
  const d = new Date(ts * 1000);
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const min = String(d.getMinutes()).padStart(2, "0");
  return `${mm}-${dd} ${hh}:${min}`;
}

function evidenceSourceLine(e: DiscoveryEvidence): string {
  const parts = [formatEvidenceTime(e.started_at), e.source, e.project].filter(Boolean);
  return parts.join(" · ");
}

function formatExpireDays(minCreatedAt: number): string {
  const deadline = minCreatedAt * 1000 + EXPIRE_DAYS * 24 * 60 * 60 * 1000;
  const remaining = Math.ceil((deadline - Date.now()) / (24 * 60 * 60 * 1000));
  if (remaining <= 0) return "即将过期";
  return `过期还剩 ${remaining} 天`;
}

function prefersReducedMotion(): boolean {
  if (typeof window === "undefined") return false;
  return window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false;
}

function kindSummary(discovery: Discovery): string {
  const stats = discovery.payload?.stats;
  if (stats && Object.keys(stats).length > 0) {
    const [[key, value]] = Object.entries(stats);
    return `${key} ${value}`;
  }
  return `置信 ${discovery.confidence.toFixed(2)}`;
}

/** SPEC-C2 T4: detect whether the discovery was LLM-enhanced (AI score) or rule-based. */
function isAiScored(discovery: Discovery): boolean {
  // The LLM enhance step writes an `enhanced_by` or `llm_enhanced` marker into
  // the payload. Check both for resilience.
  const p = discovery.payload as Record<string, unknown>;
  return p?.enhanced_by === "llm" || p?.llm_enhanced === true;
}

type SyncBanner =
  | { kind: "none" }
  | {
      kind: "partial";
      skillId: string;
      failed: number;
      success: number;
      failures: SyncFailure[];
      retrying: boolean;
    }
  | { kind: "resolved" };

export default function Inbox() {
  const setActiveTab = useAppStore((state) => state.setActiveTab);
  const setSkillLibrarySubTab = useAppStore((state) => state.setSkillLibrarySubTab);
  const requestNewSkill = useAppStore((state) => state.requestNewSkill);
  const setDiscoverSearchTerm = useAppStore((state) => state.setDiscoverSearchTerm);
  const navigateToSkillEditor = useAppStore((state) => state.navigateToSkillEditor);
  const bumpInboxRefresh = useAppStore((state) => state.bumpInboxRefresh);

  const { decide, loading: deciding } = useDiscoveryDecision();

  const [items, setItems] = useState<Discovery[] | null>(null);
  const [currentIndex, setCurrentIndex] = useState(0);
  const [expanded, setExpanded] = useState(false);
  const [statsExpanded, setStatsExpanded] = useState(false);
  const [animating, setAnimating] = useState<"accept" | null>(null);
  const [decisions, setDecisions] = useState({ accepted: 0, acceptedEdited: 0, dismissed: 0 });
  // SPEC-C2 T1: dismiss reason inline picker state.
  const [dismissOpen, setDismissOpen] = useState(false);
  // SPEC-C2 T2: partial_synced banner state.
  const [syncBanner, setSyncBanner] = useState<SyncBanner>({ kind: "none" });
  // SPEC-C2 T4: low-confidence tab.
  const [activeTab, setActiveTabInbox] = useState<"pending" | "lowConfidence">("pending");
  const [lowConfItems, setLowConfItems] = useState<GateRejection[] | null>(null);

  useHotkeyScope("inbox");

  // Load pending discoveries once on mount.
  useEffect(() => {
    let cancelled = false;
    invoke<Discovery[]>("list_discoveries", { status: "pending" })
      .then((rows) => {
        if (cancelled) return;
        const pending = rows
          .filter((r) => r.status === "pending")
          .sort((a, b) => a.created_at - b.created_at);
        setItems(pending);
        setCurrentIndex(0);
      })
      .catch(() => {
        if (!cancelled) setItems([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // SPEC-C2 T4: load low-confidence gate rejections when the tab is selected.
  useEffect(() => {
    if (activeTab !== "lowConfidence") return;
    let cancelled = false;
    invoke<GateRejection[]>("list_gate_rejections", { reason: "below_threshold", limit: 50 })
      .then((rows) => {
        if (!cancelled) setLowConfItems(rows);
      })
      .catch(() => {
        if (!cancelled) setLowConfItems([]);
      });
    return () => {
      cancelled = true;
    };
  }, [activeTab]);

  // Keep the cursor in range when the list shrinks or grows.
  useEffect(() => {
    if (items && currentIndex >= items.length) {
      setCurrentIndex(Math.max(0, items.length - 1));
    }
  }, [items, currentIndex]);

  const current = useMemo(() => {
    if (!items || items.length === 0) return null;
    return items[Math.min(currentIndex, items.length - 1)];
  }, [items, currentIndex]);

  const total = items?.length ?? 0;

  const minCreatedAt = useMemo(() => {
    if (!items || items.length === 0) return null;
    return Math.min(...items.map((d) => d.created_at));
  }, [items]);

  const goToday = useCallback(() => setActiveTab("today"), [setActiveTab]);

  const goDiscover = useCallback(
    (term: string) => {
      setDiscoverSearchTerm(term);
      setActiveTab("discover");
    },
    [setActiveTab, setDiscoverSearchTerm]
  );

  const openNewSkillDialog = useCallback(() => {
    setActiveTab("skillLibrary");
    setSkillLibrarySubTab("skills");
    requestNewSkill();
  }, [requestNewSkill, setActiveTab, setSkillLibrarySubTab]);

  // Remove a discovery from the local list (shared by all decision handlers).
  const removeFromList = useCallback(
    (id: string) => {
      setItems((prev) => prev?.filter((d) => d.id !== id) ?? null);
      setDismissOpen(false);
      setSyncBanner({ kind: "none" });
      bumpInboxRefresh();
    },
    [bumpInboxRefresh]
  );

  // SPEC-C2 T1: dismiss with reason. The card leaves immediately (optimistic);
  // on backend error it is re-inserted so the user can retry.
  const handleDismiss = useCallback(
    async (discovery: Discovery, reason?: DismissReason) => {
      const idx = items?.findIndex((d) => d.id === discovery.id) ?? -1;
      removeFromList(discovery.id);
      setDecisions((prev) => ({ ...prev, dismissed: prev.dismissed + 1 }));
      try {
        await decide(discovery, "dismiss", reason);
        showSuccess("已拒绝");
      } catch (err) {
        // Rollback: re-insert at the original position.
        if (idx >= 0) {
          setItems((prev) => {
            const copy = [...(prev ?? [])];
            copy.splice(idx, 0, discovery);
            return copy;
          });
        }
        setDecisions((prev) => ({ ...prev, dismissed: prev.dismissed - 1 }));
        bumpInboxRefresh();
        showError(humanizeError(err, { context: "拒绝发现" }));
      }
    },
    [items, decide, removeFromList, bumpInboxRefresh]
  );

  // SPEC-C2 T1: accept_edited — create a draft and navigate to the editor.
  const handleAcceptEdited = useCallback(
    async (discovery: Discovery) => {
      if (animating) return;
      setAnimating("accept");
      try {
        const result = await decide(discovery, "accept_edited");
        if (result.mock) {
          showInfo("mock 环境：修改后采纳已降级为划掉");
          setAnimating(null);
          await handleDismiss(discovery);
          return;
        }
        if (!result.created_skill_id) {
          setAnimating(null);
          showError("创建草稿失败：未返回 Skill ID");
          return;
        }
        setDecisions((prev) => ({ ...prev, acceptedEdited: prev.acceptedEdited + 1 }));
        removeFromList(discovery.id);
        setAnimating(null);
        navigateToSkillEditor(result.created_skill_id!);
      } catch (err) {
        setAnimating(null);
        showError(humanizeError(err, { context: "修改后采纳" }));
      }
    },
    [animating, decide, handleDismiss, removeFromList, navigateToSkillEditor]
  );

  // SPEC-C2 T1+T2: accept (adopt-and-sync). On partial failure the card stays
  // and shows a recovery banner.
  const handleAccept = useCallback(
    async (discovery: Discovery) => {
      if (animating) return;
      setAnimating("accept");
      try {
        const result = await decide(discovery, "accept");
        if (result.mock) {
          showInfo("mock 环境：采纳并同步已降级为划掉");
          setAnimating(null);
          await handleDismiss(discovery);
          return;
        }
        if (!result.created_skill_id) {
          setAnimating(null);
          showError("沉淀失败：未返回 Skill ID");
          return;
        }
        // SPEC-C2 T2: check sync_summary for partial failures.
        const summary = result.sync_summary;
        if (summary && summary.failed > 0) {
          // Card stays; show recovery banner.
          setAnimating(null);
          setSyncBanner({
            kind: "partial",
            skillId: result.created_skill_id,
            failed: summary.failed,
            success: summary.success,
            failures: summary.failures,
            retrying: false,
          });
          showInfo(`已入库，${summary.failed} 个 Agent 同步失败`);
          return;
        }
        // Full success: toast + sediment animation + leave.
        const agentCount = summary?.success ?? 0;
        showSuccess(agentCount > 0 ? `已入库并同步到 ${agentCount} 个 Agent` : "已入库");
        setDecisions((prev) => ({ ...prev, accepted: prev.accepted + 1 }));
        setTimeout(() => {
          removeFromList(discovery.id);
          setAnimating(null);
        }, prefersReducedMotion() ? 0 : ACCEPT_ANIMATION_MS);
      } catch (err) {
        setAnimating(null);
        showError(humanizeError(err, { context: "采纳并同步" }));
      }
    },
    [animating, decide, handleDismiss, removeFromList]
  );

  // SPEC-C2 T2: retry sync for the partially-synced skill.
  const handleRetrySync = useCallback(async () => {
    if (syncBanner.kind !== "partial") return;
    setSyncBanner({ ...syncBanner, retrying: true });
    try {
      const result = await invoke<SyncAllResult>("sync_single_skill_command", {
        skillId: syncBanner.skillId,
      });
      if (result.failure_count === 0) {
        showSuccess("同步成功");
        setSyncBanner({ kind: "resolved" });
        // Leave the card after a brief confirmation.
        setTimeout(() => {
          if (current) removeFromList(current.id);
          setSyncBanner({ kind: "none" });
        }, 800);
      } else {
        setSyncBanner({
          ...syncBanner,
          retrying: false,
          failed: result.failure_count,
          success: result.success_count,
          failures: result.failures,
        });
        showError(`仍有 ${result.failure_count} 个 Agent 同步失败`);
      }
    } catch (err) {
      setSyncBanner({ ...syncBanner, retrying: false });
      showError(humanizeError(err, { context: "重试同步" }));
    }
  }, [syncBanner, current, removeFromList]);

  // SPEC-C2 T2: ignore the partial failure (card leaves, skill stays).
  const handleIgnorePartial = useCallback(() => {
    if (!current) return;
    setDecisions((prev) => ({ ...prev, accepted: prev.accepted + 1 }));
    removeFromList(current.id);
    setSyncBanner({ kind: "none" });
    showSuccess("已入库，可稍后在 Skill 详情同步");
  }, [current, removeFromList]);

  const handlePrimary = useCallback(() => {
    if (!current) return;
    switch (current.kind) {
      case "repeat_pattern":
      case "high_value_prompt":
        handleAccept(current);
        break;
      case "skill_feedback":
        setStatsExpanded((v) => !v);
        break;
      case "capability_gap":
        goDiscover(current.payload?.note || current.title);
        break;
    }
  }, [current, handleAccept, goDiscover]);

  const hasThreeChoices =
    current?.kind === "repeat_pattern" || current?.kind === "high_value_prompt";

  // SPEC-C2 T3: keyboard three-way — A/E/R + reason number keys + Esc layering.
  useHotkey("a", () => { if (current && hasThreeChoices) handleAccept(current); },
    { scope: "inbox", description: "采纳并同步" }, [current, hasThreeChoices, handleAccept]);
  useHotkey("e", () => { if (current && hasThreeChoices) handleAcceptEdited(current); },
    { scope: "inbox", description: "修改后采纳" }, [current, hasThreeChoices, handleAcceptEdited]);
  useHotkey("r", () => { if (hasThreeChoices) setDismissOpen((v) => !v); },
    { scope: "inbox", description: "拒绝（展开原因）" }, [hasThreeChoices]);

  // Reason number keys (only when the picker is open).
  useHotkey("1", () => { if (dismissOpen && current) handleDismiss(current, "wrong"); },
    { scope: "inbox", description: "拒绝原因：内容不对" }, [dismissOpen, current, handleDismiss]);
  useHotkey("2", () => { if (dismissOpen && current) handleDismiss(current, "trivial"); },
    { scope: "inbox", description: "拒绝原因：太琐碎" }, [dismissOpen, current, handleDismiss]);
  useHotkey("3", () => { if (dismissOpen && current) handleDismiss(current, "duplicate"); },
    { scope: "inbox", description: "拒绝原因：重复" }, [dismissOpen, current, handleDismiss]);

  // Existing navigation keys.
  useHotkey("ArrowLeft", () => {
    setCurrentIndex((i) => Math.max(0, i - 1));
    setExpanded(false);
    setStatsExpanded(false);
    setDismissOpen(false);
  }, { scope: "inbox" });

  useHotkey("ArrowRight", () => {
    setCurrentIndex((i) => Math.min(total - 1, i + 1));
    setExpanded(false);
    setStatsExpanded(false);
    setDismissOpen(false);
  }, { scope: "inbox", description: "下一张发现" }, [total]);

  useHotkey("Enter", handlePrimary, { scope: "inbox", description: "采纳并同步" }, [handlePrimary]);

  // SPEC-C2 T3: Backspace = reject (expand reason picker), muscle-memory mapping.
  useHotkey("Backspace", () => {
    if (hasThreeChoices) setDismissOpen(true);
    else if (current) handleDismiss(current);
  }, { scope: "inbox", description: "拒绝（展开原因）" }, [current, hasThreeChoices, handleDismiss]);

  useHotkey(" ", () => { if (current) setExpanded((v) => !v); },
    { scope: "inbox", description: "展开/收起证据" }, [current]);

  // SPEC-C2 T3: Esc layering — first close the reason picker, then leave page.
  useHotkey("Escape", () => {
    if (dismissOpen) { setDismissOpen(false); return; }
    goToday();
  }, { scope: "inbox", description: "收起原因 / 返回今天页" }, [dismissOpen, goToday]);

  if (items === null) {
    return (
      <div className="flex h-full items-center justify-center p-8">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-amber border-t-transparent" />
      </div>
    );
  }

  // SPEC-C2 T4: low-confidence tab content.
  if (activeTab === "lowConfidence") {
    return (
      <div className="flex h-full flex-col items-center overflow-auto p-7">
        <div className="mb-4 flex w-full max-w-xl gap-2">
          <TabButton active={false} onClick={() => setActiveTabInbox("pending")} label="待确认" />
          <TabButton active={true} onClick={() => {}} label="低置信" />
        </div>
        {lowConfItems === null ? (
          <div className="text-sm text-secondary">加载中…</div>
        ) : lowConfItems.length === 0 ? (
          <EmptyState icon={InboxIcon} title="没有被闸门保留的低置信候选"
            description="置信度在 0.4–0.6 之间的候选会出现在这里，供你回溯。" />
        ) : (
          <div className="w-full max-w-xl space-y-2">
            {lowConfItems.map((r) => (
              <Card key={r.id} className="p-4">
                <div className="mb-1 flex items-center gap-2 text-2xs font-mono text-tertiary">
                  <span>{discoveryKindLabel(r.kind as DiscoveryKind)}</span>
                  <span className="ml-auto">置信 {r.confidence.toFixed(2)}</span>
                </div>
                <div className="text-sm text-primary">
                  {(r.payload as Record<string, unknown>)?.title as string ?? "（无标题）"}
                </div>
                <details className="mt-2">
                  <summary className="cursor-pointer text-2xs text-accent hover:underline">查看内容</summary>
                  <pre className="mt-1 max-h-40 overflow-auto rounded-md bg-secondary p-2 text-2xs text-secondary">
                    {JSON.stringify(r.payload, null, 2)}
                  </pre>
                </details>
              </Card>
            ))}
          </div>
        )}
        <div className="mt-5">
          <Button variant="secondary" size="sm" onClick={() => setActiveTabInbox("pending")}>
            返回待确认
          </Button>
        </div>
      </div>
    );
  }

  if (items.length === 0) {
    const hasHistory =
      decisions.accepted > 0 || decisions.acceptedEdited > 0 || decisions.dismissed > 0;
    return (
      <div className="flex h-full flex-col items-center justify-center p-7">
        {/* SPEC-C2 T4: tab switcher even on empty state. */}
        <div className="mb-4 flex gap-2">
          <TabButton active={true} onClick={() => {}} label="待确认" />
          <TabButton active={false} onClick={() => setActiveTabInbox("lowConfidence")} label="低置信" />
        </div>
        {hasHistory ? (
          <Card className="max-w-md p-8 text-center">
            <div className="mb-3 text-2xl">✓</div>
            <h2 className="text-lg font-semibold text-primary">今日裁决完成</h2>
            <p className="mt-2 text-sm text-secondary" data-testid="inbox-completion-stats">
              沉淀 <span className="font-mono text-amber">{decisions.accepted}</span> · 修改采纳{" "}
              <span className="font-mono text-accent">{decisions.acceptedEdited}</span> · 拒绝{" "}
              <span className="font-mono text-secondary">{decisions.dismissed}</span>
            </p>
            <div className="mt-5">
              <Button variant="secondary" size="sm" onClick={goToday}>回今天页</Button>
            </div>
          </Card>
        ) : (
          <EmptyState
            icon={InboxIcon}
            illustration="inbox"
            title="收件箱"
            description="这里以后会放 AI 帮你挑出的每日发现：值得复用的提问方式、你反复在做的事、Skill 的实际效果。每天采集完成后自动送达，你只需决定收下或忽略。"
            action={<Button variant="secondary" size="sm" onClick={goToday}>回今天页</Button>}
          />
        )}
      </div>
    );
  }

  const evidence = current?.payload?.evidence ?? [];
  const visibleEvidence = expanded ? evidence : evidence.slice(0, 2);
  const hiddenCount = Math.max(0, evidence.length - 2);
  const draft = current?.payload?.draft_skill;
  const stats = current?.payload?.stats;
  const isPartial = syncBanner.kind === "partial";

  return (
    <div className="flex h-full flex-col items-center overflow-auto p-7">
      {/* SPEC-C2 T4: tab switcher. */}
      <div className="mb-3.5 flex w-full max-w-xl gap-2">
        <TabButton active={true} onClick={() => {}} label={`待确认（${total}）`} />
        <TabButton active={false} onClick={() => setActiveTabInbox("lowConfidence")} label="低置信" />
      </div>

      {/* Meta row */}
      <div className="mb-3.5 flex w-full max-w-xl items-center font-mono text-2xs text-tertiary">
        <span>收件箱 · {currentIndex + 1} / {total}</span>
        {minCreatedAt && <span className="ml-4">{formatExpireDays(minCreatedAt)}</span>}
        <div className="ml-auto flex gap-1.5">
          {Array.from({ length: total }).map((_, i) => (
            <span
              key={i}
              className={`block h-1.5 w-1.5 rounded-full ${
                i < currentIndex
                  ? "bg-tertiary"
                  : i === currentIndex
                    ? "bg-amber"
                    : "border border-tertiary bg-transparent"
              }`}
            />
          ))}
        </div>
      </div>

      {/* Big card */}
      <Card
        className={`relative w-full max-w-xl border-l-[3px] border-l-amber p-6 shadow-[0_0_0_1px_var(--amber-glow),0_8px_40px_rgba(0,0,0,0.4)] transition-all duration-[250ms] ${
          animating === "accept" && !isPartial
            ? "scale-95 opacity-0"
            : "scale-100 opacity-100"
        }`}
      >
        {/* SPEC-C2 T2: partial_synced recovery banner. */}
        {syncBanner.kind === "partial" && (
          <div className="mb-4 rounded-lg border border-warning/30 bg-warning/10 p-3" data-testid="partial-sync-banner">
            <div className="flex items-center gap-2 text-sm text-warning">
              <span className="font-medium">
                已入库，{syncBanner.failed} 个 Agent 同步失败
              </span>
            </div>
            {syncBanner.failures[0]?.recovery_hint && (
              <p className="mt-1 text-xs text-secondary">
                {syncBanner.failures[0].agent_name ?? syncBanner.failures[0].agent_id}：{syncBanner.failures[0].recovery_hint}
              </p>
            )}
            <div className="mt-2 flex gap-2">
              <Button
                variant="primary"
                size="sm"
                onClick={handleRetrySync}
                loading={syncBanner.retrying}
                disabled={syncBanner.retrying}
              >
                重试同步
              </Button>
              <Button variant="ghost" size="sm" onClick={handleIgnorePartial}>
                忽略
              </Button>
            </div>
          </div>
        )}
        {syncBanner.kind === "resolved" && (
          <div className="mb-4 rounded-lg border border-success/30 bg-success/10 p-3 text-sm text-success">
            同步成功
          </div>
        )}

        {/* Eyebrow */}
        <div className="mb-1.5 flex items-center gap-2 text-2xs font-mono uppercase tracking-wider text-amber">
          <span>◆ {current ? discoveryKindLabel(current.kind) : ""}</span>
          {/* SPEC-C2 T4: score-source badge. */}
          {current && (
            <span className="rounded bg-tertiary/50 px-1.5 py-0.5 normal-case tracking-normal text-2xs text-secondary">
              {isAiScored(current) ? "AI 评分" : "规则评分"}
            </span>
          )}
          <span className="ml-auto text-tertiary normal-case tracking-normal">
            {current ? kindSummary(current) : ""}
          </span>
        </div>

        {/* Title */}
        <h2 className="text-lg font-bold leading-snug text-primary">{current?.title}</h2>

        {/* Evidence */}
        <div className="mt-4 space-y-2">
          {visibleEvidence.map((e, idx) => (
            <div
              key={idx}
              className="rounded-lg border border-[var(--border-subtle)] bg-primary px-3.5 py-2.5"
            >
              <div className="mb-1 font-mono text-2xs text-tertiary">$ {evidenceSourceLine(e)}</div>
              <div className="font-mono text-xs leading-relaxed text-secondary">
                {e.prompt_text ?? "相关会话"}
              </div>
            </div>
          ))}
        </div>

        {hiddenCount > 0 && !expanded && (
          <button
            onClick={() => setExpanded(true)}
            className="mt-2 font-mono text-2xs text-accent hover:underline"
          >
            空格 展开其余 {hiddenCount} 条证据 ▾
          </button>
        )}
        {expanded && evidence.length > 2 && (
          <button
            onClick={() => setExpanded(false)}
            className="mt-2 font-mono text-2xs text-accent hover:underline"
          >
            收起证据 ▴
          </button>
        )}

        {/* Draft preview */}
        {draft && (
          <div className="mt-4 rounded-lg border border-amber-glow bg-amber-dim p-3.5">
            <div className="mb-1 font-mono text-2xs uppercase tracking-wider text-amber">
              AI 已备好草稿
            </div>
            <div className="text-sm text-secondary">
              <span className="font-medium text-primary">「{draft.name}」</span>
              {draft.body && (
                <>
                  {" "}— {draft.body.slice(0, 120)}
                  {draft.body.length > 120 ? "…" : ""}
                </>
              )}
            </div>
          </div>
        )}

        {/* Stats for skill_feedback */}
        {current?.kind === "skill_feedback" && statsExpanded && stats && (
          <div className="mt-4 rounded-lg border border-[var(--border-subtle)] bg-primary p-3.5">
            <div className="mb-2 font-mono text-2xs text-tertiary">使用效果对比</div>
            <div className="grid grid-cols-2 gap-2">
              {Object.entries(stats).map(([k, v]) => (
                <div key={k} className="rounded-md bg-secondary px-2.5 py-1.5">
                  <div className="text-2xs text-secondary">{k}</div>
                  <div className="font-mono text-sm text-primary">{v}</div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* SPEC-C2 T1: three-choice actions. */}
        <div className="mt-5">
          {hasThreeChoices ? (
            <>
              {/* Three buttons: adopt-and-sync / edit-then-adopt / reject. */}
              {!dismissOpen && !isPartial && (
                <div className="flex items-center gap-2" data-testid="three-choice-actions">
                  <Button
                    variant="primary"
                    size="sm"
                    className="!bg-amber !text-inverse hover:brightness-105"
                    onClick={() => current && handleAccept(current)}
                    loading={animating === "accept"}
                    disabled={deciding || animating === "accept"}
                  >
                    采纳并同步
                  </Button>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => current && handleAcceptEdited(current)}
                    disabled={deciding || animating === "accept"}
                  >
                    修改后采纳
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setDismissOpen(true)}
                    disabled={deciding || animating === "accept"}
                  >
                    拒绝
                  </Button>
                  <span className="ml-auto font-mono text-2xs text-tertiary">
                    30 天内不再提示同类
                  </span>
                </div>
              )}
              {/* SPEC-C2 T1: inline dismiss reason picker (non-modal). */}
              {dismissOpen && (
                <div className="flex flex-wrap items-center gap-2" data-testid="dismiss-reasons">
                  <span className="text-sm text-secondary">拒绝原因：</span>
                  {REASON_OPTIONS.map((opt) => (
                    <button
                      key={opt.value}
                      onClick={() => current && handleDismiss(current, opt.value)}
                      disabled={deciding}
                      className="rounded-lg border border-[var(--border-subtle)] bg-primary px-3 py-1.5 text-xs text-secondary transition-colors hover:border-danger/40 hover:text-danger"
                    >
                      <kbd className="mr-1 font-mono text-2xs text-tertiary">{opt.key}</kbd>
                      {opt.label}
                    </button>
                  ))}
                  <button
                    onClick={() => setDismissOpen(false)}
                    className="ml-auto text-xs text-tertiary hover:text-secondary"
                  >
                    取消
                  </button>
                </div>
              )}
            </>
          ) : current?.kind === "skill_feedback" ? (
            <div className="flex items-center gap-2">
              <Button variant="ghost" size="sm"
                onClick={() => setStatsExpanded((v) => !v)} disabled={deciding}>
                {statsExpanded ? "收起对比" : "查看对比"}
              </Button>
              <Button variant="ghost" size="sm"
                onClick={() => current && handleDismiss(current)} disabled={deciding}>
                知道了
              </Button>
              <span className="ml-auto font-mono text-2xs text-tertiary">30 天内不再提示同类</span>
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <Button variant="ghost" size="sm"
                onClick={() => current && goDiscover(current.payload?.note || current.title)}
                disabled={deciding}>
                去发现页找
              </Button>
              <Button variant="ghost" size="sm" onClick={openNewSkillDialog} disabled={deciding}>
                自己写
              </Button>
              <Button variant="ghost" size="sm"
                onClick={() => current && handleDismiss(current)} disabled={deciding}>
                划掉
              </Button>
              <span className="ml-auto font-mono text-2xs text-tertiary">30 天内不再提示同类</span>
            </div>
          )}
        </div>
      </Card>

      {/* SPEC-C2 T3: keyboard hints. */}
      <div className="mt-5 flex flex-wrap items-center justify-center gap-4 font-mono text-2xs text-tertiary">
        <Hint kbd="A" text="采纳" />
        <Hint kbd="E" text="修改" />
        <Hint kbd="R" text="拒绝" />
        <Hint kbd="← →" text="切换" />
        <Hint kbd="空格" text="证据" />
        <Hint kbd="esc" text="返回" />
      </div>
    </div>
  );
}

function TabButton({ active, onClick, label }: { active: boolean; onClick: () => void; label: string }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-lg px-3 py-1.5 text-sm transition-colors ${
        active
          ? "bg-amber/15 text-amber"
          : "text-secondary hover:bg-tertiary/40 hover:text-primary"
      }`}
    >
      {label}
    </button>
  );
}

function Hint({ kbd, text }: { kbd: string; text: string }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      {kbd.split(" ").map((k) => (
        <kbd
          key={k}
          className="rounded border border-[var(--border-subtle)] px-1 py-0.5 text-secondary"
        >
          {k}
        </kbd>
      ))}
      {text}
    </span>
  );
}
