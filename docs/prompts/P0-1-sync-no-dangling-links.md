# P0-1：同步引擎禁止对不存在的 center 源建链

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：AGENTS.md、docs/OPTIMIZATION-2026-07.md 的 P0-1 节、skillmint/src-tauri/src/sync.rs（重点 evaluate_sync_target :15、apply_sync_target :59、sync_all :139）、fs.rs:10、src/tests.rs 中 sync 相关用例。

背景（真实事故）：center repo 里 9 个 skill 目录被改名后，app 的自动同步按过期 sync_targets 重建了 18 条软链——名字是旧名、指向已不存在的 repo/<旧名>，app 自己制造了悬空软链并计入同步健康。

现状缺陷：
1. apply_sync_target 先无条件 remove_path(agent_path)，再 create_symlink_or_copy，创建前不校验 center 源是否存在；Unix 下 symlink() 对不存在的源也会成功 → 悬空链。
2. evaluate_sync_target 已能判出 Broken，但 sync_all 对 Broken 状态仍走 apply。
3. symlink 失败静默降级为整目录 copy，用户无感知。

任务：
1. apply_sync_target 入口加 center_path.exists() 检查：源缺失时不触碰 agent 侧任何文件，只把该 target 状态置为 Broken 并计入 SyncReport.broken（带 target 名与原因）。
2. symlink 创建失败不再静默降级 copy；失败计入 SyncReport 并在返回值中可见（copy 仅在 mode=copy 的 target 上发生）。
3. 补单测：a) center 目录被改名/删除后 sync_all 不产生悬空链、agent 侧已有链接原样保留、状态=Broken；b) 正常 synced 场景不回归；c) symlink 失败路径有报告。

边界：
- 不改 SyncStatus 枚举语义（Broken 定义不变）。
- 不触碰项目级同步 resolve_skill_link / resolve_skill_diff 路径。

验收：cargo test 全绿；手动验证——把 center 里任一 skill 目录改名，跑 sync_all_command，各 agent 目录无新增悬空链，get_sync_targets 显示 Broken。
完成后：npm run test:ci 确认前端无回归；commit（引用 P0-1）；在 docs/OPTIMIZATION-2026-07.md P0-1 标题后标 ✅ 与日期。
```
