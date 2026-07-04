import { describe, it, expect } from "vitest";
import { humanizeError } from "./errors";

describe("humanizeError", () => {
  it("maps 404 unknown command to a friendly message and keeps detail", () => {
    const result = humanizeError(new Error("invoke(get_sources) → 404: unknown command"), {
      context: "加载远程源",
    });
    expect(result.message).toContain("此功能在当前环境不可用");
    expect(result.detail).toContain("404: unknown command");
  });

  it("maps network failures to a friendly message", () => {
    const result = humanizeError(new Error("Failed to fetch"), { context: "分析数据加载" });
    expect(result.message).toContain("无法连接后端服务");
    expect(result.detail).toContain("Failed to fetch");
  });

  it("maps timeout errors to a retry message", () => {
    const result = humanizeError(new Error("request timed out"), { context: "同步" });
    expect(result.message).toContain("无法连接后端服务");
  });

  it("falls back to a generic message for unrecognized errors", () => {
    const result = humanizeError(new Error("something weird"), { context: "初始化" });
    expect(result.message).toContain("操作失败，请重试");
    expect(result.detail).toContain("something weird");
  });

  it("handles plain string errors", () => {
    const result = humanizeError("permission denied", { context: "写入" });
    expect(result.message).toContain("没有权限");
  });

  // SPEC-F6 T3: 新增的四条 RULES 各覆盖一条。
  it("同步失败（写 Agent 目录）给出恢复动作", () => {
    const result = humanizeError(new Error("create symlink failed: permission denied"), { context: "同步到 Agent" });
    expect(result.action).toBeTruthy();
    expect(result.action).toContain("目录权限");
  });

  it("采集源被占用/锁定给出自动重试提示", () => {
    const result = humanizeError(new Error("database is locked"), { context: "采集" });
    expect(result.action).toContain("稍后");
  });

  it("Skill 重名冲突给出换名/删除提示", () => {
    const result = humanizeError(new Error("Skill 'weekly-report' already exists"), { context: "创建 Skill" });
    expect(result.action).toContain("名称");
  });

  it("导入空目录 / 缺 SKILL.md 给出确认目录提示", () => {
    const result = humanizeError(new Error("no skill found: SKILL.md not found"), { context: "导入 Skill" });
    expect(result.action).toContain("规则文件");
  });
});
