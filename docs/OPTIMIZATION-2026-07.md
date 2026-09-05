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

## P1-5 `repair_paths` / `check_repo_integrity` 扩展：自动发现并治愈"改名+孤儿" ✅ (2026-07-21)

**现象**：本次排障前跑了 `repair_paths`，migrated 0——它只 detect「`repo_path` 指向旧扁平布局 `~/.skillmint/<skill>`」的行（commands.rs:60 `migrate_skills_to_repo`），对"目录被改名"和"center 有目录未登记"两类最常见漂移无能为力。`check_repo_integrity`（commands.rs:203）也只查"DB 有但 center 缺"的单向问题。

**方案**（按优先级）：
1. **改名配对**：center repo 内存在未登记目录、且 DB 存在 `repo_path` 缺失的行时，用 `compute_hash`（目录等价判定已有 `dirs_equivalent` 可参考，commands.rs:151）或 `SKILL.md` 相似度做配对建议，默认 dry-run 输出报告，`--apply` 才执行 P1-4 的 rename 流程。
2. **孤儿发现**：center repo 里未登记目录 → 报告并引导走「从 Agent 导入」同款登记流程（insert skill + sync targets）。本次 22 个孤儿就是手工用 `comm` 比对发现的，应该内置。
3. **legacy 布局**：识别 `~/.skillsync` 等上一代残留（本次实清：61 条悬空链 + `~/.skills`/`.openclaw` 里 6 条指向它的断链），给出清理建议。
4. `repair_paths` 二进制的打包声明缺失，随 P2-2 一起修。

**验收**：制造一次改名 + 一个孤儿目录，`repair_paths --dry-run` 报告两项，`--apply` 后 DB 与磁盘收敛一致。

---

## P2-1 git 集成：导入后自动提交 + 内嵌仓防护 ✅ (2026-07-21)

**现象**：app 有 `git_init_repo` / `git_commit`（commands.rs:5029/5048，底层 `run_git` 调系统 git），但导入 24 个目录后 center repo 留下一堆未跟踪文件，需手工提交；手工 `git add -A` 时 `archify`（内嵌 git 仓）会变成无效 gitlink，`marketplaces` 里也有内嵌 `.git`。

**方案**：
1. 设置项「导入/同步后自动 commit」（默认关），commit message 含导入清单。
2. `git add` 前扫描内嵌 `.git`：命中则自动加入 `.gitignore` 并提示，而不是形成 gitlink（本次是把 `archify/` 写进 `.gitignore` 解决的）。
3. 已在仓的 `.gitignore` 模板补齐 `node_modules/`、`__pycache__/`（`git_init_repo` 时生成）。

**验收**：开启 auto-commit 后导入 3 个 skill，repo 自动产生一条含清单的 commit；含内嵌 `.git` 的目录不会被 add。

---

## P2-2 打包与工程治理 ✅ (2026-07-21)

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

---

## P3 npx skills 集成：app 全面转为 `skills` CLI 的上层 GUI ✅ (2026-09-04)

**决策背景**：SkillMint 自研的 center-repo symlink 同步引擎与 `npx skills` 生态（vercel-labs/skills，canonical 目录 `.agents/skills` + 两份 lock 文件 + 77-agent 矩阵）在同一个 agent 目录空间里各自为政，存在互相打架的结构性风险。定案：**不另起炉灶**——安装/更新/卸载/搜索全部走真实 CLI，app 是终端命令的上层 GUI（Mole 范式：磁盘状态唯一事实源，app 无平行状态）。实施于分支 `feat/npx-skills-integration`。

### P3-1 npx 运行时 ✅
- `src-tauri/src/npx.rs`：node/npx 探测链（设置覆盖 → PATH → nvm → Homebrew），`detect_node_env` 报告 Node ≥ 22.20 检查结果并给出安装引导文案；
- 显式参数构造器（`add/remove/update/ls`，一律 `-s/-a/-y`，GUI 安装禁止交互）；env 注入：`SKILLS_API_URL`（搜索镜像）、`HTTPS_PROXY/HTTP_PROXY/ALL_PROXY`（代理，覆盖 CLI 的 git clone 步骤）、`DISABLE_TELEMETRY=1`（默认，可关）；
- 流式执行（超时保护 + `npx-output` 事件）与 `skills ls --json` 解析。

### P3-2 agent 全表 ✅
- `src-tauri/src/agents_table.rs`：静态镜像 vercel-labs/skills `src/agents.ts` 的 77-agent 矩阵（key/显示名/项目目录/全局目录），单测锚定 claude-code/zcode/codex/cursor 等关键映射；
- `scan.rs` preset 由全表派生；`~/.agents/skills` 归属合成 Agent "Universal Agents"（npx 全局 canonical 语义）；`~/.cursor/rules`、`~/.claude/commands`、`~/.codex/instructions`、`~/.zcode/cli/plugins` 等资源目录保留扫描。

### P3-3 可重建索引 ✅
- `skills_lock.rs`：解析项目 `skills-lock.json`（schema v1，`computedHash`）与全局 `~/.agents/.skill-lock.json`（schema v3，`skillFolderHash`）；
- `index.rs`：统一索引 = 两份 lock + canonical 目录 + 全部 agent 目录（含 unmanaged/断链识别）+ 两个私有 hub；`db/index_store.rs`：`skill_index` 表整体替换式写入，内容 hash 与上次扫描比对产生「已被本地修改」状态；
- 刷新策略：窗口聚焦 + 安装动作后 + 定时（复用 `auto_sync_interval_minutes`）的 mtime 指纹探测，变化才重建；启动时后台重建一次；`skillmint://sync` 语义改为「重建索引」。

### P3-4 安装/卸载/更新 UI ✅
- `pages/InstalledSkills.tsx` 取代 center-repo 库页成为「技能」子页：来源/管辖/状态徽章、agent 芯片、项目上下文选择器、更新/卸载/收集到 Hub 动作；
- 安装面板：scope/agent 多选/`--copy`，操作前展示**等价 CLI 命令**（复制 + 在终端打开），运行时流式回显；安装完成后自动重建索引 + 对落盘 SKILL.md 跑 `scan_safety` + 记 `install_audit`；
- Discover：新增 skills.sh 注册表搜索区（`/api/search`，走镜像设置），一键 npx 安装；原两个安装对话框从 `install_remote_skill` 切到 `npx_install`（安装逻辑单轨化，安装前安全扫描保留为提示、安装后扫描成为兜底）。

### P3-5 私有 hub ✅
- `src-tauri/src/hub.rs`：全局 `~/.skillmint/hub`（独立 git，可配远端、手动推送）与项目 `<project>/.skillmint/hub`（随项目仓库提交，不嵌套 .git）；
- 新建 skill（模板 + git 自动提交）、收集到 hub（**只复制**，不动源目录/npx lock/agent 链接，跳过 `.git/node_modules/…`）、远端配置与推送状态（无 upstream 时 ahead=待推提交数）；
- 卸载前快照到 `~/.skillmint/trash/` 并登记 SPEC-C3 trash 表（30 天过期，启动清理沿用）。

### P3-6 退役清理 ✅（UI/入口层）
- 设置删减：`center_repo` / `default_sync_mode` / `skill_scope_mode` / `project_skill_dir_name` 从 Settings 暴露面（后端 AppSettings 模型 + 前端类型/store/偏好面板）移除；
- 入口全断：App.tsx 自动 tick 与 deep-link 改为索引刷新、center repo 恢复横幅移除；lib.rs 不再引导 center repo、启动一致性检查替换为指纹化索引重建；Skills.tsx 库页与相关测试删除；Onboarding 不再询问仓库路径。
### P3-6b 机械删除 ✅
- 删除孤儿命令（无前端调用者，25 个）：`add_skill`、`remove_skill`、`create_skill`（命令面；采纳管线改用内部 `create_skill_internal`）、`update_skill_status`、`rename_skill`、`create_project_skill`、`promote_to_global`、`install_skill_to_project`、`resolve_skill_link_command`、`resolve_skill_diff_command`、`list_skill_versions_command`、`pin/delete/set/get_version_note`、`save_version`、`rollback_to_version`、`add/remove/get_skill_bindings`、`install_remote_skill`、`import_skill`、`read_skill_content`、`check_skill_external_change`、`check_repo_integrity`；
- 删除孤儿组件（前端）：SkillEditor / VersionPanel / ImportSkillModal；Projects 页重写为纯使用洞察页（安装/版本/一致性 UI 移除）；`backup_center_repo` 从任务表单选项移除（后端任务类型保留，DB 已有种子行）；
- 删除死代码：`resolve_diff_core` 相关版本 core、`move_into_center`、`run_startup_consistency_check`、`SkillVersion`/`RollbackResult`/`SkillContent`/`RepoIntegrity`（后者保留为 repair 测试 oracle，标 allow）、`InstallRemoteResult` 模型；`init_app` 的 center repo 引导与迁移剥离；
- trash 恢复/清除机制保留（TrashPanel 仍在用），其测试改用本地夹具播种回收站；
### P3-9 真机启动修复 ✅ (2026-09-05)

用户真机报告：启动卡在「正在初始化...」且界面不美观。三层根因，全部修复并真机（`open -a SkillMint` + 截图）验证通过：
1. **启动挂死**：appStore.loadData 仍调用已在 P3-8 删除的 `get_sync_targets` 命令 → init 链 reject → 永远停在启动页。调用链已删，且 init 改为 fail-open（任何初始化失败只 toast，必定进入主界面）。
2. **落到 Onboarding + 默认设置**（第二层，靠界面内嵌错误横幅取证）：`init_app` 在真实库上报 `FOREIGN KEY constraint failed`——`insert_agent` 用 `INSERT OR REPLACE`（SQLite 语义 = 先 DELETE 父行），`agent_instances`/`sync_targets` 子行存在时每次启动必炸。改为 `ON CONFLICT DO UPDATE` 真 upsert；`delete_agent` 先清子表（sync_targets/agent_directory_skills/agent_directories/agent_instances，bindings 置 NULL）；`scan_and_persist_agents`/`link_sessions_to_projects` 降级为非致命步骤。
3. **启动页重设计**：logo 徽标 + 不定进度条取代空白骨架块；init 失败信息以红色横幅持久显示在启动页/引导页（不再静默吞掉）。

教训：此前只跑了 cargo/vitest + bundle 断言，没有真机启动过——「构建安装成功」≠「能进主界面」。此后每次 build-install 都应 `open` 一次并确认越过启动页。

### P3-10 智能体目录：实时计数 / 分组 / 关联项目清洗 ✅ (2026-09-05)

用户真机报告智能体目录页四个问题，grilling 访谈定案后全修：
1. **「N 个 Skill」恒为 0**：列表计数走 `agent_directory_skills` 缓存表，而缓存只在点开详情页时填充——没点开过的 agent 全部假 0。`get_agent_skill_counts` 改为**实时扫目录**（`scan::count_skills_in_dir`：含 SKILL.md 的子目录、跟随符号链接、排除 cache/data/marketplaces/node_modules/.git/.trash），删除孤儿缓存计数方法。缓存仅剩详情页技能列表使用。
2. **分组**：列表拆成两个固定区块——「智能体（Harness）」（有独立 source key 的真实工具）在上、「共享 / 占位目录」（source 为空或 `universal`：canonical `~/.agents/skills`、兜底 `~/.skills`、SkillMint 旧版）在下；组内按技能数降序、平局按名称。旧版遗留行 `Agent → ~/.skillmint/skills` 显示名固定为「SkillMint 旧版目录」（数据保留，仅改名）。
3. **关联项目脏数据**：projects 表 471 行中 256 行是相对路径碎片（`claude-chrome` 等）、3 行是工具数据目录（`~/.codex/sessions/...`）、大量同路径异拼重复行——Claude Code 显示 177 个项目实际约 102 个。新增规则 `is_linkable_project_path`（须绝对路径且不在 $HOME 隐藏顶级目录下）拦截增量；一次性 migration `project_link_hygiene_done`（带预迁移备份 + 事务 + 哨兵）清洗存量：删幻影项目、按 canonical 路径合并重复、`agent_instances` 与 `project_count`/`last_used_at` 全量重建；`link_sessions_to_projects` 的重算 UPDATE 去掉 `WHERE EXISTS` 守卫（否则失去全部实例的 agent 永远挂着脏计数）。
4. **source 错配修正（只动了真错配）**：Kimi 采集 tag `kimi-code` 与矩阵 key `kimi-code-cli` 同产品不同拼写 → 常量对齐 + 存量六表改名。**Antigravity 故意不动**：`SOURCE_ANTIGRAVITY="antigravity"` 采集的是 Antigravity **IDE**（`~/.gemini/antigravity/brain`），与矩阵里的 `antigravity-cli`（CLI）是两个产品——IDE 会话不归属到 CLI agent 是语义正确，`trae-solo` 同理（独立产品，无对应 agent）。
5. **覆盖面结论（不改代码）**：扫描 = 77 agent 静态矩阵 global 目录 + 资源目录 + `~/.skills` 兜底，逐目录存在性判定、按 `(name, source)` 合并 "one Agent per tool"；真机核查所有实际存在的 skills 全局目录均已入表，无遗漏，使用中但无目录的来源不造占位行。

**验收**：cargo test 245✓（+4：计数口径/路径规则/关联跳过/迁移清洗）、vitest 163✓（+8：分组判据/旧版改名/区块排序）、tsc ✓；真机 `make build-install` + 启动截图验证分组、计数、关联项目。

### P3-8 多角色审查修复 ✅ (2026-09-05)

虚拟五类用户（新装机/CLI 老手/团队协作/国内网络/老版本升级）走查后修复：
- **B1** 技能库 hub 行重复（索引已含 hub 行，删除二次拼接）；
- **B2** 项目级安装未选项目时 CLI 会以 $HOME 为 cwd 污染家目录——安装按钮改为禁用 + 提示，Hub「安装到项目」同样前置校验；
- **B3** 回收站「恢复」原样写回已退役 center repo（恢复后不可见）——语义改为**内容找回**：快照恢复进全局 hub 并 git 提交，重名走覆盖/重命名（覆盖的旧目录先快照进回收站，可逆）；回到 agent 目录用 `npx skills add` 显式重装；
- **B5** `skills_search` 的 HTTP 请求注入 `proxy_env` 代理（此前只有 CLI 的 git 步骤走代理）；
- **Q4** 安装面板打开即检测 Node 运行时，不可用直接禁用安装并内联给引导文案；
- **Q5** `skillmint://open/skill/<name>` 改为预填技能库搜索过滤；
- **Q3a 采纳管线迁移**：Inbox 采纳 / Usage 的 prompt→skill 创建改写全局 hub（git 自动提交、kebab-case 强制），不再自动同步（`sync_summary` 恒为 None）；删除 Inbox 部分同步横幅与重试流；Dashboard 页移除；Today 的「同步健康」改为「技能索引」健康（total/modified/broken + 一键刷新）；`sync_all_command`/`sync_single_skill_command`/`get_sync_targets`/`save_skill_content`/`get_conflict_contents`/`resolve_conflict` 命令面删除；
- **P3-6c 余量收窄**：采纳管线已迁 hub；sync 引擎（sync.rs）仅剩库函数形态服务 repair/定时任务 legacy 类型与 bundles 流，无任何 UI 入口。

**验收**：`cargo test --lib` 247 通过；`npm run test:ci`（Node 22）166 通过；真实机器上 `npx skills add vercel-labs/agent-skills -s pdf …` 后 app 聚焦即显示该 skill（lock 驱动），卸载走确认 + trash 快照。

**加速配置结论**（回答「npx skills 是否需要加速配置」）：需要、且已内置最小集——skills.sh 搜索在国内可能不可达（`SKILLS_API_URL` 可指向镜像），CLI 内部 fetch 不读 `HTTP(S)_PROXY`，但安装下载走 git（受 `HTTPS_PROXY`/git config 影响），api.github.com 失败会自动回退 git clone，因此「搜索镜像 + git 代理」两板斧即可覆盖安装主链路；`GITHUB_TOKEN` 只解决限流不解决可达性。
