# P0-2：软链目录的 hash 语义——别把"已同步"误判成"内容冲突"

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P0-2 节、skillmint/src-tauri/src/fs.rs:81（compute_hash）、commands.rs:557（scan_agent_skills）与 :662（scan_directory_skills_inner）的比对逻辑、前端 skillmint/src/components/ImportSkillModal.tsx 的状态标注（:176 附近）。

背景：9 个 skill 改名后，各 agent 目录里的 whyactions-* 已是指向 center 的软链，但「从 Agent 导入」对话框把它们全标为「内容冲突」。根因：compute_hash 对 symlink 直接 hash 目标路径字符串，center 侧 hash 的是目录内容，两边恒不相等 → content_match 恒 false。

任务：
1. 在扫描判定层加短路逻辑：agent 侧为 symlink 且 canonicalize 解析后等于 center repo 内同名目录 → 直接置 exists_in_center=1、content_match=1，不再走内容 hash。说明你为什么选择在扫描层短路而不是改 compute_hash（改 hash 语义会影响 sync 状态机，需论证）。
2. ImportSkillModal.tsx：这类条目显示「与中心一致」（或新标签「已同步（软链）」），默认不勾选，且不可选"保留本地"（防止软链被实体目录覆盖）。
3. 测试：Rust 侧为软链判定补用例；前端为该条目的展示与不可选状态补组件测试。

边界：
- 只改"已指向 center 的合法软链"这一场景的判定；实体目录的内容比对逻辑不动。
- 与 P0-3（导入过滤）串行，不要把两项揉进一个 commit。

验收：cargo test + npm run test:ci 全绿；手动——agent 目录里放一条指向 center 的合法软链，扫描结果显示「与中心一致」而非「内容冲突」。
完成后：commit（引用 P0-2）；在 docs/OPTIMIZATION-2026-07.md P0-2 标题后标 ✅ 与日期。
```
