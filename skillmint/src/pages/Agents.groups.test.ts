import { describe, it, expect } from "vitest";
import {
  agentGroup,
  legacyAgentDisplayName,
  buildAgentRows,
  AGENT_GROUP_LABELS,
  type AgentRow,
} from "./Agents";
import type { Agent } from "../types";

// P3-10: 构造一个最小 Agent（分组/排序只依赖 id/name/source/skill_directory）。
function agent(id: string, name: string, source?: string): Agent {
  return {
    id,
    name,
    skill_directory: `/home/u/.${id}/skills`,
    is_enabled: true,
    ...(source !== undefined ? { source } : {}),
  };
}

describe("P3-10: agentGroup 分组判据", () => {
  it("有独立 source key 的工具 → harness", () => {
    expect(agentGroup(agent("a1", "Claude Code", "claude-code"))).toBe("harness");
    expect(agentGroup(agent("a2", "ZCode", "zcode"))).toBe("harness");
  });

  it("source 为空/缺失/whitespace → shared（共享/占位）", () => {
    expect(agentGroup(agent("a3", "Generic Skills", ""))).toBe("shared");
    expect(agentGroup(agent("a4", "NoSource"))).toBe("shared");
    expect(agentGroup(agent("a5", "Spaces", "  "))).toBe("shared");
  });

  it("source=universal（canonical 仓库）→ shared", () => {
    expect(agentGroup(agent("a6", "Universal Agents", "universal"))).toBe("shared");
  });
});

describe("P3-10: legacyAgentDisplayName 旧版行改名", () => {
  it("~/.skillmint/skills 目录 → 固定显示名", () => {
    expect(legacyAgentDisplayName({ skill_directory: "/Users/u/.skillmint/skills" })).toBe(
      "SkillMint 旧版目录",
    );
    expect(legacyAgentDisplayName({ skill_directory: "~/.skillmint/skills" })).toBe(
      "SkillMint 旧版目录",
    );
  });

  it("其他目录返回 null（走正常消歧）", () => {
    expect(legacyAgentDisplayName({ skill_directory: "/Users/u/.claude/skills" })).toBeNull();
    expect(legacyAgentDisplayName({})).toBeNull();
    // 目录末段相同但不是 .skillmint/skills 本身。
    expect(legacyAgentDisplayName({ skill_directory: "/Users/u/.skillmint/skills-backup" })).toBeNull();
  });
});

describe("P3-10: buildAgentRows 双分组与排序", () => {
  it("两个区块：Harness 在上、共享/占位在下，组内按技能数降序", () => {
    const agents = [
      agent("generic", "Generic Skills", ""), // shared
      agent("cc", "Claude Code", "claude-code"), // harness, 112
      agent("uni", "Universal Agents", "universal"), // shared, 114
      agent("zx", "ZCode", "zcode"), // harness, 76
      agent("legacy", "Agent"), // shared（旧版行）
    ];
    const counts = { cc: 112, zx: 76, uni: 114, generic: 83, legacy: 83 };
    const rows = buildAgentRows(agents, counts);

    const headers = rows.filter((r: AgentRow) => r.kind === "header");
    expect(headers.map((h) => (h.kind === "header" ? h.title : ""))).toEqual([
      AGENT_GROUP_LABELS.harness,
      AGENT_GROUP_LABELS.shared,
    ]);
    expect(headers.map((h) => (h.kind === "header" ? h.count : 0))).toEqual([2, 3]);

    const harnessIds = rows.slice(1, 3).map((r) => (r.kind === "agent" ? r.agent.id : ""));
    expect(harnessIds).toEqual(["cc", "zx"]); // 112 > 76

    const sharedIds = rows.slice(4).map((r) => (r.kind === "agent" ? r.agent.id : ""));
    // 114 > 83 = 83，平局按名称排序（"Agent" < "Generic Skills"）。
    expect(sharedIds).toEqual(["uni", "legacy", "generic"]);
  });

  it("技能数未加载（缺 key）时沉底，不阻塞渲染", () => {
    const agents = [agent("a", "A", "src-a"), agent("b", "B", "src-b")];
    const rows = buildAgentRows(agents, {});
    const ids = rows.filter((r) => r.kind === "agent").map((r) => (r.kind === "agent" ? r.agent.id : ""));
    expect(ids).toEqual(["a", "b"]); // 稳定回退到名称序
  });

  it("全部同组时不输出空区块头", () => {
    const rows = buildAgentRows([agent("a", "A", "src-a")], { a: 1 });
    expect(rows).toHaveLength(2); // 1 header + 1 agent
    expect(rows[0].kind).toBe("header");
  });
});
