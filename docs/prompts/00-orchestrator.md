# 总控 Prompt：串行推进 OPTIMIZATION-2026-07

```text
你是本仓库（/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource）的高级 Rust/Tauri 工程师。

先完整阅读：
1. AGENTS.md、CONTRIBUTING.md（工程约定与验证命令）
2. docs/OPTIMIZATION-2026-07.md（优化方案全文，含每项的现状定位与验收标准）
3. docs/prompts/README.md（执行顺序与依赖）

任务：按 docs/prompts/README.md 的顺序逐项推进优化方案。对每一项：
1. 先读该项指定的源码位置，向我复述现状与你的改动计划，等我确认后再动手（P0 项必须等确认；P1/P2 可直接继续）。
2. 实现 + 补齐测试（Rust 单测进 src/tests.rs 或对应模块 #[cfg(test)]，前端进 vitest）。
3. 跑 cargo test 与 npm run test:ci，双全绿后做一个 commit（祈使句，说明动机，引用 docs/OPTIMIZATION-2026-07.md 条目号）。
4. 在 docs/OPTIMIZATION-2026-07.md 对应章节标题后标注 ✅ 与日期。
5. 简要汇报：改了什么、验证结果、下一项是什么。

硬约束：
- local-first，不引入外发网络行为；不读写 ~/.skillmint 之外的路径。
- 不并行做相互依赖的项；P0-2 与 P0-3 必须串行。
- 打包相关改动（P2-2）额外跑 make build-install verify。
- 遇到方案与现实不符（如代码已漂移），停下来报告差异，不要自行改方案范围。
```
