# 积微 · SkillMint

> **把每一次重复，铸成会复利的技能资产。**
> *Mint your AI experience into compounding skill assets.*

[English](#english) · [中文](#中文)

---

## English

SkillMint is a local-first macOS app that turns your everyday AI coding sessions into reusable, measurable `SKILL.md` assets — collected, distilled, synced across Cursor / Claude Code / Codex and 10+ agents, with usage-based ROI.

### Why

Every day you repeat yourself to AI assistants: the same project conventions, the same review checklist, the same debugging ritual. Those repetitions are experience — but until now they evaporated the moment a session closed. SkillMint captures them as durable `SKILL.md` files, versioned in one local repository, and syncs them back into every agent you use.

### Features

- **Collect** — Read-only collection of usage data from 10+ AI agents (Cursor, Claude Code, Codex, ZCode, Kimi, …) running on your machine.
- **Distill** — Discover repeat patterns and high-value prompts; turn them into first-class skills with YAML frontmatter.
- **Sync** — One central local repository (`~/.skillmint/repo`) is symlinked into every agent's own skill directory, so an edit lands everywhere.
- **Measure** — Per-skill usage stats, token spend, and ROI, so you know which skills actually compound.
- **Local-first** — Zero servers. All data stays on your machine. Remote sources are opt-in.

### Supported Agents

Cursor · Claude Code · Codex · ZCode · Kimi · Gemini CLI · ACP-compatible CLIs · and more (see the in-app Agents page for the full, auto-detected list).

### Install

Pre-built binaries are planned. For now, build from source:

```sh
git clone https://github.com/<org>/skillmint.git
cd skillmint
make build-install   # builds, installs to /Applications, and verifies
```

Requirements: macOS 14+, Node 22 (via nvm), Rust stable, `rsync`.

### Privacy

SkillMint runs entirely on your device. It opens no outbound network connection unless you explicitly enable a remote skill source. Your sessions, prompts, and tokens never leave your machine. See `AGENTS.md` for the local data layout.

### Roadmap

- [ ] v0.2 — Cross-agent skill deduplication & merge suggestions
- [ ] v0.2 — Skill knowledge graph
- [ ] v0.3 — Opt-in remote skill bundles (pull-only, signed)
- [ ] v0.3 — Multi-machine sync via your own Git remote

### License

Apache License 2.0. See [`LICENSE`](./LICENSE).

---

## 中文

积微(SkillMint):本地优先的 macOS 应用,自动采集 Cursor / Claude Code / Codex 等 10+ AI Agent 的使用数据,把重复的高价值经验沉淀为 `SKILL.md` 资产,统一版本化并同步回所有 Agent,用真实调用数据度量每个 Skill 的复利回报。

### 为什么需要

你每天都在向 AI 助手重复同一件事:项目约定、代码评审清单、调试流程……这些重复本身就是经验,但会话一结束就蒸发了。积微把它们固化成版本化的 `SKILL.md`,集中存放在本地中心仓库,再软链回每个 Agent 的 skill 目录,一次编辑,处处生效。

### 功能

- **采集** — 只读采集本机 10+ AI Agent 的使用数据(Cursor、Claude Code、Codex、ZCode、Kimi……)。
- **沉淀** — 自动发现重复模式与高价值 Prompt,一键转化为带 YAML frontmatter 的标准 Skill。
- **同步** — 中心仓库 `~/.skillmint/repo` 通过软链分发到所有 Agent 目录,编辑一次,处处生效。
- **度量** — 按 Skill 统计调用次数、Token 消耗与 ROI,让复利可见。
- **本地优先** — 零服务端,数据全部留在本机;远程源为可选项,默认不发起任何网络请求。

### 支持的 Agent

Cursor · Claude Code · Codex · ZCode · Kimi · Gemini CLI · 兼容 ACP 的 CLI · 以及更多(应用内「Agent」页会自动探测已安装的 Agent)。

### 安装

```sh
git clone https://github.com/<org>/skillmint.git
cd skillmint
make build-install   # 构建 + 安装到 /Applications + 自动校验
```

环境要求:macOS 14+、Node 22(通过 nvm)、Rust stable、`rsync`。

### 隐私

积微完全运行在你的设备上。除非你主动开启远程 skill 源,否则不会发起任何对外网络请求;会话、Prompt、Token 数据绝不离开本机。本地数据布局见 `AGENTS.md`。

### 路线图

- [ ] v0.2 — 跨 Agent skill 去重与合并建议
- [ ] v0.2 — Skill 知识图谱
- [ ] v0.3 — 可选的远程 skill bundle(只读拉取,签名校验)
- [ ] v0.3 — 通过自有 Git 远程实现多机同步

### 协议

Apache License 2.0,见 [`LICENSE`](./LICENSE)。

---

> 「积微成著」—— 微小的经验持续沉淀,终成显著的能力。
