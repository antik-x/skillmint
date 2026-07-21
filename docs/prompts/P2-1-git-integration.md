# P2-1：git 集成——导入后自动提交 + 内嵌仓防护

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P2-1 节、commands.rs:5012（run_git）、:5029（git_init_repo）、:5048（git_commit）、import_skill（:730）。

背景：导入 24 个 skill 目录后 center repo 留下一堆未跟踪文件，需手工提交；手工 git add -A 时 archify（内嵌 git 仓）变成无效 gitlink，marketplaces 里也有内嵌 .git。repo 现有 .gitignore 是靠手工补 node_modules/、__pycache__/、archify/ 才补齐的。

任务：
1. 新设置项 auto_commit_after_import（默认关，落在 settings.json + 设置页开关）：import_skill / import_all_agent_skills 成功后自动 git_commit，message 含本次导入的 skill 清单（祈使句）。
2. git add 前置防护：add 前扫描待加路径中的内嵌 .git 目录；命中则不 add 该路径、自动把该顶层目录写入 .gitignore，并在返回值/UI 提示用户（可选择后续 git submodule add）。不得形成 gitlink。
3. git_init_repo 初始化时生成 .gitignore 模板：.DS_Store、node_modules/、__pycache__/。
4. 测试：用临时 git repo 验证——含内嵌 .git 的目录不会被 add、.gitignore 被正确追加；auto-commit 开关关时不产生 commit。

边界：
- 继续用 run_git shell-out，不引入 git2 等库依赖。
- 不改变既有 git_push/git_pull/git_versions 命令行为。

验收：cargo test 全绿；手动——开 auto-commit 导入 3 个 skill，repo 自动产生一条含清单的 commit；含内嵌 .git 的目录被跳过且 .gitignore 有记录。
完成后：commit（引用 P2-1）；在 docs/OPTIMIZATION-2026-07.md P2-1 标题后标 ✅ 与日期。
```
