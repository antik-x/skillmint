import { describe, it, expect } from "vitest";
import { agentDisplayName } from "./Agents";

describe("SPEC-F6 T1: agentDisplayName 三层消歧", () => {
  it("无 allAgents 时返回原名（向后兼容）", () => {
    const agent = { id: "a1", name: "claude-code", skill_directory: "/x/.cursor/skills" };
    expect(agentDisplayName(agent)).toBe("claude-code");
  });

  it("无名时回退到目录末段", () => {
    const agent = { id: "a1", name: "", skill_directory: "/x/.cursor/skills" };
    expect(agentDisplayName(agent)).toBe("skills");
  });

  it("同名时第一层回退：追加目录末段", () => {
    // 两个同名 Agent，目录末段不同
    const a = { id: "a1", name: "skills", skill_directory: "/x/.cursor/skills" };
    const b = { id: "a2", name: "skills", skill_directory: "/y/.claude/rules" };
    const list = [a, b];
    // a 的末段是 skills，base="skills"；冲突；末段==base 跳过；
    // 追加末两段 → "skills · .cursor/skills"，在 list 内唯一 → 返回。
    expect(agentDisplayName(a, list)).toBe("skills · .cursor/skills");
    // b 的末段是 rules，base="skills"；冲突；追加末段 → "skills · rules"，唯一 → 返回。
    expect(agentDisplayName(b, list)).toBe("skills · rules");
  });

  it("同目录极端例：27 个同名同目录 → 必须走到 id 短后缀且唯一", () => {
    // 模拟评委场景：27 个完全相同的通用 Agent（name=skills, 目录=~/.skillmint/skills）
    const dir = "/home/u/.skillmint/skills";
    const list = Array.from({ length: 27 }, (_, i) => ({
      id: `agent-${i.toString(16).padStart(2, "0")}`,
      name: "skills",
      skill_directory: dir,
    }));

    const labels = list.map((a) => agentDisplayName(a, list));
    // 关键断言：列表内显示名必须两两唯一。
    const uniq = new Set(labels);
    expect(uniq.size).toBe(labels.length);

    // 每个显示名应包含 id 短后缀（4 位）。
    for (let i = 0; i < list.length; i++) {
      const expectedSuffix = list[i].id.replace(/[^a-zA-Z0-9]/g, "").slice(-4);
      expect(labels[i]).toContain(expectedSuffix);
    }
  });

  it("id 相同时回退到目录路径区分（不碰撞）", () => {
    // 罕见：两个 Agent 没有独立 id，但目录不同。
    const a = { id: "same", name: "skills", skill_directory: "/a/x/skills" };
    const b = { id: "same", name: "skills", skill_directory: "/b/y/skills" };
    const list = [a, b];
    expect(agentDisplayName(a, list)).not.toBe(agentDisplayName(b, list));
  });

  it("同名但目录末段不同时，仅追加末段即可消歧", () => {
    const a = { id: "a1", name: "claude", skill_directory: "/x/rules" };
    const b = { id: "a2", name: "claude", skill_directory: "/y/skills" };
    const list = [a, b];
    expect(agentDisplayName(a, list)).toBe("claude · rules");
    expect(agentDisplayName(b, list)).toBe("claude · skills");
  });

  // 防御性：mock/未来字段变动可能导致 name/skill_directory 缺失，绝不能抛错。
  it("skill_directory 缺失时不抛错并返回兜底名", () => {
    const agent = { id: "a1", name: "claude" } as never;
    expect(() => agentDisplayName(agent)).not.toThrow();
    expect(agentDisplayName(agent)).toBe("claude");
  });

  it("name 与 skill_directory 均缺失时返回「未命名 Agent」并按 id 消歧", () => {
    const a = { id: "abc-1234" } as never;
    const b = { id: "def-5678" } as never;
    const list = [a, b];
    expect(() => agentDisplayName(a, list)).not.toThrow();
    expect(agentDisplayName(a, list)).not.toBe(agentDisplayName(b, list));
  });

  it("列表内含 skill_directory 缺失的 Agent 时不抛错（不崩溃 Onboarding）", () => {
    // 模拟 mock scan_agents 只返回 id 的场景（EVAL-03 复验中发现的真实回归）。
    const list = Array.from({ length: 36 }, (_, i) => ({ id: `agent-${i}` })) as never[];
    const labels = list.map((a) => (agentDisplayName as (x: never, y: never[]) => string)(a, list));
    // 关键：不抛 + 列表内唯一。
    expect(new Set(labels).size).toBe(labels.length);
  });

  // EVAL-03 复验中发现：多个 Agent id 末段相同（agent-claude-code / agent-kimi-code /
  // agent-zcode）时，固定 4 位短后缀会全部撞成 "code"，破坏唯一性。
  it("id 末段相同（*-code）时逐位加长后缀直到唯一", () => {
    const list = [
      { id: "agent-claude-code" },
      { id: "agent-kimi-code" },
      { id: "agent-zcode" },
    ] as never[];
    const labels = list.map((a) => (agentDisplayName as (x: never, y: never[]) => string)(a, list));
    expect(new Set(labels).size).toBe(labels.length);
    // 每个标签都应带足以区分的尾部（e / ecode 之类），不再都是 "code"。
    expect(labels.every((l) => !l.endsWith("· code"))).toBe(true);
  });
});
