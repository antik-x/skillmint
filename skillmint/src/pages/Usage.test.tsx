import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useCollectionStore } from "../stores/collectionStore";
import { invoke } from "@tauri-apps/api/core";
import Usage, { cleanPromptText, sourceLabel } from "./Usage";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function mockWindowMetrics() {
  return {
    kind: "week",
    ref_date: "2026-07-02",
    current_window: { start: "2026-06-26", end: "2026-07-02" },
    previous_window: { start: "2026-06-19", end: "2026-06-25" },
    yoy_window: { start: "2025-06-27", end: "2025-07-03" },
    has_previous_baseline: true,
    has_yoy_baseline: false,
    comparison: {
      sessions: { current: 10, mom: { current: 10, previous: 8, delta: 2, pct: 0.25 } },
      total_tokens: { current: 1000, mom: { current: 1000, previous: 800, delta: 200, pct: 0.25 } },
      est_cost_cny: { current: 5, mom: { current: 5, previous: 4, delta: 1, pct: 0.25 } },
      quality_score: { current: 80, mom: { current: 80, previous: 75, delta: 5, pct: 0.0667 } },
    },
    token_dimension: {
      distribution: { by_platform: {}, by_project: {}, by_model: {} },
      cost: { est_cost_cny: 5, by_platform_cny: {}, billing_mix: {} },
      diagnostics: { cache_ratio: 0, heavy_sessions: [] },
    },
    prompt_dimension: {
      penetration: { total_prompts: 0, by_platform: {}, by_project: {} },
      semantics: {
        classified_ratio: 0,
        requested_action: {},
        target_object: {},
        interaction_state: {},
        interaction_mode: {},
      },
      quality: {
        score: 80,
        clarification_correction_rate: 0,
        planning_ratio: 0,
        test_object_ratio: 0,
        improvement_suggestions: [],
      },
    },
    leverage: {
      est_cost_cny: 5,
      variable_cost_cny: 5,
      subscription_cost_cny: 0,
      fresh_tokens: 1000,
      output_proxy: 0,
      leverage_per_cny: 0,
      cost_per_prompt_cny: 0,
    },
  };
}

function resetStore() {
  useCollectionStore.setState({
    sources: [{ source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 }],
  });
}

describe("Usage page", () => {
  beforeEach(() => {
    resetStore();
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_collection_status") {
        return Promise.resolve([
          { source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 },
        ]);
      }
      if (cmd === "get_window_metrics") return Promise.resolve(mockWindowMetrics());
      if (cmd === "get_high_value_prompts") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
  });

  it("passes days: 0 to get_agent_usage when '全部' range is selected", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string, args?: import("@tauri-apps/api/core").InvokeArgs) => {
      if (cmd === "get_collection_status") {
        return Promise.resolve([
          { source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 },
        ]);
      }
      if (cmd === "get_window_metrics") return Promise.resolve(mockWindowMetrics());
      if (cmd === "get_high_value_prompts") return Promise.resolve([]);
      if (cmd === "get_agent_usage") {
        const a = args as Record<string, unknown> | undefined;
        return Promise.resolve({ source: a?.source, days: a?.days });
      }
      return Promise.resolve(undefined);
    });

    render(<Usage />);
    await waitFor(() => expect(screen.getByText("按数据源")).toBeInTheDocument());

    const allButton = screen.getByRole("button", { name: "全部" });
    await userEvent.click(allButton);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("get_agent_usage", { source: "claude-code", days: 0 });
    });
  });

  // SPEC-F5 T3: sediment dialog pulls the suggested name/description from the
  // backend `preview_skill_from_prompt` (tokenized name) instead of a naive
  // client-side char filter; on validation failure the confirm button is
  // disabled and the reason is shown.
  it("SedimentDialog uses backend preview name/description", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_collection_status") {
        return Promise.resolve([
          { source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 },
        ]);
      }
      if (cmd === "get_window_metrics") return Promise.resolve(mockWindowMetrics());
      if (cmd === "get_high_value_prompts") {
        return Promise.resolve([
          {
            prompt_text: "Continue from where you left off and finish the task",
            source: "claude-code",
            repeat_count: 5,
            first_seen: 1,
            last_seen: 2,
            sample_session_id: "s1",
          },
        ]);
      }
      if (cmd === "preview_skill_from_prompt") {
        return Promise.resolve({
          name: "continue-from-where-you-left-off",
          description: "Continue from where you left off.\n\n适用场景：复现该工作流。",
        });
      }
      if (cmd === "get_agent_usage") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<Usage />);
    await waitFor(() => expect(screen.getByText("查看并沉淀")).toBeInTheDocument());
    await userEvent.click(screen.getByText("查看并沉淀"));

    // Confirm button is enabled (preview loaded) and the input shows the tokenized name.
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("preview_skill_from_prompt", { promptText: "Continue from where you left off and finish the task" }));
    const nameInput = await screen.findByDisplayValue("continue-from-where-you-left-off");
    expect(nameInput).toBeInTheDocument();
  });

  it("SedimentDialog disables confirm and shows reason when preview fails validation", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_collection_status") {
        return Promise.resolve([
          { source: "claude-code", collector_kind: "claude", data_path: "/x", status: "ok", record_count: 10 },
        ]);
      }
      if (cmd === "get_window_metrics") return Promise.resolve(mockWindowMetrics());
      if (cmd === "get_high_value_prompts") {
        return Promise.resolve([
          {
            prompt_text: "[Image: only.png]",
            source: "claude-code",
            repeat_count: 5,
            first_seen: 1,
            last_seen: 2,
            sample_session_id: "s1",
          },
        ]);
      }
      if (cmd === "preview_skill_from_prompt") {
        return Promise.reject(new Error("此 Prompt 内容无法沉淀为 Skill（占位符占比过高）"));
      }
      if (cmd === "get_agent_usage") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<Usage />);
    await waitFor(() => expect(screen.getByText("查看并沉淀")).toBeInTheDocument());
    await userEvent.click(screen.getByText("查看并沉淀"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("preview_skill_from_prompt", { promptText: "[Image: only.png]" })
    );
    // Confirm button must be disabled while error reason is surfaced.
    await waitFor(() => {
      const confirmButton = screen.getByText("在编辑器中确认并创建").closest("button");
      expect(confirmButton).toBeDisabled();
    });
    // The error container (humanized reason) must be rendered inside the dialog.
    await waitFor(() => {
      const dialog = document.querySelector(".border-danger\\/20");
      expect(dialog).not.toBeNull();
    });
  });
});

describe("Usage page helpers", () => {
  describe("cleanPromptText", () => {
    it("strips HTML-like tags and local-command-caveat markers", () => {
      const raw = "<system>important</system> do this local-command-caveat now";
      const { display } = cleanPromptText(raw);
      expect(display).toBe("important do this now");
    });

    it("collapses consecutive whitespace", () => {
      const raw = "line1\n\n  line2\t\tline3";
      const { display } = cleanPromptText(raw);
      expect(display).toBe("line1 line2 line3");
    });

    it("returns the full text when it is within the limit", () => {
      const raw = "short prompt";
      const { display, truncated } = cleanPromptText(raw);
      expect(display).toBe("short prompt");
      expect(truncated).toBe(false);
    });

    it("truncates text longer than the limit and reports truncation", () => {
      const raw = "a".repeat(200);
      const { display, truncated } = cleanPromptText(raw);
      expect(display.length).toBeLessThan(200);
      expect(display.endsWith("a")).toBe(true);
      expect(truncated).toBe(true);
    });

    it("allows expanding with an infinite limit", () => {
      const raw = "a".repeat(500);
      const { display, truncated } = cleanPromptText(raw, Infinity);
      expect(display).toBe(raw);
      expect(truncated).toBe(false);
    });

    // SPEC-F5 T2: agent-injected human-readable noise must be stripped.
    it("strips Caveat: sentences", () => {
      const { display } = cleanPromptText("Do the thing. Caveat: this is noise. Keep going.");
      expect(display).toBe("Do the thing. Keep going.");
    });

    it("strips bracket placeholders ([Request interrupted] / [Image] / [File] / [Attachment])", () => {
      const { display } = cleanPromptText(
        "Refactor X [Image: foo.png] then [File: bar.ts] [Attachment: baz.zip] done [Request interrupted by user]"
      );
      expect(display).toBe("Refactor X then done");
    });

    it("strips parenthesized tool echoes ((Bash completed...) / (Tool ...))", () => {
      const { display } = cleanPromptText("Run tests (Bash completed in 2s) verify (Tool: grep)");
      expect(display).toBe("Run tests verify");
    });

    it("strips The TodoWrite tool lines", () => {
      const { display } = cleanPromptText("First line\nThe TodoWrite tool was called with args\nThird line");
      expect(display).toBe("First line Third line");
    });

    it("falls back to original text when everything is noise", () => {
      const raw = "[Image: only.png] (Tool: something)";
      const { display } = cleanPromptText(raw);
      // No empty card — at least the bracket/paren-stripped skeleton is shown.
      expect(display.length).toBeGreaterThan(0);
    });

    // SPEC-F5 T2 regression: the module-level `/g` noise regexes must not leak
    // `lastIndex` across calls — a long render of many prompts must clean every
    // prompt consistently, including all-noise short prompts after a full match.
    it("stays consistent across many sequential calls (global-regex state reset)", () => {
      const inputs = [
        "Refactor the authentication module to use JWT. Caveat: do not break sessions. (Bash completed in 2s) [Image: diagram.png]",
        "The TodoWrite tool hasn't been used recently.\nDo the actual task please.",
        "(Bash completed with no output)",
        "[Image: x.png]",
        "(Bash completed with no output)",
      ];
      const results = inputs.map((s) => cleanPromptText(s, 160).display);
      expect(results[0]).toBe("Refactor the authentication module to use JWT.");
      expect(results[1]).toBe("Do the actual task please.");
      // Pure-noise short prompts fall back to the original (tag-stripped), never empty.
      expect(results[2].length).toBeGreaterThan(0);
      expect(results[3].length).toBeGreaterThan(0);
      expect(results[4]).toBe(results[2]);
    });
  });

  describe("sourceLabel", () => {
    it("maps known agent sources to friendly names", () => {
      expect(sourceLabel("claude-code")).toBe("Claude Code");
      expect(sourceLabel("codex")).toBe("Codex");
      expect(sourceLabel("zcode")).toBe("ZCode");
      expect(sourceLabel("cursor")).toBe("Cursor");
    });

    it("returns the raw source when unknown", () => {
      expect(sourceLabel("custom-agent")).toBe("custom-agent");
    });

    it("falls back to 未知来源 when source is missing", () => {
      expect(sourceLabel(undefined)).toBe("未知来源");
      expect(sourceLabel("")).toBe("未知来源");
    });
  });
});
