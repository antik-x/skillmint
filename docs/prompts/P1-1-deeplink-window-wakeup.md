# P1-1：deep link 必须能唤起窗口

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P1-1 节、skillmint/src-tauri/src/lib.rs:314-333（deep-link 处理）、:378（setup 末尾 show）、tauri.conf.json:23（visible:false）、前端 src/App.tsx:148-163（deep-link 监听）。

背景（实测）：skillmint://sync、skillmint://open-skill 唤起窗口成功率约五成。窗口隐藏/未建时，app 只向 WebView emit 事件而不 show 窗口；WebView 未 ready 时事件直接丢失。自动化与双击协议链接的用户都依赖这个入口。

任务：
1. lib.rs 的 deep-link 处理入口：先 window.show() + window.set_focus()，再向前端 emit 路由事件。
2. 事件可靠性：WebView 未 ready 期间到达的 deep-link 在 Rust 侧缓存（如 Vec<String> + Mutex）；前端 App.tsx 挂载完成后 emit 一个"app-ready"事件，Rust 收到后按序重发缓存的 deep-link 事件。
3. 不要改 deep-link 的路由语义（skillmint://sync → deep-link-sync、skillmint://open/skill/<name> → deep-link-open-skill 保持不变）。

边界：
- 仅窗口唤起与事件投递，不动托盘菜单（P1-2 处理）。
- 不引入新依赖。

验收：三种状态下 open "skillmint://sync" 均 100% 唤起窗口并触发动作——a) 窗口已隐藏/关闭；b) 窗口从未创建（冷启动后立即发）；c) 窗口可见。cargo test + npm run test:ci 全绿；手动验证记录进 commit message 或 PR 描述。
完成后：commit（引用 P1-1）；在 docs/OPTIMIZATION-2026-07.md P1-1 标题后标 ✅ 与日期。
```
