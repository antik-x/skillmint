/**
 * P3-4: the npx-driven skills library — index rows render with provenance
 * badges, and destructive actions are exposed per managed_by (design lock-in:
 * npx rows get update/remove, unmanaged rows get collect-to-hub).
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import InstalledSkills from "./InstalledSkills";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

const agentTable = [
  { key: "claude-code", display_name: "Claude Code", project_dir: ".claude/skills", global_dir: "~/.claude/skills" },
  { key: "zcode", display_name: "ZCode", project_dir: ".zcode/skills", global_dir: "~/.zcode/skills" },
];

const rows = [
  {
    name: "pdf",
    scope: "global",
    managed_by: "npx",
    path: "~/.agents/skills/pdf",
    skill_md_path: "~/.agents/skills/pdf/SKILL.md",
    source: "vercel-labs/agent-skills",
    source_url: null,
    source_type: "github",
    ref_spec: null,
    hash: "treehash",
    content_hash: "sha256-1",
    status: "ok",
    agents: ["claude-code", "zcode"],
    description: "PDF 处理技能",
    updated_at: 0,
  },
  {
    name: "hand-made",
    scope: "global",
    managed_by: "unmanaged",
    path: "~/.claude/skills/hand-made",
    skill_md_path: "~/.claude/skills/hand-made/SKILL.md",
    source: null,
    source_url: null,
    source_type: null,
    ref_spec: null,
    hash: "",
    content_hash: "sha256-2",
    status: "ok",
    agents: ["claude-code"],
    description: null,
    updated_at: 0,
  },
  {
    name: "my-own-skill",
    scope: "hub-global",
    managed_by: "hub",
    path: "~/.skillmint/hub/my-own-skill",
    skill_md_path: "~/.skillmint/hub/my-own-skill/SKILL.md",
    source: null,
    source_url: null,
    source_type: null,
    ref_spec: null,
    hash: "",
    content_hash: "sha256-3",
    status: "ok",
    agents: [],
    description: "自建技能",
    updated_at: 0,
  },
];

function routeInvoke(cmd: string) {
  switch (cmd) {
    case "rebuild_skill_index":
      return { total: rows.length, npx_global: 1, unmanaged: 1, hub: 1, modified: 0, broken: 0 };
    case "get_skill_index":
      return rows.filter((r) => r.scope !== "hub-global");
    case "hub_list_skills":
      return rows.filter((r) => r.scope === "hub-global");
    case "get_agents_table":
      return agentTable;
    case "get_projects":
      return [];
    case "hub_status_cmd":
      return {
        scope: "global",
        path: "~/.skillmint/hub",
        exists: true,
        git_inited: true,
        skill_count: 1,
        remote: null,
        branch: "main",
        ahead: 0,
        behind: 0,
        dirty_files: 0,
        last_commit: null,
      };
    default:
      return null;
  }
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => Promise.resolve(routeInvoke(cmd)));
});

describe("InstalledSkills", () => {
  it("renders index rows with provenance badges and agent chips", async () => {
    render(<InstalledSkills />);
    await waitFor(() => {
      expect(screen.getByText("pdf")).toBeInTheDocument();
    });
    expect(screen.getByText("hand-made")).toBeInTheDocument();
    expect(screen.getByText("my-own-skill")).toBeInTheDocument();
    // Managed-by labels distinguish the three provenances.
    expect(screen.getAllByText("npx 管理").length).toBeGreaterThan(0);
    expect(screen.getAllByText("未管理").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Hub 创作").length).toBeGreaterThan(0);
    // Agent keys are displayed via the static table's display names.
    expect(screen.getAllByText("Claude Code").length).toBeGreaterThan(0);
    expect(screen.getAllByText("ZCode").length).toBeGreaterThan(0);
  });

  it("only offers collect-to-hub for unmanaged rows and update/remove for npx rows", async () => {
    const user = userEvent.setup();
    render(<InstalledSkills />);
    await waitFor(() => expect(screen.getByText("pdf")).toBeInTheDocument());

    // npx row: update + remove available.
    const pdfRow = screen.getByText("pdf").closest("li");
    expect(pdfRow).not.toBeNull();
    expect(pdfRow!.textContent).toContain("更新");
    expect(pdfRow!.textContent).toContain("卸载");

    // unmanaged row: collect offered, no remove (the CLI owns the lifecycle).
    const handRow = screen.getByText("hand-made").closest("li");
    expect(handRow!.textContent).toContain("收集到 Hub");
    expect(handRow!.textContent).not.toContain("卸载");

    // Hub row: authored content, no npx lifecycle actions.
    const hubRow = screen.getByText("my-own-skill").closest("li");
    expect(hubRow!.textContent).not.toContain("卸载");

    void user;
  });

  it("remove asks for confirmation and routes through npx_remove", async () => {
    const user = userEvent.setup();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<InstalledSkills />);
    await waitFor(() => expect(screen.getByText("pdf")).toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: /卸载/ }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "npx_remove",
        expect.objectContaining({ skill: "pdf", global: true }),
      );
    });
    confirmSpy.mockRestore();
  });

  it("remove keeps the skill when the confirmation is dismissed", async () => {
    const user = userEvent.setup();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<InstalledSkills />);
    await waitFor(() => expect(screen.getByText("pdf")).toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: /卸载/ }));
    expect(invokeMock).not.toHaveBeenCalledWith("npx_remove", expect.anything());
    confirmSpy.mockRestore();
  });
});
