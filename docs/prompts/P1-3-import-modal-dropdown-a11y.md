# P1-3：导入对话框 Agent 下拉的可访问性修复

```text
项目：/Users/jiangjianyong/projects/03-OPC/tools/skillmint-opensource
先读：docs/OPTIMIZATION-2026-07.md 的 P1-3 节、skillmint/src/components/ImportSkillModal.tsx（含现有 ImportSkillModal.test.tsx）。

背景（实测）：ImportSkillModal 的 Agent 选择器在 AX 树里是 AXPopUpButton（WebKit 把 <select> 映射为原生控件），不响应任何合成输入——AXPress、坐标点击、set_value、键盘导航全部无法展开。真人一点就开。这堵死了 UI 自动化与端到端测试，也是可访问性缺陷。

任务：
1. 把该下拉从原生 <select> 换成应用内自绘 combobox：按钮 + 内联列表（点击展开、Esc 关闭、上下键导航、Enter 选定），与项目现有 Tailwind 风格一致；列表项来自按 skill_directory 去重后的 agents（保持现有 :114 附近的数据逻辑）。
2. 保证 AXPress 可展开（即可访问性树下是普通按钮+列表，而非原生 popup）。
3. 测试（vitest + Testing Library）补全链路：打开对话框 → 选择 agent → 列表渲染扫描结果 → 勾选 → 导入按钮计数变化。

边界：
- 只改这个组件；不触碰后端 scan/import 命令。
- 不引入新的组件库依赖（项目用 Tailwind + 自绘即可）。

验收：npm run test:ci 全绿；合成事件（Testing Library 的 click/keyboard）可完整走通"选 agent→勾选→导入"流程；真机手动点一遍确认交互手感不回退。
完成后：commit（引用 P1-3）；在 docs/OPTIMIZATION-2026-07.md P1-3 标题后标 ✅ 与日期。
```
