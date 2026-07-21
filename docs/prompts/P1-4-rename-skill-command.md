# P1-4：新增官方 `rename_skill` 命令

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P1-4 节与附 A runbook（已验证的修复 recipe，直接当 spec）、skillmint/src-tauri/src/commands.rs 的 import_skill（:730）与 sync_single_skill_command（:3447）、db.rs 的 update_skill(:1965)/insert_sync_target(:2238)/update_sync_target_status(:2280)、fs.rs 的 create_symlink_or_copy。

背景：用户在 app 外重命名 skill 目录是真实场景（一次改 9 个）。app 没有改名能力，结果是 DB 记录指向不存在的旧路径、8 个 agent 目录里几十条悬空旧名链。本次手工修复已验证正确语义：保 id 改名（kg_skill_nodes、sync_targets 的 FK 引用全保）+ 重写各 farm 软链 + 刷新同步状态。

前置：P0-1 已完成（同步引擎不会在此过程中对缺失源建链）。

任务：新增 Tauri 命令 rename_skill(old_name, new_name)：
1. 前置校验：old_name 在 DB 存在、new_name 不与现有 skill/目录冲突、center 里 old_name 目录存在；任一不满足，明确报错、不做任何变更。
2. 文件操作：center 目录 mv 为新名；若该 skill 的 SKILL.md front matter name 与目录名一致，同步改写 name 字段。
3. 每个 sync target 对应的 agent 目录：删除旧名软链，建新名软链（复用现有 fs 原语；copy 模式的 target 只做目录改名，不切 symlink）。
4. DB（单事务）：UPDATE skills SET name, repo_path WHERE name=old_name（保 id）；sync_targets 状态由修好的 evaluate 逻辑重算刷新。
5. 失败补偿：文件操作中途失败时，尽力把已改名的目录/链接改回旧名并返回错误；DB 变更必须整体回滚。
6. 前端（可选但鼓励）：Skills 页编辑入口加"重命名"，调该命令。

验收：src/tests.rs 新增用例覆盖成功路径、冲突报错、中途失败回滚；手动——对一个已同步到 3 个 agent 的 skill 执行 rename，DB 记录、各 farm 软链、同步状态全部指向新名，无悬空链。cargo test + npm run test:ci 全绿。
完成后：commit（引用 P1-4）；在 docs/OPTIMIZATION-2026-07.md P1-4 标题后标 ✅ 与日期。
```
