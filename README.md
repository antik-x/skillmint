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
- **Install (npx skills)** — Installs, updates, removals and registry search all run the real [`skills` CLI](https://skills.sh) (`npx skills add/update/remove`) across its 77-agent matrix — project or global scope, with the exact equivalent command shown before every operation.
- **Private hubs** — Author personal skills in `~/.skillmint/hub` (own git, push to a private remote) or team skills in `<project>/.skillmint/hub` (committed with the project); install them anywhere via `npx skills add <hub>`.
- **Measure** — Per-skill usage stats, token spend, and ROI, so you know which skills actually compound.
- **Local-first** — Zero servers. All data stays on your machine. Network features (skills.sh search, git installs) are opt-in; npx telemetry is disabled by default.

### Supported Agents

The full `npx skills` matrix is built in — 77 agents including Claude Code (`.claude/skills`), Codex, Cursor, ZCode (`.zcode/skills`), OpenCode, Windsurf, Gemini CLI, GitHub Copilot, Cline, Kimi and more. Project-scope installs share the `.agents/skills` canonical store with per-agent symlinks; SkillMint auto-detects anything the CLI installs (via its lock files) the moment the app regains focus.

### Install

Pre-built binaries are planned. For now, build from source:

```sh
git clone https://github.com/<org>/skillmint.git
cd skillmint
make build-install   # builds, installs to /Applications, and verifies
```

Requirements: macOS 14+, Node ≥ 22.20 (via nvm; `npx skills` requires it), Rust stable, `rsync`, `git`.

### Privacy

SkillMint runs entirely on your device. It opens no outbound network connection unless you explicitly enable a remote skill source. Your sessions, prompts, and tokens never leave your machine. See `AGENTS.md` for the local data layout.

### Roadmap

- [ ] v0.2 — Cross-agent skill deduplication & merge suggestions
- [ ] v0.2 — Skill knowledge graph
- [ ] v0.3 — Opt-in remote skill bundles (pull-only, signed)
- [x] v0.3 — Multi-machine sync via your own Git remote (global private hub: push/pull through any git remote, install with `npx skills add <url>`)

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
- **安装(npx skills)** — 安装、更新、卸载与搜索全部通过真实 [`skills` CLI](https://skills.sh)(`npx skills add/update/remove`)完成,覆盖其 77 个 Agent 矩阵;支持项目级/全局两种作用域,每次操作前都会展示等价的终端命令。
- **私有 Hub** — 个人技能创作于 `~/.skillmint/hub`(独立 git,可推送私有远端),团队技能放 `<project>/.skillmint/hub`(随项目仓库提交);任何位置都可用 `npx skills add <hub>` 安装。
- **度量** — 按 Skill 统计调用次数、Token 消耗与 ROI,让复利可见。
- **本地优先** — 零服务端,数据全部留在本机;联网功能(skills.sh 搜索、git 安装)均为可选项,npx 遥测默认关闭。

### 支持的 Agent

内置 `npx skills` 全量矩阵 —— 77 个 Agent,包括 Claude Code(`.claude/skills`)、Codex、Cursor、ZCode(`.zcode/skills`)、OpenCode、Windsurf、Gemini CLI、GitHub Copilot、Cline、Kimi 等。项目级安装共享 `.agents/skills` canonical 目录并向各 Agent 建符号链接;CLI 装的技能会通过其 lock 文件被 app 自动扫到(窗口聚焦即刷新)。

### 安装

```sh
git clone https://github.com/<org>/skillmint.git
cd skillmint
make build-install   # 构建 + 安装到 /Applications + 自动校验
```

环境要求:macOS 14+、Node ≥ 22.20(通过 nvm,`npx skills` 硬性要求)、Rust stable、`rsync`、`git`。

### 隐私

积微完全运行在你的设备上。除非你主动开启远程 skill 源,否则不会发起任何对外网络请求;会话、Prompt、Token 数据绝不离开本机。本地数据布局见 `AGENTS.md`。

### 路线图

- [ ] v0.2 — 跨 Agent skill 去重与合并建议
- [ ] v0.2 — Skill 知识图谱
- [ ] v0.3 — 可选的远程 skill bundle(只读拉取,签名校验)
- [x] v0.3 — 通过自有 Git 远程实现多机同步(全局私有 Hub:git 推拉,`npx skills add <url>` 安装)

### 协议

Apache License 2.0,见 [`LICENSE`](./LICENSE)。

---

> 「积微成著」—— 微小的经验持续沉淀,终成显著的能力。
