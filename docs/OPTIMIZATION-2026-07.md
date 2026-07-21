# SkillMint 优化方案：同步正确性、导入过滤与窗口可靠性

> 来源：2026-07-21 一次真实排障。当天在一台生产机上完成了三件事：① 把 9 个 `wechat-pipeline` 相关 skill 改名为 `whyactions-*` 前缀并收编进中心仓；② 通过「从 Agent 导入」收编 22 个孤儿 skill；③ 手工修复改名后 SkillMint 留下的全部烂摊子（6 行过期 DB 记录、8 个 agent 目录里 60+ 条悬空软链、app 按过期记录重建的 18 条断链）。
>
> 本文把排障中暴露的问题逐条映射到代码位置，给出修复方案与验收标准。每条均可独立推进。

---

## P0-1 同步引擎：禁止对不存在的 center 源建链 ✅ (2026-07-21)

**现象（实锤）**：改名使 `~/.skillmint/repo/<旧名>` 消失后，app 的自动同步在 23:15 按过期 `sync_targets` 重建了 18 条软链，名字是旧名、指向不存在的 `repo/<旧名>`——即 app 自己制造了悬空软链，并把它们计入同步健康。

**现状**：
- `skillmint/src-tauri/src/sync.rs:59` `apply_sync_target` **先无条件 `remove_path(agent_path)` 再 `create_symlink_or_copy(...)`**（fs.rs:10），创建前不校验 center 源是否存在；Unix 下 `symlink()` 对不存在的源也会成功，直接产出悬空链。
- `evaluate_sync_target`（sync.rs:15）能算出 center 缺失 = Broken，但 `sync_all`（sync.rs:139）对 Broken 仍走 apply。
- symlink 失败会**静默降级为整目录 copy**，用户无感知。

**方案**：
1. `apply_sync_target` 入口加 `center_path.exists()` 检查：源缺失时**不触碰 agent 侧**，只把状态置为 Broken 并计入 `SyncReport.broken`。
2. symlink 失败不再静默降级 copy；降级必须写进 `SyncReport` 并在 UI 可见（建议直接报错，copy 只在用户显式选择时发生）。
3. 补状态机文档与单测：`src/tests.rs` 已有 sync 测试基础，新增用例「center 目录被改名/删除后，sync_all 不得创建悬空链、agent 侧已有链接保持原样、状态=Broken」。

**验收**：重命名 center 内任一 skill 目录后跑 `sync_all_command`，agent 目录无新增悬空链；对应 target 状态为 Broken；`get_sync_targets` 与托盘同步健康数一致。

---

## P0-2 软链目录的 hash 语义：别把"已同步"误判成"内容冲突" ✅ (2026-07-21)

**现象**：改名完成后，`whyactions-*` 在各 agent 目录里已是指向 center 的软链，但「从 Agent 导入」对话框把它们全部标为「内容冲突」。根因：`compute_hash`（fs.rs:81）对 symlink 直接 hash 目标路径字符串，而 center 侧 hash 的是目录内容，两边恒不相等。

**方案**：
1. 扫描判定（`scan_agent_skills` commands.rs:557 / `scan_directory_skills_inner` :662）加一条捷径：agent 侧为 symlink 且解析目标等于 center 内同名目录 → 直接判 `content_match=1`，不再走内容 hash。
2. 前端 `ImportSkillModal.tsx` 对这类条目标注「已同步（软链）」而非「内容冲突」，且默认不勾选、不可选"保留本地"（防止软链被实体覆盖）。

**验收**：agent 目录里是合法指向 center 的软链时，导入对话框显示「与中心一致」；实体目录内容不同才显示「内容冲突」。

---

## P0-3 扫描/导入过滤：非 skill 目录不得出现 ✅ (2026-07-21)

**现象**：`~/.agents/skills` 下的 `cache`、`data`、`marketplaces`（分别是指向 `~/.skillmint/cache|data|marketplaces` 的软链，均非 skill）在导入列表里显示为「新 Skill」，用户勾上后 `marketplaces`（8.6MB、含内嵌 `.git` 的插件市场镜像）被当作 skill 拷进 center 并登记，事后只能手工回滚。

**现状**：`scan_agent_skills` / `scan_directory_skills_inner` 只要 `read_dir` 里 `is_dir()` 就算 skill——不校验 `SKILL.md` 是否存在、不排除软链目录；`import_all_agent_skills`（commands.rs:417）同样逻辑。

**方案**：
1. skill 判定改为：目录（或软链解析后的目标）**包含 `SKILL.md`**。
2. 内置排除名（`cache`、`data`、`marketplaces`、`node_modules`、`.git`、`.trash` 等），并在设置页可配置追加。
3. `ImportSkillModal.tsx` 增加「全选 / 仅选"新 Skill" / 清空」批量操作——本次用户手动勾了 25 个框。

**验收**：在 agent 目录放置无 `SKILL.md` 的目录与指向非 skill 目录的软链，扫描结果不包含它们；导入列表可一键选中全部「新 Skill」。

---

## P1-1 deep link 必须能唤起窗口 ✅ (2026-07-21)

**现象**：`skillmint://sync`、`skillmint://open-skill` 唤起窗口成功率约五成。窗口不可见时，deep link 到达后 app 只向 WebView emit 事件，窗口不 show；多次实测（含重启后）窗口保持隐藏，事件疑似被前端消费但用户不可见。

**现状**：`lib.rs:314-333` 收到 `deep-link://new-url` 后仅向前端 emit；`tauri.conf.json:23` `visible:false`，setup 末尾才 `show()`（lib.rs:378）。deep link 到达时若 WebView 尚未 ready，前端 `App.tsx:148-163` 的监听尚未注册，事件直接丢失。

**方案**：
1. deep-link 处理入口先 `window.show(); window.set_focus();`，再 emit 给前端。
2. 前端 ready 前到达的 deep-link 在 Rust 侧缓存，前端 `App.tsx` 挂载后主动拉取（或 Rust 侧延迟重发）。
3. 端到端用例写进手动验证清单：`kill` 后冷启动 + `open "skillmint://sync"`，窗口必须出现并触发同步。

**验收**：窗口关闭/未建/已隐藏三种状态下，`open "skillmint://sync"` 100% 唤起窗口并触发对应动作。

---

## P1-2 窗口生命周期与托盘入口 ✅ (2026-07-21)

**现象**：使用中窗口多次"消失"（AX 树无窗口，进程与 WebContent 均存活），只能靠托盘图标重开。代码里**没有**失焦隐藏逻辑（全仓库无 `hide()`/`Focused` 处理），关窗即销毁窗口但进程常驻——一旦窗口被关闭，只剩托盘一个入口，而托盘图标的 AXPress 在自动化下不稳定（这是 macOS 的已知限制，非 app bug，但 app 应提供冗余入口）。

**方案**：
1. 拦截关窗（`CloseRequested`）改为 `hide()` 而不是销毁：SPA 状态（如打开到一半的导入对话框）得以保留，托盘/deep-link 重开时现场不丢。
2. 托盘菜单在「打开 SkillMint / 退出」之外增加直达项：「Skill 库」「同步健康」，点击后 show+focus 并 emit 路由事件（复用 deep-link 通道）。
3. 托盘图标双态图 `tray-normal.png` / `tray-warning.png` 目前**未在 `tauri.conf.json` 的 bundle 里声明 resources**——分发后可能丢图，随打包治理一并修（见 P2-2）。

**验收**：关窗后再通过托盘/deep-link 打开，恢复关窗前页面与对话框状态；托盘菜单三项均可用。

---

## P1-3 导入对话框的 Agent 下拉无法被自动化操作 ✅ (2026-07-21)

**现象**：`ImportSkillModal` 的 Agent 选择器（AXPopUpButton）不响应任何合成输入：AXPress、坐标点击、`set_value`、键盘均无法展开。真人一下就能点开——这是可访问性/可测试性缺陷，也堵死了任何 UI 自动化与端到端测试。

**方案**：把该下拉从原生映射的 `<select>` 换成应用内自绘 combobox（按钮 + 内联列表，与项目其他按钮一样走普通 click 事件），或至少保证 `AXPress` 可展开。前端测试（vitest + Testing Library）补「打开对话框 → 选 agent → 勾选 → 导入」全链路。

**验收**：合成事件可完整走通导入流程；`ImportSkillModal.test.tsx` 覆盖该路径。

---

## P1-4 官方 `rename_skill` 命令（本 session 的手工修复已验证 recipe） ✅ (2026-07-21)

**现象**：用户在 app 外重命名 skill 目录是真实场景（本次改了 9 个）。app 没有改名能力，结果是：DB 6 行记录指向不存在的旧路径、8 个 agent 目录里 60+ 条悬空旧名链、`kg_skill_nodes`/`sync_targets` 靠 id 的引用倒是没断（这是我们手工修复能成功的关键）。

**已验证的修复 recipe（可直接作为实现 spec）**：
1. `UPDATE skills SET name=?, repo_path=? WHERE name=?`（**保 id**，`kg_skill_nodes`、`sync_targets` 的 FK 引用全保）；
2. 每个相关 farm 目录：删旧名软链，建 `whyactions-* → repo/whyactions-*`；
3. `sync_targets` 状态刷新为 synced（可由 P0-1 修好的 `evaluate_sync_target` 重算）；
4. `agent_directory_skills` 缓存不用手工管，下次扫描自愈。

**方案**：新增 `rename_skill(old_name, new_name)` Tauri 命令，按上述 4 步在一个事务 + 一组文件操作里完成，冲突检测（新名已存在、center 已有同名目录）前置报错。`sync_single_skill_command`（commands.rs:3447）提供了单 skill 同步的现成通道，可复用。

**验收**：对一个已同步到 3 个 agent 的 skill 执行 rename，DB 记录、各 farm 软链、同步状态全部指向新名，无悬空链；kg/usage 历史不断。

---

## P1-5 `repair_paths` / `check_repo_integrity` 扩展：自动发现并治愈"改名+孤儿"

**现象**：本次排障前跑了 `repair_paths`，migrated 0——它只 detect「`repo_path` 指向旧扁平布局 `~/.skillmint/<skill>`」的行（commands.rs:60 `migrate_skills_to_repo`），对"目录被改名"和"center 有目录未登记"两类最常见漂移无能为力。`check_repo_integrity`（commands.rs:203）也只查"DB 有但 center 缺"的单向问题。

**方案**（按优先级）：
1. **改名配对**：center repo 内存在未登记目录、且 DB 存在 `repo_path` 缺失的行时，用 `compute_hash`（目录等价判定已有 `dirs_equivalent` 可参考，commands.rs:151）或 `SKILL.md` 相似度做配对建议，默认 dry-run 输出报告，`--apply` 才执行 P1-4 的 rename 流程。
2. **孤儿发现**：center repo 里未登记目录 → 报告并引导走「从 Agent 导入」同款登记流程（insert skill + sync targets）。本次 22 个孤儿就是手工用 `comm` 比对发现的，应该内置。
3. **legacy 布局**：识别 `~/.skillsync` 等上一代残留（本次实清：61 条悬空链 + `~/.skills`/`.openclaw` 里 6 条指向它的断链），给出清理建议。
4. `repair_paths` 二进制的打包声明缺失，随 P2-2 一起修。

**验收**：制造一次改名 + 一个孤儿目录，`repair_paths --dry-run` 报告两项，`--apply` 后 DB 与磁盘收敛一致。

---

## P2-1 git 集成：导入后自动提交 + 内嵌仓防护

**现象**：app 有 `git_init_repo` / `git_commit`（commands.rs:5029/5048，底层 `run_git` 调系统 git），但导入 24 个目录后 center repo 留下一堆未跟踪文件，需手工提交；手工 `git add -A` 时 `archify`（内嵌 git 仓）会变成无效 gitlink，`marketplaces` 里也有内嵌 `.git`。

**方案**：
1. 设置项「导入/同步后自动 commit」（默认关），commit message 含导入清单。
2. `git add` 前扫描内嵌 `.git`：命中则自动加入 `.gitignore` 并提示，而不是形成 gitlink（本次是把 `archify/` 写进 `.gitignore` 解决的）。
3. 已在仓的 `.gitignore` 模板补齐 `node_modules/`、`__pycache__/`（`git_init_repo` 时生成）。

**验收**：开启 auto-commit 后导入 3 个 skill，repo 自动产生一条含清单的 commit；含内嵌 `.git` 的目录不会被 add。

---

## P2-2 打包与工程治理

1. **bundle 资源缺口**：`tauri.conf.json` 未声明 `tray-normal.png`/`tray-warning.png`（`update_tray_status` 从 exe `../Resources` 读图）与 `repair_paths` 二进制的 `externalBin`——分发后托盘双态与修复工具都会缺失。补 bundle 配置并在 `make build-install verify` 里加断言。
2. **god-module 拆分**：`commands.rs`（5313 行）、`db.rs`（约 5000 行）按 sync / import / scan / agents / git / trash 拆分子模块；纯搬运，不改行为。
3. **CI**：`.github/` 目前只有 ISSUE_TEMPLATE。加 workflow：`cargo test`（`src/tests.rs` 4000+ 行已有基础）、`npm run test:ci`、macOS 上 `make build-install verify`。
4. **文档治理**：代码注释大量引用 `PRD-xx`/`SPEC-Fx`，但这些规范不在仓库里。要么收进 `docs/specs/`，要么注释里去引用化。本文档放 `docs/` 即作为该目录的首个文件。

---

## 附 A：本 session 已验证的手工修复 runbook

> 以下步骤已在生产机完整执行并验证，可作为 P1-4/P1-5 实现时的操作语义参考（含边界情况）。

**改名（9 个 skill，跨 8 个 agent 目录）**：
1. 重命名 center 目录与每个 `SKILL.md` 的 front matter `name`（AGENTS.md 约定：目录名 == name）。
2. 更新全部交叉引用：`skill="..."` 调用、`~/.agents/skills/...` 绝对路径、`../` 相对链接、方法论文档互引；数据目录（如文章库 `公众号SEO/`）与上游包身份（package.json、README 安装说明）保持不动。
3. 各 farm 删旧软链、建新软链（kimi/claude/codex/cursor/generic/openclaw/zcode/skillmint 共 8 处）。
4. DB：`UPDATE skills SET name, repo_path`（保 id）；缺的技能 `INSERT` + 按需 `INSERT sync_targets`（mode=symlink, status=synced, agent_directory_id 按 claude/codex 两条有值、其余 NULL 的现存模式）。
5. 清理 app 按过期记录重建的悬空链（见 P0-1）；`agent_directory_skills` 缓存交给下次扫描自愈。

**孤儿收编（22+2 个）**：GUI「从 Agent 导入」完成；`marketplaces` 非 skill 误收 → 从 repo 删除、清 DB 行与 sync_targets、恢复 agent 目录原软链（app 删除 skill 的官方路径见 `remove_skill_impl` commands.rs:1142，回收站语义）。

## 附 B：验证清单（每修完一条跑一遍）

- [ ] `sqlite3 "$DB" "SELECT name, repo_path FROM skills"` 全部指向存在的 center 目录
- [ ] 8 个 farm 目录无悬空软链（`find <farm> -type l ! -exec test -e {} \; -print` 为空）
- [ ] 托盘同步健康 = sync_targets 总数中 synced 占比，与 DB 直查一致
- [ ] `cd ~/.skillmint/repo && git status` 干净（或 auto-commit 已生效）
- [ ] 冷启动 + `open "skillmint://sync"` 窗口必现
