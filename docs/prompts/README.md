# 优化方案执行 Prompts

配合 [`../OPTIMIZATION-2026-07.md`](../OPTIMIZATION-2026-07.md) 使用。每个文件是一条**自包含 prompt**，粘给该项目里的编码 agent（Kimi Code / Claude Code / Codex 等）即可开工。建议**每项一个新会话**，避免上下文污染。

## 执行顺序与依赖

| 顺序 | Prompt | 依赖 | 说明 |
|---|---|---|---|
| 1 | [P0-1](P0-1-sync-no-dangling-links.md) | 无 | 同步引擎地基，后续多项引用它 |
| 2 | [P0-2](P0-2-symlink-hash-semantics.md) | 建议 P0-1 后 | 与 P0-3 改同一批扫描函数，不要并行 |
| 3 | [P0-3](P0-3-import-filtering.md) | P0-2 后 | 同上 |
| 4 | [P1-1](P1-1-deeplink-window-wakeup.md) | 无 | |
| 5 | [P1-2](P1-2-window-lifecycle-tray.md) | P1-1 后 | 同改 lib.rs 窗口处理 |
| 6 | [P1-3](P1-3-import-modal-dropdown-a11y.md) | 无 | 纯前端，可任意时间并行 |
| 7 | [P1-4](P1-4-rename-skill-command.md) | P0-1 后 | 复用修好的同步语义 |
| 8 | [P1-5](P1-5-repair-and-integrity-extension.md) | P1-4 后 | `--apply` 调 rename_skill |
| 9 | [P2-1](P2-1-git-integration.md) | 建议 P0-3 后 | auto-commit 与导入流程相关 |
| 10 | [P2-2](P2-2-packaging-engineering.md) | 无（建议最后） | 打包/拆分/CI/文档治理 |

想让一个 agent 全程串行推进，用 [00-orchestrator](00-orchestrator.md) 作为总控 prompt。

## 通用约定（已写入各 prompt，此处备查）

- 开工先读：`AGENTS.md`、`CONTRIBUTING.md`、`docs/OPTIMIZATION-2026-07.md` 对应章节。
- local-first：不引入任何外发网络行为，不读写 `~/.skillmint` 之外的路径。
- 完成标准：Rust `cargo test` 与前端 `npm run test:ci` 双全绿；涉及打包的改动另跑 `make build-install verify`。
- 每项一个 commit；完成后在 `docs/OPTIMIZATION-2026-07.md` 对应章节标题后标注 `✅ (YYYY-MM-DD)`。
