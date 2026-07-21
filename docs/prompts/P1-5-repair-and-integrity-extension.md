# P1-5：`repair_paths` / `check_repo_integrity` 扩展——自动发现并治愈"改名+孤儿+legacy 残留"

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P1-5 节、commands.rs:60（migrate_skills_to_repo）与 :151（dirs_equivalent）、:203（check_repo_integrity 现状：只查"DB 有但 center 缺"）、src/bin/repair_paths.rs、P1-4 完成的 rename_skill 命令。

背景：生产环境实测，repair_paths 对最常见漂移无能为力， migrated 0：
- 目录被改名（DB repo_path 悬空 + center 里有等价新目录）——本次 6 个；
- center repo 里有未登记目录（孤儿）——本次手工用 comm 比对发现 22 个；
- 上一代 ~/.skillsync 布局残留——本次实清 61 条悬空链 + agent 目录里 6 条指向它的断链。

前置：P1-4（rename_skill）已完成。

任务（全部默认 dry-run 出报告，--apply 才执行）：
1. 改名配对：扫描 DB 中 repo_path 不存在的行 × center repo 内未登记目录，用目录等价（参考 dirs_equivalent，可升级为 compute_hash 比对）给出配对建议；--apply 时逐对调 rename_skill 完成迁移。配不出唯一候选的，列入 failed 并说明，不得猜。
2. 孤儿发现：center repo 内未登记目录（且通过 is_skill_dir 判定，复用 P0-3 的公共函数）→ 报告；--apply 走与 import 相同的登记流程（insert skill + sync targets）。
3. legacy 残留检测：发现 ~/.skillsync 等上一代布局与 agent 目录里指向它们的断链 → 报告并给出清理建议（--apply 仅删除已确认悬空的 symlink，不删任何实体目录）。
4. check_repo_integrity 增加反向检查（center 有但 DB 无）并复用上述报告格式。
5. repair_paths 二进制同步获得这些能力（它与 repair_skill_paths 命令本就共用实现，保持如此）。

验收：在测试环境制造一次改名 + 一个孤儿目录 + 一个 legacy 悬空链，dry-run 报告三项齐全且配对正确；--apply 后 DB 与磁盘收敛一致、无悬空链。cargo test 全绿（为配对/孤儿/legacy 三类各补用例）。
完成后：commit（引用 P1-5）；在 docs/OPTIMIZATION-2026-07.md P1-5 标题后标 ✅ 与日期。
```
