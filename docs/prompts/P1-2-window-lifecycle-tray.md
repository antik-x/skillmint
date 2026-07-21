# P1-2：窗口生命周期与托盘入口

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P1-2 节、skillmint/src-tauri/src/lib.rs:226-285（托盘 setup 与菜单）、P1-1 项完成后的 deep-link 唤起代码、前端 src/App.tsx。

背景：使用中窗口多次"消失"（AX 树无窗口、进程与 WebContent 存活），只剩托盘一个入口；而托盘图标的 AXPress 在自动化下不稳定（macOS 已知限制）。代码里没有失焦隐藏逻辑，关窗即销毁窗口、SPA 状态（如打开到一半的导入对话框）全丢。

前置：P1-1 已完成（deep-link 先 show+focus）。

任务：
1. 拦截关窗（CloseRequested）→ window.hide()，不销毁窗口：SPA 状态保留，托盘/deep-link 重开时恢复现场。注意评估"退出"语义——托盘菜单的「退出」必须仍能真正退出进程。
2. 托盘菜单从「打开 SkillMint / 退出」扩为：「打开 SkillMint」「Skill 库」「同步健康」「退出」。新增项行为：show+focus 并 emit 路由事件（复用 P1-1 的 deep-link 事件通道或新增专用事件），前端 App.tsx 监听后跳转对应页面。
3. 前端为「Skill 库」「同步健康」两个事件补路由与测试。

边界：
- 托盘双态图标（tray-normal/warning.png）的打包声明问题归 P2-2，本项不修。
- 不改动 update_tray_status 的冲突数切换逻辑。

验收：关窗→托盘「打开 SkillMint」→恢复关窗前页面与对话框状态；托盘四个菜单项全部可用；cargo test + npm run test:ci 全绿；手动验证三种托盘入口。
完成后：commit（引用 P1-2）；在 docs/OPTIMIZATION-2026-07.md P1-2 标题后标 ✅ 与日期。
```
