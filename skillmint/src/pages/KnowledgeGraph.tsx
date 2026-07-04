import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Share2, Lightbulb, Moon } from "lucide-react";
import { showError, showSuccess } from "../stores/toastStore";
import { useAppStore } from "../stores/appStore";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { EmptyState } from "../components/ui/EmptyState";
import type { KgGraph, KgNode, Skill, SkillRecommendation, TaskRecommendation } from "../types";
import Graph from "graphology";
import { SigmaContainer, useLoadGraph, useRegisterEvents, useSetSettings, useSigma } from "@react-sigma/core";
import "@react-sigma/core/lib/style.css";
import forceAtlas2 from "graphology-layout-forceatlas2";

// --- Constants -------------------------------------------------------------

const TYPE_COLORS: Record<string, string> = {
  concept: "#60a5fa", // blue-400
  scenario: "#fb923c", // orange-400
  practice: "#c084fc", // purple-400
  object: "#4ade80", // green-400
  action: "#f87171", // red-400
};
const FALLBACK_COLOR = "#94a3b8"; // slate-400

const TYPE_LABELS: Record<string, string> = {
  concept: "概念",
  scenario: "场景",
  practice: "实践",
  object: "对象",
  action: "动作",
};

const BASE_NODE_SIZE = 8;
const MAX_NODE_SIZE = 26;
const WORKER_LAYOUT_NODE_THRESHOLD = 220;
const LABEL_COLOR_DARK = "#e2e8f0";
const EDGE_COLOR = "rgba(100,116,139,0.18)"; // slate-500 low alpha
const DIMMED_EDGE = "rgba(71,85,105,0.12)";
const ACTIVE_EDGE = "#38bdf8"; // sky-400

type HoverState = { node: string; neighbors: Set<string> } | null;

// Cache computed node positions across re-renders so we don't re-layout unnecessarily.
const positionCache = new Map<string, { x: number; y: number }>();
let lastLayoutDataKey = "";
let pendingLayoutDataKey = "";

function nodeColor(type: string): string {
  return TYPE_COLORS[type] ?? FALLBACK_COLOR;
}

function mixColor(color1: string, color2: string, ratio: number): string {
  const hex = (c: string) => parseInt(c, 16);
  const r1 = hex(color1.slice(1, 3)), g1 = hex(color1.slice(3, 5)), b1 = hex(color1.slice(5, 7));
  const r2 = hex(color2.slice(1, 3)), g2 = hex(color2.slice(3, 5)), b2 = hex(color2.slice(5, 7));
  const r = Math.round(r1 + (r2 - r1) * ratio);
  const g = Math.round(g1 + (g2 - g1) * ratio);
  const b = Math.round(b1 + (b2 - b1) * ratio);
  return `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
}

function nodeSize(degree: number, maxDegree: number): number {
  if (maxDegree === 0) return BASE_NODE_SIZE;
  const ratio = degree / maxDegree;
  return BASE_NODE_SIZE + Math.sqrt(ratio) * (MAX_NODE_SIZE - BASE_NODE_SIZE);
}

function layoutIterations(n: number): number {
  if (n > 2500) return 28;
  if (n > 1200) return 40;
  if (n > 600) return 65;
  if (n > 250) return 90;
  return 140;
}

function edgeAlpha(weight: number, maxWeight: number): number {
  if (maxWeight === 0) return 0.18;
  const norm = weight / maxWeight;
  return (40 + norm * 180) / 255;
}

function graphDataKey(nodes: readonly KgNode[], edges: readonly { source_id: string; target_id: string; weight?: number }[]): string {
  const ns = nodes.map((n) => n.id).sort().join(",");
  const es = edges.map((e) => `${e.source_id}->${e.target_id}`).sort().join(",");
  return `${nodes.length}:${edges.length}:${ns}:${es}`;
}

function makeLayoutWorker(): Worker | null {
  try {
    return new Worker(new URL("./graph-layout-worker.ts", import.meta.url), { type: "module" });
  } catch (err) {
    console.warn("[KG] layout worker unavailable; main-thread fallback:", err);
    return null;
  }
}

// --- Inner Sigma components ------------------------------------------------

function GraphLoader({ nodes, edges }: { nodes: KgNode[]; edges: KgGraph["edges"] }) {
  const loadGraph = useLoadGraph();
  const sigma = useSigma();

  useEffect(() => {
    const dataKey = graphDataKey(nodes, edges);
    const needsLayout = dataKey !== lastLayoutDataKey && dataKey !== pendingLayoutDataKey;
    let cancelled = false;
    let worker: Worker | null = null;

    const graph = new Graph();

    // Compute degree per node.
    const degree = new Map<string, number>();
    for (const e of edges) {
      degree.set(e.source_id, (degree.get(e.source_id) ?? 0) + 1);
      degree.set(e.target_id, (degree.get(e.target_id) ?? 0) + 1);
    }
    const maxDegree = Math.max(1, ...nodes.map((n) => degree.get(n.id) ?? 0));

    for (const node of nodes) {
      const cached = positionCache.get(node.id);
      graph.addNode(node.id, {
        type: "circle",
        x: cached?.x ?? Math.random() * 100,
        y: cached?.y ?? Math.random() * 100,
        size: nodeSize(degree.get(node.id) ?? 0, maxDegree),
        color: nodeColor(node.type),
        label: node.label,
        nodeType: node.type,
        nodeSource: node.source ?? "",
        nodeDescription: node.description ?? "",
      });
    }

    const maxWeight = Math.max(1, ...edges.map((e) => e.weight ?? 1));
    for (const edge of edges) {
      if (!graph.hasNode(edge.source_id) || !graph.hasNode(edge.target_id)) continue;
      const key = `${edge.source_id}->${edge.target_id}`;
      if (graph.hasEdge(key) || graph.hasEdge(`${edge.target_id}->${edge.source_id}`)) continue;
      const w = edge.weight ?? 1;
      graph.addEdgeWithKey(key, edge.source_id, edge.target_id, {
        color: `rgba(100,116,139,${edgeAlpha(w, maxWeight)})`,
        size: 0.5 + (w / maxWeight) * 3.5,
        sourceNode: edge.source_id,
        targetNode: edge.target_id,
        relation: edge.relation,
      });
    }

    const runMainThreadLayout = () => {
      const settings = forceAtlas2.inferSettings(graph);
      forceAtlas2.assign(graph, {
        iterations: layoutIterations(nodes.length),
        settings: {
          ...settings,
          gravity: 1,
          scalingRatio: nodes.length > 400 ? 3 : 2,
          strongGravityMode: true,
          barnesHutOptimize: nodes.length > 50,
        },
      });
      lastLayoutDataKey = dataKey;
      graph.forEachNode((id, attrs) => positionCache.set(id, { x: attrs.x, y: attrs.y }));
    };

    if (needsLayout && nodes.length > 1 && nodes.length < WORKER_LAYOUT_NODE_THRESHOLD) {
      runMainThreadLayout();
    }
    loadGraph(graph);

    if (needsLayout && nodes.length >= WORKER_LAYOUT_NODE_THRESHOLD) {
      worker = makeLayoutWorker();
      if (!worker) {
        runMainThreadLayout();
        loadGraph(graph);
        return undefined;
      }
      pendingLayoutDataKey = dataKey;
      worker.onmessage = (event: MessageEvent<{ key: string; positions: Array<{ id: string; x: number; y: number }> }>) => {
        if (cancelled || event.data.key !== dataKey) return;
        for (const { id, x, y } of event.data.positions) {
          if (!graph.hasNode(id)) continue;
          graph.setNodeAttribute(id, "x", x);
          graph.setNodeAttribute(id, "y", y);
          positionCache.set(id, { x, y });
        }
        lastLayoutDataKey = dataKey;
        if (pendingLayoutDataKey === dataKey) pendingLayoutDataKey = "";
        sigma.refresh();
      };
      worker.onerror = () => {
        if (cancelled) return;
        if (pendingLayoutDataKey === dataKey) pendingLayoutDataKey = "";
        runMainThreadLayout();
        loadGraph(graph);
      };
      worker.postMessage({
        key: dataKey,
        nodes: nodes.map((n) => {
          const c = positionCache.get(n.id);
          return { id: n.id, x: c?.x ?? graph.getNodeAttribute(n.id, "x"), y: c?.y ?? graph.getNodeAttribute(n.id, "y") };
        }),
        edges: edges.map((e) => ({ source: e.source_id, target: e.target_id, weight: e.weight ?? 1 })),
        iterations: layoutIterations(nodes.length),
        scalingRatio: nodes.length > 400 ? 3 : 2,
      });
    }

    return () => {
      cancelled = true;
      if (pendingLayoutDataKey === dataKey) pendingLayoutDataKey = "";
      worker?.terminate();
    };
  }, [loadGraph, sigma, nodes, edges]);

  return null;
}

function GraphRenderSettings({
  hoverState,
  selectedNode,
  matchedNodeIds,
  hiddenTypes,
}: {
  hoverState: HoverState;
  selectedNode: string | null;
  matchedNodeIds: Set<string> | null;
  hiddenTypes: Set<string>;
}) {
  const sigma = useSigma();
  const setSettings = useSetSettings();

  useEffect(() => {
    setSettings({
      hideEdgesOnMove: true,
      hideLabelsOnMove: true,
      renderEdgeLabels: false,
      nodeReducer: (node, attrs) => {
        const result = { ...attrs };
        // Hide nodes whose type is filtered out.
        if (hiddenTypes.has(String(attrs.nodeType))) {
          result.hidden = true;
          return result;
        }
        const hasHover = !!hoverState;
        const hasSelect = !!selectedNode;
        const hasMatch = matchedNodeIds && matchedNodeIds.size > 0;
        const isHoverNode = hoverState?.node === node;
        const isHoverNeighbor = hoverState?.neighbors.has(node) ?? false;
        const isSelected = selectedNode === node;
        const isSelectNeighbor =
          !!selectedNode &&
          sigma.getGraph().neighbors(selectedNode).includes(node);
        const isMatched = matchedNodeIds?.has(node) ?? false;

        if (isHoverNode || isSelected) {
          result.size = (attrs.size ?? BASE_NODE_SIZE) * 1.5;
          result.zIndex = 10;
          result.forceLabel = true;
        }
        if (isMatched) {
          result.size = (attrs.size ?? BASE_NODE_SIZE) * 1.3;
          result.forceLabel = true;
        }
        const dim = (hasHover && !isHoverNode && !isHoverNeighbor) ||
          (hasSelect && !isSelected && !isSelectNeighbor) ||
          (hasMatch && !isMatched);
        if (dim) {
          result.color = mixColor(attrs.color ?? FALLBACK_COLOR, "#334155", 0.75);
          result.label = "";
          result.size = (attrs.size ?? BASE_NODE_SIZE) * 0.6;
        }
        return result;
      },
      edgeReducer: (_edge, attrs) => {
        const result = { ...attrs };
        const source = String(attrs.sourceNode ?? "");
        const target = String(attrs.targetNode ?? "");
        const hasHover = !!hoverState;
        const hasSelect = !!selectedNode;
        const hoverEdge = hasHover && (source === hoverState?.node || target === hoverState?.node);
        const selectEdge = hasSelect && (source === selectedNode || target === selectedNode);

        if ((hasHover && !hoverEdge) || (hasSelect && !selectEdge)) {
          result.color = DIMMED_EDGE;
          result.size = 0.3;
        }
        if (hoverEdge || selectEdge) {
          result.color = ACTIVE_EDGE;
          result.size = Math.max(2, (attrs.size ?? 1) * 1.5);
        }
        return result;
      },
    });
    sigma.refresh();
  }, [setSettings, sigma, hoverState, selectedNode, matchedNodeIds, hiddenTypes]);

  return null;
}

function EventHandler({
  onNodeClick,
  onHoverChange,
  onStageClick,
}: {
  onNodeClick: (nodeId: string) => void;
  onHoverChange: (state: HoverState) => void;
  onStageClick: () => void;
}) {
  const registerEvents = useRegisterEvents();
  const sigma = useSigma();

  useEffect(() => {
    registerEvents({
      clickNode: ({ node }) => onNodeClick(node),
      clickStage: () => onStageClick(),
      enterNode: ({ node }) => {
        const container = sigma.getContainer();
        container.style.cursor = "pointer";
        onHoverChange({ node, neighbors: new Set(sigma.getGraph().neighbors(node)) });
      },
      leaveNode: () => {
        const container = sigma.getContainer();
        container.style.cursor = "default";
        onHoverChange(null);
      },
    });
  }, [registerEvents, sigma, onNodeClick, onHoverChange, onStageClick]);

  return null;
}

function ZoomControls() {
  const sigma = useSigma();
  return (
    <div className="absolute right-3 top-3 z-10 flex flex-col gap-1">
      <button
        onClick={() => sigma.getCamera().animatedZoom({ duration: 200 })}
        className="flex h-7 w-7 items-center justify-center rounded border border-[var(--border-prominent)] bg-secondary/80 text-sm text-primary backdrop-blur hover:bg-tertiary"
        title="放大"
      >
        +
      </button>
      <button
        onClick={() => sigma.getCamera().animatedUnzoom({ duration: 200 })}
        className="flex h-7 w-7 items-center justify-center rounded border border-[var(--border-prominent)] bg-secondary/80 text-sm text-primary backdrop-blur hover:bg-tertiary"
        title="缩小"
      >
        −
      </button>
      <button
        onClick={() => sigma.getCamera().animatedReset({ duration: 300 })}
        className="flex h-7 w-7 items-center justify-center rounded border border-[var(--border-prominent)] bg-secondary/80 text-sm text-primary backdrop-blur hover:bg-tertiary"
        title="重置视角"
      >
        ⤢
      </button>
    </div>
  );
}

// --- Main page -------------------------------------------------------------

export default function KnowledgeGraph() {
  const skills = useAppStore((state) => state.skills);
  const [graph, setGraph] = useState<KgGraph>({ nodes: [], edges: [] });
  const [analyzing, setAnalyzing] = useState(false);
  const [hoverState, setHoverState] = useState<HoverState>(null);
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [hiddenTypes, setHiddenTypes] = useState<Set<string>>(new Set());
  const [rec, setRec] = useState<TaskRecommendation | null>(null);
  const [task, setTask] = useState("");
  const [searching, setSearching] = useState(false);
  const [sigmaKey, setSigmaKey] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);

  const loadGraph = useCallback(async () => {
    try {
      const g = await invoke<KgGraph>("get_knowledge_graph", { maxNodes: 200 });
      setGraph(g);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`加载图谱失败：${msg}`);
    }
  }, []);

  useEffect(() => {
    loadGraph();
  }, [loadGraph]);

  const handleAnalyze = async () => {
    if (analyzing) return;
    setAnalyzing(true);
    try {
      const res = await invoke<[number, number]>("analyze_knowledge_graph");
      showSuccess(`分析完成：${res[0]} 个节点，${res[1]} 条关系`);
      positionCache.clear();
      lastLayoutDataKey = "";
      await loadGraph();
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`分析失败：${msg}`);
    } finally {
      setAnalyzing(false);
    }
  };

  const handleRecommend = async () => {
    if (searching || !task.trim()) return;
    setSearching(true);
    try {
      const r = await invoke<TaskRecommendation>("recommend_skills_for_task", { task });
      setRec(r);
    } catch (err) {
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
      showError(`推荐失败：${msg}`);
    } finally {
      setSearching(false);
    }
  };

  // Resize guard: remount Sigma when container resizes to avoid renderer crash.
  useEffect(() => {
    if (!containerRef.current) return;
    const ro = new ResizeObserver(() => {
      setSigmaKey((k) => k + 1);
    });
    ro.observe(containerRef.current);
    return () => ro.disconnect();
  }, []);

  // Search matches
  const matchedNodeIds = useMemo(() => {
    if (!search.trim()) return null;
    const q = search.toLowerCase();
    const set = new Set<string>();
    for (const n of graph.nodes) {
      if (n.label.toLowerCase().includes(q)) set.add(n.id);
    }
    return set;
  }, [search, graph.nodes]);

  // Selected node detail data
  const selectedDetail = useMemo(() => {
    if (!selectedNode) return null;
    const node = graph.nodes.find((n) => n.id === selectedNode);
    if (!node) return null;
    const neighbors: { node: KgNode; relation: string }[] = [];
    for (const e of graph.edges) {
      if (e.source_id === selectedNode) {
        const nb = graph.nodes.find((n) => n.id === e.target_id);
        if (nb) neighbors.push({ node: nb, relation: e.relation });
      } else if (e.target_id === selectedNode) {
        const nb = graph.nodes.find((n) => n.id === e.source_id);
        if (nb) neighbors.push({ node: nb, relation: e.relation });
      }
    }
    return { node, neighbors };
  }, [selectedNode, graph]);

  // Type counts for legend
  const typeCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const n of graph.nodes) counts[n.type] = (counts[n.type] ?? 0) + 1;
    return counts;
  }, [graph.nodes]);

  const hasGraph = graph.nodes.length > 0;

  return (
    <div className="flex h-full flex-col">
      {/* Top bar */}
      <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-6 py-3">
        <div className="flex items-center gap-3">
          <h1 className="text-lg font-bold">知识图谱</h1>
          {hasGraph && (
            <span className="text-xs text-tertiary">
              {graph.nodes.length} 节点 · {graph.edges.length} 关系
              {hiddenTypes.size > 0 && ` · 隐藏 ${hiddenTypes.size} 类型`}
            </span>
          )}
        </div>
        <Button variant="primary" size="sm" onClick={handleAnalyze} loading={analyzing} disabled={analyzing}>
          <RefreshCw className={`h-4 w-4 ${analyzing ? "animate-spin" : ""}`} />
          {analyzing ? "分析中…" : "重新分析"}
        </Button>
      </div>

      {!hasGraph ? (
        <div className="flex flex-1 items-center justify-center p-8">
          <EmptyState
            icon={Share2}
            illustration="graph"
            title="还没有知识图谱"
            description="先在 Skill 管理创建或导入几个 Skill，再点击「重新分析」即可从 SKILL.md 抽取概念与关系。"
          />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          {/* Graph canvas + legend + search */}
          <div ref={containerRef} className="relative min-w-0 flex-1 bg-primary">
            {/* Search box */}
            <div className="absolute left-3 top-3 z-10">
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="搜索节点…"
                className="w-48 rounded-lg border border-[var(--border-prominent)] bg-secondary px-3 py-1 text-sm text-primary placeholder:text-tertiary focus:border-accent focus:outline-none"
              />
            </div>

            <SigmaContainer
              key={sigmaKey}
              style={{ width: "100%", height: "100%", background: "transparent" }}
              settings={{
                defaultNodeType: "circle",
                renderEdgeLabels: false,
                hideEdgesOnMove: true,
                hideLabelsOnMove: true,
                defaultEdgeColor: EDGE_COLOR,
                defaultNodeColor: FALLBACK_COLOR,
                labelSize: 13,
                labelWeight: "bold",
                labelColor: { color: LABEL_COLOR_DARK },
                stagePadding: 30,
              }}
            >
              <GraphLoader nodes={graph.nodes} edges={graph.edges} />
              <EventHandler
                onNodeClick={(id) => setSelectedNode((prev) => (prev === id ? null : id))}
                onHoverChange={setHoverState}
                onStageClick={() => setSelectedNode(null)}
              />
              <GraphRenderSettings
                hoverState={hoverState}
                selectedNode={selectedNode}
                matchedNodeIds={matchedNodeIds}
                hiddenTypes={hiddenTypes}
              />
              <ZoomControls />
            </SigmaContainer>

            {/* Legend */}
            <div className="absolute bottom-3 left-3 z-10 max-w-[240px] rounded-lg border border-[var(--border-subtle)] bg-secondary/90 px-3 py-2 text-xs backdrop-blur">
              <div className="mb-1.5 font-semibold text-primary">节点类型</div>
              <div className="space-y-0.5">
                {Object.entries(typeCounts)
                  .filter(([, c]) => c > 0)
                  .map(([type, count]) => {
                    const isHidden = hiddenTypes.has(type);
                    return (
                      <button
                        key={type}
                        onClick={() =>
                          setHiddenTypes((prev) => {
                            const next = new Set(prev);
                            if (next.has(type)) next.delete(type);
                            else next.add(type);
                            return next;
                          })
                        }
                        className={`flex w-full items-center gap-2 rounded px-1 py-0.5 text-left transition-colors hover:bg-tertiary/50 ${
                          isHidden ? "opacity-40" : ""
                        }`}
                        title={isHidden ? "点击显示" : "点击隐藏"}
                      >
                        <span
                          className="inline-block h-3 w-3 shrink-0 rounded-full"
                          style={{ backgroundColor: nodeColor(type) }}
                        />
                        <span className="text-primary">{TYPE_LABELS[type] ?? type}</span>
                        <span className="ml-auto text-tertiary">{count}</span>
                      </button>
                    );
                  })}
              </div>
            </div>

            {/* No-visible-nodes overlay */}
            {matchedNodeIds !== null && matchedNodeIds.size === 0 && (
              <div className="pointer-events-none absolute inset-0 z-20 flex flex-col items-center justify-center gap-2 bg-primary/85 text-secondary">
                <div className="text-sm">无匹配节点</div>
                <button
                  onClick={() => setSearch("")}
                  className="pointer-events-auto rounded border border-[var(--border-prominent)] px-3 py-1 text-xs hover:bg-tertiary"
                >
                  清除搜索
                </button>
              </div>
            )}
          </div>

          {/* Detail sidebar */}
          {selectedDetail && (
            <div className="w-80 shrink-0 overflow-y-auto border-l border-[var(--border-subtle)] bg-secondary">
              <div className="flex items-center justify-between border-b border-[var(--border-subtle)] px-4 py-3">
                <span className="text-sm font-medium text-primary">节点详情</span>
                <button
                  onClick={() => setSelectedNode(null)}
                  className="rounded p-1 text-secondary hover:bg-tertiary hover:text-white"
                >
                  ✕
                </button>
              </div>
              <div className="space-y-4 p-4">
                <div>
                  <div className="flex items-center gap-2">
                    <span
                      className="inline-block h-3 w-3 rounded-full"
                      style={{ backgroundColor: nodeColor(selectedDetail.node.type) }}
                    />
                    <span className="font-medium text-white">{selectedDetail.node.label}</span>
                  </div>
                  <div className="mt-1 text-xs text-tertiary">
                    {TYPE_LABELS[selectedDetail.node.type] ?? selectedDetail.node.type}
                    {selectedDetail.node.source && ` · 来源 ${selectedDetail.node.source}`}
                  </div>
                </div>
                {selectedDetail.node.description && (
                  <div className="text-sm text-primary">{selectedDetail.node.description}</div>
                )}
                <div>
                  <div className="mb-1.5 text-xs font-medium text-secondary">
                    相关节点（{selectedDetail.neighbors.length}）
                  </div>
                  {selectedDetail.neighbors.length === 0 ? (
                    <div className="text-xs text-tertiary">该节点无关系。</div>
                  ) : (
                    <ul className="space-y-1">
                      {selectedDetail.neighbors.map(({ node, relation }) => (
                        <li key={node.id}>
                          <button
                            onClick={() => setSelectedNode(node.id)}
                            className="flex w-full items-center justify-between rounded px-2 py-1 text-left text-sm hover:bg-tertiary"
                          >
                            <span className="text-primary">{node.label}</span>
                            <span className="text-xs text-tertiary">{relation}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {/* P2-1: insight cards */}
      {hasGraph && <InsightsPanel graph={graph} skills={skills} />}

      {/* Task-driven recommendation (preserved) */}
      {hasGraph && (
        <section className="shrink-0 border-t border-[var(--border-subtle)] bg-secondary p-4">
          <div className="mb-2 text-sm font-medium text-primary">任务驱动推荐</div>
          <div className="flex gap-2">
            <input
              value={task}
              onChange={(e) => setTask(e.target.value)}
              placeholder="如：实现带 JWT 认证的 Node.js API"
              className="flex-1 rounded-lg border border-[var(--border-prominent)] bg-primary px-3 py-2 text-sm text-white focus:border-accent focus:outline-none"
            />
            <button
              onClick={handleRecommend}
              disabled={searching || !task.trim()}
              className="rounded-lg bg-tertiary px-4 py-2 text-sm font-medium text-white hover:bg-tertiary disabled:opacity-50"
            >
              {searching ? "分析中…" : "推荐 Skill 组合"}
            </button>
          </div>
          {rec && (
            <div className="mt-3 space-y-2">
              {rec.recommendations.length === 0 ? (
                <div className="text-sm text-secondary">未找到匹配的 Skill。</div>
              ) : (
                rec.recommendations.map((r: SkillRecommendation) => (
                  <div key={r.skill_id} className="rounded-lg border border-[var(--border-prominent)] p-3">
                    <div className="font-medium">{r.skill_name}</div>
                    <div className="mt-1 text-xs text-secondary">
                      {r.reason}（{r.matched_concepts.join(" / ")}）
                    </div>
                  </div>
                ))
              )}
              {rec.gaps.length > 0 && (
                <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-400">
                  缺口：未覆盖概念 {rec.gaps.join(" / ")}，建议新建相关 Skill。
                </div>
              )}
            </div>
          )}
        </section>
      )}
    </div>
  );
}

function InsightsPanel({ graph, skills }: { graph: KgGraph; skills: Skill[] }) {
  const sleeping = useMemo(() => {
    const threshold = Date.now() / 1000 - 90 * 24 * 3600;
    return skills.filter((s) => s.updated_at < threshold).sort((a, b) => a.updated_at - b.updated_at);
  }, [skills]);

  const topNodes = useMemo(() => {
    const degree = new Map<string, number>();
    for (const e of graph.edges) {
      degree.set(e.source_id, (degree.get(e.source_id) ?? 0) + 1);
      degree.set(e.target_id, (degree.get(e.target_id) ?? 0) + 1);
    }
    return graph.nodes
      .map((n) => ({ node: n, degree: degree.get(n.id) ?? 0 }))
      .sort((a, b) => b.degree - a.degree)
      .slice(0, 5);
  }, [graph]);

  return (
    <section className="shrink-0 border-t border-[var(--border-subtle)] bg-secondary p-4">
      <div className="mb-3 flex items-center gap-2 text-sm font-medium text-primary">
        <Lightbulb className="h-4 w-4 text-accent" />
        图谱洞察
      </div>
      <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
        <Card className="p-3" padding="none">
          <div className="mb-2 flex items-center gap-2 text-xs font-medium text-secondary">
            <Moon className="h-3.5 w-3.5" />
            沉睡 Skill（90 天未更新）
          </div>
          {sleeping.length === 0 ? (
            <div className="text-xs text-tertiary">暂无沉睡 Skill</div>
          ) : (
            <ul className="space-y-1">
              {sleeping.slice(0, 5).map((s) => (
                <li key={s.id} className="text-xs text-primary">
                  {s.name}
                  <span className="ml-1 text-tertiary">{new Date(s.updated_at * 1000).toLocaleDateString()}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card className="p-3" padding="none">
          <div className="mb-2 text-xs font-medium text-secondary">高连接概念</div>
          {topNodes.length === 0 ? (
            <div className="text-xs text-tertiary">暂无节点</div>
          ) : (
            <ul className="space-y-1">
              {topNodes.map(({ node, degree }) => (
                <li key={node.id} className="flex items-center justify-between text-xs">
                  <span className="text-primary">{node.label}</span>
                  <span className="text-tertiary">{degree} 连接</span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card className="p-3" padding="none">
          <div className="mb-2 text-xs font-medium text-secondary">图谱健康度</div>
          <div className="space-y-1 text-xs text-secondary">
            <div className="flex justify-between">
              <span>节点</span>
              <span className="text-primary">{graph.nodes.length}</span>
            </div>
            <div className="flex justify-between">
              <span>关系</span>
              <span className="text-primary">{graph.edges.length}</span>
            </div>
            <div className="flex justify-between">
              <span>平均度</span>
              <span className="text-primary">
                {graph.nodes.length > 0 ? (graph.edges.length * 2 / graph.nodes.length).toFixed(1) : "—"}
              </span>
            </div>
          </div>
        </Card>
      </div>
    </section>
  );
}

