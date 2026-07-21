# P0-3：扫描/导入过滤——非 skill 目录不得出现

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P0-3 节、commands.rs:557（scan_agent_skills）、:662（scan_directory_skills_inner）、:417（import_all_agent_skills）、前端 skillmint/src/components/ImportSkillModal.tsx。

背景（真实事故）：~/.agents/skills 下的 cache、data、marketplaces（指向非 skill 目录的软链）在导入列表显示为「新 Skill」，其中 marketplaces（8.6MB、含内嵌 .git 的插件市场镜像）被误导入 center 并登记，事后只能手工回滚（删 repo 拷贝、清 DB、恢复软链）。

现状缺陷：三处扫描/发现逻辑只要 read_dir 里 is_dir() 就算 skill——不校验 SKILL.md 是否存在、不排除软链目录、无排除名单。

任务：
1. 抽公共判定函数（建议放 fs.rs 或 scan.rs）：is_skill_dir(path) = 目录（或 symlink 解析后的目标）包含 SKILL.md。scan_agent_skills、scan_directory_skills_inner、import_all_agent_skills 三处统一改用该判定。
2. 内置排除名单：cache、data、marketplaces、node_modules、.git、.trash；并在设置中允许用户追加（落在 settings.json，带默认值）。
3. ImportSkillModal.tsx 加批量操作：「全选 / 仅选"新 Skill" / 清空」（真实痛点：用户手动勾了 25 个框）。
4. 测试：Rust 侧——无 SKILL.md 目录、指向非 skill 目录的软链、排除名单命中，三种场景扫描结果为空；前端——批量按钮行为。

边界：
- 不改变"内容冲突/与中心一致"的既有判定语义（P0-2 已处理的软链短路保持有效）。
- 与 P0-2 串行，分开 commit。

验收：cargo test + npm run test:ci 全绿；手动——在 agent 目录放一个无 SKILL.md 的目录和一条指向 ~/.skillmint/cache 的软链，导入列表不含它们。
完成后：commit（引用 P0-3）；在 docs/OPTIMIZATION-2026-07.md P0-3 标题后标 ✅ 与日期。
```
