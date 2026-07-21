# P2-2：打包与工程治理

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P2-2 节、tauri.conf.json（bundle 配置）、Makefile（build-install verify）、CONTRIBUTING.md。

本项含四个独立子任务，每个子任务一个 commit（可拆多个 PR，按 a→d 顺序）：

a) bundle 资源缺口：
   - tauri.conf.json 未声明 tray-normal.png / tray-warning.png（update_tray_status 从 exe ../Resources 读图），也未给 repair_paths 声明 externalBin——分发后托盘双态与修复工具都会缺失。
   - 补 bundle resources / externalBin 声明；在 make build-install verify（或其脚本）里加断言：两个 png 与 Contents/MacOS/repair_paths 存在。
   - 跑 make build-install verify 实证。

b) god-module 拆分：commands.rs（5313 行）、db.rs（约 5000 行）按 sync / import / scan / agents / git / trash 拆分子模块。纯搬运：不改任何行为、不改命令签名；每拆一个域跑一遍 cargo test。

c) CI：.github/ 目前只有 ISSUE_TEMPLATE。加 GitHub Actions workflow（macOS runner）：cargo test、npm run test:ci、make build-install verify（或至少 tauri build 冒烟）。

d) 文档治理：代码注释引用的 PRD-xx/SPEC-Fx 规范不在仓库里。二选一并说明理由：把可公开的规范收进 docs/specs/；或把注释里的引用改为就地描述（去外部引用化）。docs/ 目录以 OPTIMIZATION-2026-07.md 与 prompts/ 为起点，保持新增文档入 docs/。

验收：
- a) make build-install verify 通过且断言生效（临时移走一个资源能验出失败）。
- b) 拆分后 cargo test + npm run test:ci 全绿，diff 不含行为变更。
- c) CI 在 PR 上跑通。
- d) 仓库内不再出现指向不存在文档的悬空引用（或 docs/specs/ 就位）。
完成后：每个子任务单独 commit（引用 P2-2-a/b/c/d）；全部完成后再在 docs/OPTIMIZATION-2026-07.md P2-2 标题后标 ✅ 与日期。
```
