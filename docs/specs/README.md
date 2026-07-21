# Spec 引用索引（PRD-xx / SPEC-Fx）

代码注释中大量引用 `PRD-xx` / `SPEC-Fx` 形式的设计文档编号。这些文档产生于
SkillMint 开源发布之前的私有规划阶段，全文不在本仓库中。本索引按代码中的实际
实现整理每个编号的范围，使仓库内的引用不再悬空。

> 治理决策（P2-2-d）：在「收进规范全文」与「注释去引用化」之间选择**就地建立
> 本索引**，理由：① 规范原文不可得，无法收录，更不应凭空杜撰；② 全仓约 400
> 处引用，逐条改写注释 churn 大、易出错；③ 这些编号是代码评审与排障时的定位
> 词汇（如「SPEC-F3 的启动一致性检查」），保留编号 + 索引比抹掉编号更有价值。
> 新写的注释请优先就地描述行为，不再新增编号引用。

## 索引

| 编号 | 范围（按代码实现归纳） |
|---|---|
| PRD-0 | 应用基座：中心仓、设置、自动同步调度（§4.7）、快照备份与恢复（§4.8） |
| PRD-01 | 项目级 Skill 管理：项目绑定、多版本布局（latest/v<N>）、差异处理（FR-C/FR-F，§4.5c 留底） |
| PRD-02 | 使用数据采集：各 Agent 会话数据的只读采集（含硬约束）、collector 体系 |
| PRD-03 | 知识图谱：SKILL.md 分析、节点/边、关系发现（FR-2.1 frontmatter related_skills） |
| PRD-05 | Agent 采集源扩展：ZCode（CLI/IDE）、Cursor（仅代码贡献，无 token） |
| PRD-06 | Agent 实体模型：Agent（名+source）与目录 1:N（§3.3）、发现与归一化 key、§5.2 子目录记录 |
| PRD-07 | 远程 Skill 源：GitHub tarball 拉取、安装前安全扫描（§3.2 高风险指令）、remote_enabled 总开关（local-first） |
| PRD-08 | AI 分析：LLM/ACP 配置与 Keychain 凭据（§3.6）、prompt 语义分类、计费口径、用量时间线 |
| PRD-09 | 编辑器与版本：内置 SKILL.md 编辑器（改名联动）、快照 + Git 版本控制（§4） |
| PRD-10 | 计划任务：定时任务引擎、运行记录、暂停/恢复 |
| PRD-11 | AI 每日摘要价值模块：每日总结生成与展示 |
| PRD-12 | 发现收编的置信度红线（A-R1 等）：低置信静默入箱、失败回传 |
| SPEC-C1 | 发现收编的门槛与台账：低置信带、gate-rejection ledger（T2/T4） |
| SPEC-C2 | 部分同步恢复：单 Skill 重试通道（partial_synced 恢复路径） |
| SPEC-C3 | 回收站：删除即快照、恢复/清理、过期清扫（T2） |
| SPEC-F2 | 编辑器可靠性：统一 invoke 错误包装（T2）、外部改动检测（T10）、终端打开（T11） |
| SPEC-F3 | 启动与唤起：启动一致性检查、skillmint:// deep link、文件锁重试 |
| SPEC-F4 | 旧扁平布局迁移：~/.skillmint/<skill> → repo/（T12） |
| SPEC-F5 | 实机验证：repair_paths 验证 harness（T5） |
| SPEC-F6 | 导入与引导 UX：导入弹窗目录去重（T1）、成功 toast 直达动作（T4）、HelpTip 接线 |
| SPEC-I1 | 原子性/健壮性测试基线（F2/F4/F5/F6 各面） |
| SPEC-I2 | 发现收件箱 + 周报聚合 |
| SPEC-I4 | 信息架构：顶层导航与 tab 体系 |

编号后的 `Tn`（如 T5、T12）是各规范内的任务/验收条目号。

## 相关文档

- [`../OPTIMIZATION-2026-07.md`](../OPTIMIZATION-2026-07.md) — 2026-07 同步正确性/导入过滤/窗口可靠性优化方案（含逐项验收）。
- [`../prompts/`](../prompts/) — 上述方案的执行 prompts 与顺序依赖表。
