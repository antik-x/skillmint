# OpenViking 融入 SkillMint：设计思考

> 状态：设计提案，不包含实现。
> 基线：SkillMint `docs/product-epics.md` 定稿；OpenViking 本机实例 v0.4.10。

## 1. 结论

OpenViking 应定位为 **SkillMint 个人成长飞轮的外置上下文引擎**。
它把 SkillMint 已拥有、已采集的本地资产投影成可语义检索、可关联、可回忆的上下文层，
为 E1 的技能查找、E2 的工作记忆、E3 的关系发现和后续推荐提供可选增强。

“外置”同时表达三个事实：

- OpenViking 是独立运行在 `http://localhost:1933` 的进程，不嵌入 SkillMint 数据库；
- 即使 HTTP 目标是 localhost，OpenViking 的 embedding/VLM 仍可能经火山引擎 Ark 出网；
- SkillMint 的主链路不依赖它，关闭或失效后必须完整退回现有规则版。

OpenViking **不是**：

- SkillMint 本地 SQLite 的替代品；
- 本地 Agent 原始会话的真相源；
- npx locks、canonical store 或私有 Hub 的替代品；
- 技能安装、更新、卸载的执行器；
- 默认开启的 AI 后端；
- E1、E2、E3 任一 epic 的硬依赖。

四条不可破坏的所有权规则：

1. 技能安装生命周期仍由 `npx skills` 持有，并且只能通过 `npx.rs` 执行。
2. 技能真相源仍是 npx locks、canonical store 与全局/项目私有 Hub。
3. 会话真相源仍是本地 Agent 目录；SkillMint SQLite 与 OpenViking 都是派生层。
4. `remote_enabled=false` 时不得向 OpenViking 发出任何请求，已有规则功能不得减少。

本文只定义方向、数据所有权、调用契约与阶段 gate；不新增 Tauri command，
不改设置 schema，不改变 Docker 部署，也不作成本或性能基准承诺。

本文采用的已知运行基线是：OpenViking 由 Docker 在本机运行，版本 v0.4.10，
地址为 `http://localhost:1933`，使用 `Authorization: Bearer <api_key>`；
vectordb `context` 与 agfs 均为 local，embedding 为 `doubao-embedding-vision`（1024 维），
VLM 为 `ark-code-latest`。后两者经火山引擎 Ark，是隐私门控中必须显式披露的出网路径。

## 2. Epic 映射

| Epic | OpenViking 能力与真实 endpoint | 复用的现有 Rust 模块 | 定性与规则版基线 |
|---|---|---|---|
| E1 管好技能资产 | 技能副本、校验与语义查找：`/api/v1/skills`、`/api/v1/skills/find`、`/api/v1/skills/validate`、`/api/v1/skills/{skill_name}` | `scan.rs`、`hub.rs`、`npx.rs` | **增强，不替代。** 规则版仍由本地扫描、npx locks/canonical store、Hub 和 `npx skills` 完成浏览与安装生命周期；OV 只保存可重建副本并改善查找/校验。 |
| E2 看清投入产出 | 记忆摄取与召回：`POST /api/v1/sessions`、`POST /api/v1/sessions/{id}/messages`、`POST /api/v1/sessions/{id}/messages/batch`、`POST /api/v1/sessions/{id}/commit`、`POST /api/v1/sessions/{id}/extract`、`GET /api/v1/sessions/{id}/context`、`GET /api/v1/sessions/{id}/used`，以及 `POST /api/v1/search/find`、`POST /api/v1/search/search`、`POST /api/v1/search/recall`、`POST /api/v1/search/grep`、`POST /api/v1/search/glob` | `collector.rs`、`collectors_ext.rs`、`analyzer.rs`、`classifier.rs`、`metrics.rs`、`pricing.rs`、`scheduler/` | **增强，不替代。** 规则版仍由只读采集、SQLite 聚合、本地分类、指标、成本与工作记忆报告产出；OV 只补充上下文召回与记忆证据。 |
| E3 越用越富 | 语义关系与候选证据：`GET /api/v1/relations`、`POST /api/v1/relations/build_graph`、`POST /api/v1/relations/link`、`DELETE /api/v1/relations/link`、`/api/v1/skills/find`，以及 `POST /api/v1/search/find`、`POST /api/v1/search/search`、`POST /api/v1/search/recall`、`POST /api/v1/search/grep`、`POST /api/v1/search/glob` | `kg.rs`、`db/kg.rs`、`discovery/capability_gap.rs`、`discovery/high_value_prompt.rs`、`discovery/keywords.rs`、`discovery/llm_enhance.rs`、`hub.rs` | **增强，不替代。** 规则版仍由本地图谱、重复模式/能力缺口/高价值 Prompt 检测和 Hub 草稿流程承担；OV 只增加语义边、解释证据和 later 推荐候选。 |

横切的集成可观测性可读取：

- `GET /api/v1/console/dashboard/summary`
- `GET /api/v1/console/tokens`
- `GET /api/v1/console/context-commits`
- `GET /api/v1/console/audit`
- `GET /api/v1/stats/memories`

这些数据只回答“OpenViking 集成是否健康、处理了多少上下文”，
不得替代 `metrics.rs` 与 `pricing.rs` 所定义的 SkillMint 投入产出口径。
后者始终是 E2 的规则版基线。

## 3. 集成架构

### 3.1 数据流

```text
本地 Agent 会话目录（真相源，只读）
  -> collector.rs / collectors_ext.rs / scan.rs
  -> 本地 SQLite（可重建读模型）
  -> analyzer.rs / classifier.rs / metrics.rs / pricing.rs（规则版，始终可用）
  -> [remote_enabled AND OpenViking 专属 opt-in]
  -> OpenViking adapter（认证、超时、能力探测、幂等、重试、脱敏、审计）
  -> http://localhost:1933
  -> sessions / search / memory（派生增强，可删除、可重建）

npx locks + canonical store / 私有 Hub（技能真相源）
  -> scan.rs + npx.rs + hub.rs
  -> SkillMint 本地技能索引（可重建）
  -> kg.rs + db/kg.rs（规则图谱，始终可用）
  -> [remote_enabled AND OpenViking 专属 opt-in]
  -> OpenViking skills 副本
  -> relations/build_graph
  -> OV 增强边（独立来源，可删除、可重建）
  -> 本地图谱展示读模型
```

只允许 SkillMint Rust 后端调用 OpenViking；React 前端不直接访问服务，也不持有 Bearer key。
OpenViking 不得回写 Agent 目录、Hub、npx locks 或 canonical store，
也不得直接触发 `npx.rs` 的安装、更新或卸载。

在实现阶段，应在外部 HTTP 依赖处建立窄 seam：
业务模块只提交“同步技能”“摄取会话”“获取增强关系”等意图并接收带来源的结果，
生产使用 HTTP adapter，测试使用内存 adapter。
认证、版本探测、超时、有限重试、熔断、幂等和错误归类都收拢在 adapter 内，
避免 endpoint 细节散落到 `kg.rs`、`discovery/` 或采集模块。

该 seam 不取代 SkillMint 现有可选 AI 层：`llm.rs`、`acp.rs` 仍负责既有模型能力，
OpenViking adapter 只负责上下文数据库交互，不扩张成通用模型网关。

OV 返回的关系、记忆和检索结果必须标注 `provider=openviking`、来源 URI、
对应 skill/session 标识与降级状态；不得冒充本地规则结果。

### 3.2 数据所有权与重建

| 数据 | 真相源 | OV/SkillMint 中的角色 | 删除与重建 |
|---|---|---|---|
| 原始 Agent 会话 | 本地 Agent 数据目录 | 只读输入；OV 不直接扫描 | 由 Agent 自身管理；SkillMint 不回写 |
| 归一化会话、指标、报告 | 原始 Agent 会话 + 本地规则 | SQLite 可重建读模型；OV 只收获授权投影 | 清缓存后从 Agent 目录重采集 |
| 技能安装状态 | npx locks + canonical store | SkillMint 为 GUI；OV 不拥有安装状态 | 重新扫描 locks 与目录；变更只走 `npx.rs` |
| 自制技能内容 | 全局/项目私有 Hub | 创作真相源；OV 为检索副本 | 删除 OV 副本后由 `hub.rs` 对应内容重投影 |
| OV 技能副本 | npx/Hub 技能真相源 | 可重建派生数据 | 按当前 device/account 所有权清理后重建 |
| OV sessions/memories | 本地已授权会话投影 | 可重建增强数据，不反向覆盖本地 | 清理后按本地游标与授权范围重摄取 |
| OV relations | 本地技能投影 + OV 推导 | 增强边；本地规则边和用户裁决优先 | 清理后重新 `build_graph`；不得覆盖拒绝记录 |
| OV pack/snapshot 文件 | 用户选定的 OV 派生数据快照 | 搬迁/恢复介质，不升级为真相源 | 用户显式管理；恢复不改 Agent、Hub、npx 状态 |

### 3.3 配置与凭据

- base URL、启用状态和非敏感能力状态可沿用 `settings.rs` 的设置实践；本文不定义字段名。
- Bearer API key 必须存入 macOS Keychain，服务名沿用 `com.skillmint`，使用 OV 独立 account 名。
- API key 不得写入 `settings.json`、SQLite、任务记录、错误消息或前端状态持久化。
- `Authorization: Bearer <api_key>` 只由 Rust 侧 adapter 注入。
- 日志只记录 endpoint、状态码、耗时、对象类别/数量和脱敏标识，不记录正文或认证头。
- `device_id` 可先作为本地映射键；是否映射 `/api/v1/admin/accounts...` 留待契约核实。

## 4. HTTP 调用契约

本文只使用已核实路径，不假定 brief 未提供的 JSON 字段、分页或错误结构。
实现前应从 v0.4.10 的 OpenAPI 固化最小 schema fixture 与契约测试。

### 4.1 P0 只读探针

- 不虚构或依赖 `/health`。
- 用 `GET /api/v1/console/dashboard/summary` 检查可达性与认证。
- 再用 `GET /api/v1/skills`、`GET /api/v1/relations` 探测所需只读能力。
- 明确区分：连接失败、超时、认证失败、endpoint 不兼容、服务内部错误。
- 探针失败只更新集成状态，不改变任何业务结果。

### 4.2 技能投影

- 从 npx locks/canonical store 或 Hub 读取稳定名称、内容 hash、scope 与 device 元数据。
- 先调用 `/api/v1/skills/validate`，再通过 `POST/GET /api/v1/skills` 及
  `GET/PUT/DELETE /api/v1/skills/{skill_name}` 维护受控镜像。
- `/api/v1/skills/find` 只用于语义查找或推荐证据，不能判定安装状态。
- 删除操作只针对能够证明由当前 SkillMint device/account 创建的副本。
- **规则版打底：** `scan.rs` 继续发现资产，`hub.rs` 继续创作，`npx.rs` 独占安装生命周期。

### 4.3 图谱增强

- 技能投影成功后调用 `POST /api/v1/relations/build_graph`。
- 通过 `GET /api/v1/relations` 拉取增强关系，并以独立 provider 合并到展示读模型。
- `POST /api/v1/relations/link` 与 `DELETE /api/v1/relations/link` 可承载未来人工关系，
  但 P1 不双写人工确认/拒绝，直至反馈语义与冲突处理得到验证。
- 本地用户拒绝的边不得因下一次 OV 构图重新出现为有效边。
- **规则版打底：** `kg.rs` 与 `db/kg.rs` 的规则边、权重和用户裁决始终保留且优先。

### 4.4 会话摄取

- OpenViking 不直接读取 Agent 目录；输入只来自 `collector.rs`、`collectors_ext.rs` 的归一化结果。
- 最小顺序为：`POST /api/v1/sessions` → messages 或 messages/batch → commit → 按策略 extract。
- 按需读取 `GET /api/v1/sessions/{id}/context` 与 `/api/v1/sessions/{id}/used`。
- 本地保存稳定 session 映射、内容 hash 与增量游标，使超时重试不产生重复会话或消息。
- 项目范围、时间范围和正文级别都必须由用户显式选择；默认采用最小必要内容。
- **规则版打底：** 本地采集、分类、指标、成本和规则工作记忆报告继续独立生成。

### 4.5 检索与隐私

- 按场景使用 `POST /api/v1/search/find`、`/api/v1/search/search`、
  `/api/v1/search/recall`、`/api/v1/search/grep`、`/api/v1/search/glob`。
- 本地规则命中与 OV 语义命中必须可区分，结果必须可追溯到本地对象或 `viking://` URI。
- **规则版打底：** `discovery/capability_gap.rs`、`discovery/high_value_prompt.rs`、
  `discovery/keywords.rs` 先产出候选；`discovery/llm_enhance.rs` 和 OV 都只是可失败增强。
- 发送前读取并按需配置 `GET/POST /api/v1/privacy-configs/...`，但仍须先在 SkillMint 本地最小化和脱敏。
- 不得把 OV privacy config 当成“正文不会进入 Ark”的充分保证。

## 5. 分阶段路线

### P0：只读探针、健康与能力探测

范围：默认禁用，只探测，不上传 skill、不摄取 session、不构建关系。

- 所有调用必须同时通过 `remote_enabled` 与 OpenViking 专属 opt-in。
- 状态至少区分：未启用、可用、认证失败、能力不兼容、暂时不可达。
- 只调用 4.1 所列只读 endpoint；不改变现有页面和任务的结果。
- `remote.rs` 承担远程总门控语义，`settings.rs` 沿用非敏感配置与 Keychain 实践。

验收标志：

- 远程总开关或 OV 专属开关任一关闭时，网络请求数为零；
- OV 停止、超时或 key 错误时，能显示准确状态且不阻塞 UI；
- E1/E2/E3 的现有规则功能与关闭集成前完全一致。

### P1：E3 技能关系图谱增强

P1 先做 E3 图谱，而不先做 E2 会话摄取。
技能内容的真相边界更清楚，敏感面与数据量通常低于完整会话，
并且 `kg.rs` + `db/kg.rs` 已提供可直接对照的规则基线。
这样可先验证 OV 0.4.x 契约、同步幂等和关系质量，再触碰更敏感的会话数据。

- 将 npx/Hub 技能写成带 device/account 所有权标记的可重建副本。
- 完成 validate、受控 upsert、`POST /api/v1/relations/build_graph` 与关系拉取。
- OV 边作为独立来源合并，不覆盖规则边或用户确认/拒绝。
- 同步任务复用 `scheduler/` 的任务记录与失败隔离；部分失败不影响下次重试。
- **规则版打底：** OV 不可用时继续展示 `kg.rs` + `db/kg.rs` 的本地图谱。

验收标志：

- OV 可用时出现可辨识来源、理由和对应技能的新增关系；
- 关闭或断开 OV 后立即退回现有规则图谱；
- npx locks、canonical store、Hub 和 Agent 目录不因投影发生变更；
- 删除 OV 副本后能从本地真相源完整重建，重复执行不生成重复副本。

### P2：E2 会话记忆摄取与上下文召回

- 仅从已归一化的本地采集结果摄取，不另开原始目录读取路径。
- 支持项目、时间与内容级别授权；本地脱敏后再执行 session → batch → commit → extract。
- 保存本地 session ID 与 OV ID 的幂等映射和增量游标。
- 用 context、used、search/recall 增强工作记忆报告或会话回看。
- 明示 privacy-config 状态、实际发送范围和 Ark 潜在出网。
- **规则版打底：** `metrics.rs`、`pricing.rs`、`classifier.rs`、`analyzer.rs` 与规则报告不依赖 OV。

验收标志：

- 重试不会产生重复 session/message；
- 取消授权后立即停止新增摄取，并明确已有副本的处理选择；
- OV 或 Ark 不可用时，本地采集、指标、成本和规则报告结果不变；
- 用户能在发送前预览范围，并在发送后从审计中核对数据类别。

### P3：E3 发现增强与 Later 推荐

- 用 `/api/v1/skills/find` 和 search 系列为现有规则候选补充语义证据。
- 保留 `discovery/` 的规则候选、去重、冷却、每日上限和人工采纳流程。
- OV 只能补充候选、重排或解释，不能自动创建、安装或发布技能。
- 最终 Skill 草稿仍由 `hub.rs` 写入私有 Hub，安装仍由 `npx.rs` 执行。
- “任务驱动推荐”仍属于 Later，只有 P1/P2 数据质量达标后才启用。
- Resources 与 Watches 延后到明确用例出现后：`POST /api/v1/resources`、
  `POST /api/v1/resources/temp_upload`、`/webdav/resources/...`、`/api/v1/watches...`。
- **规则版打底：** 收件箱、重复模式、能力缺口和高价值 Prompt 的现有结果不得减少。

验收标志：

- 每条增强建议能同时展示本地规则证据与 OV 证据；
- 禁用 OV 后候选数量与能力不低于当前规则版；
- 采纳建议不会绕过 Hub 创作和 npx 安装生命周期。

### P4：可移植性与运维闭环

- 以 `POST /api/v1/pack/export`、`POST /api/v1/pack/backup` 提供可携出口。
- 以 `POST /api/v1/pack/import`、`POST /api/v1/pack/restore` 支持迁移与恢复。
- 以 `POST /api/v1/snapshot/commit`、`POST /api/v1/snapshot/restore` 管理 OV 派生状态快照。
- console/stats endpoint 仅展示集成健康、摄取量、提交和审计信息。
- **规则版打底：** 本地 SQLite 可重建、Hub 与 npx 真相源的恢复流程均不依赖 pack/snapshot。

验收标志：

- 用户可以导出并在隔离环境恢复 OV 派生数据；
- 恢复操作不改变本地 Agent 会话、Hub 或 npx 安装状态；
- 审计能够回答何时发送了何种数据类别，但不泄露正文与凭据。

## 6. 隐私、离线与降级

### 6.1 门控原则

- `remote_enabled` 是不可绕过的总开关；关闭即零 OV 请求。
- 还需要独立的 OV opt-in，避免“允许访问 skills.sh”被误解为“允许上传会话”。
- 会话正文进入 OV/Ark 应再取得内容级明确同意；技能投影同意不能自动扩张到会话。
- localhost 不等于完全离线：当前 embedding/VLM 经过 Ark，设置与发送确认必须披露。
- 即使 OV privacy config 已激活，SkillMint 仍先做本地最小化、脱敏和范围过滤。

### 6.2 降级矩阵

| 状态 | 行为 | 规则版保障 |
|---|---|---|
| `remote_enabled=false` | 不探针、不同步、不重试 | E1/E2/E3 全部本地运行 |
| OV 专属 opt-in 关闭 | 不访问 OV；其他已获准远程能力不受影响 | 同上 |
| OV 服务不可达或超时 | 快速失败、有限重试、记录脱敏状态，后台任务隔离 | 采集、指标、发现、图谱和 npx 生命周期继续 |
| API key 无效 | 停止重试并提示重新授权，不回显 key | 本地功能继续 |
| OV 可达但 Ark 不可达 | 标记 AI 子能力不可用，不假定本地存储等于处理成功 | 规则关系、规则报告和发现继续 |
| 单个 endpoint 不兼容 | 禁用对应增强能力，不把整套集成判死 | 其他 OV 能力可独立探测；本地基线不变 |
| 部分同步失败 | 保留幂等游标与待重试项，不发布半成品为规则结果 | 已有本地结果不被覆盖或删除 |

停用集成时必须区分三种动作：

1. 停止同步：保留 OV 副本与本地映射，不再新增数据；
2. 清理 OV 副本：仅删除可证明属于当前 device/account 的派生数据；
3. 清理本地映射缓存：不删除 Agent 会话、Hub 内容、npx 状态或规则结果。

`POST /api/v1/pack/export` 是用户带走 OV 数据的首选出口；
pack/export、backup 或 snapshot 都不改变“本地 Agent/npx/Hub 才是真相源”的判断。

## 7. 风险与缓解

| 风险 | 检测信号 | 缓解措施 | 残余风险 / 阶段 gate |
|---|---|---|---|
| 本机 Docker 生命周期、端口占用、启动时序或 key 失效使 localhost 不稳定 | 连接、超时、认证分类状态 | P0 探针；短超时、有限重试、熔断；任务隔离 | 无法消除外部进程故障；P0 未稳定不得进 P1 |
| OV 仍处于 0.4.x，路径、schema、语义或错误码演进 | 契约 fixture 失败、能力探测不兼容 | 固化 v0.4.10 fixture；逐 endpoint capability gate；不猜字段 | 升级仍需适配；关键读写契约未验证不得进 P1/P2 |
| local storage 仍通过 Ark 做 embedding/VLM，与默认离线存在张力 | audit、tokens、Ark 错误与发送确认 | 两级 opt-in；显式出网披露；本地最小化/脱敏 | 云端处理边界需供应方事实支持；未核清不得默认摄取正文 |
| 双侧副本造成重复摄取、删除漂移、同名技能冲突和孤儿数据 | hash/游标冲突、对象数异常、无法归属的副本 | 稳定外部 ID、内容 hash、device/account 标记、幂等 upsert、所有权删除 | `/skills` 命名/元数据能力未核实前不得做破坏性同步 |
| OV 关系质量与规则边或用户反馈冲突 | 重复率、冲突率、人工拒绝率 | provider 分层；规则与用户裁决优先；P1 先只读合并 | 自动关系仍可能制造噪声；质量 gate 未达标不进推荐 |
| 会话或 Skill 正文含密钥、源码、客户数据或 prompt injection | 本地扫描命中、异常内容类别、审计抽查 | 最小范围、脱敏、预览、项目白名单、正文日志禁用 | 无法保证自动脱敏穷尽；P2 必须保持显式授权 |
| `device_id` 与 OV account 的换机、重装、多用户隔离语义不明 | 跨设备重名、孤儿 account、权限异常 | P1 先标记 device；核实 `/api/v1/admin/accounts...` 后再决定账户映射 | 语义未定前不承诺跨设备无缝合并 |
| pack/snapshot 的兼容性、加密、保留期与删除保证未知 | 跨版本恢复失败、产物含敏感字段 | 隔离恢复测试；导出前提示；用户控制保留与删除 | 未验证前仅作实验性可携出口，不作为唯一备份 |

## 8. 开放问题

以下问题不阻塞本文结论，但分别是 P1、P2 或 P4 进入实现/发布前的 gate：

1. v0.4.10 各 endpoint 的精确请求、响应、分页、错误码和幂等语义是什么？
2. `/api/v1/skills` 是否支持客户端元数据、namespace 或稳定外部 ID？同名全局/项目/npx skill 如何隔离？
3. `POST /api/v1/relations/build_graph` 的作用域是 account、全库还是指定 URI？删除后如何收敛？
4. OV 自动关系如何映射到现有关系类型、权重与理由？需要独立缓存还是扩展现有 provenance？
5. 用户确认/拒绝边是否应写入 relations/link？P1 默认仅本地覆盖，不双写未确认语义。
6. 会话默认只发送标题/摘要、Prompt 还是完整消息？建议默认最小摘要，正文需二次同意。
7. `/api/v1/privacy-configs/...` 在 embedding/VLM 前覆盖哪些字段，能否由 audit 证明敏感内容未进入 Ark？
8. `device_id` 应只是 OV 元数据，还是映射到 `/api/v1/admin/accounts...` 的独立账户？
9. pack/export 与 backup 产物是否加密、是否含正文/key/audit、跨 0.4.x 是否兼容？
10. P1 成功阈值采用新增边数、人工接受率、冲突率还是规则图谱覆盖提升？
11. Watches 的 URI 触发语义和去重保证是否足以替代部分 `scheduler/` 轮询？在核实前不接入。

## 9. 决策摘要

- 产品定位：OpenViking 是飞轮的外置上下文引擎，不是数据或安装真相源。
- 集成顺序：P0 只读探针 → P1 E3 技能图谱 → P2 E2 会话记忆 → P3 发现/推荐 → P4 可移植性。
- 所有权：会话归 Agent 目录，技能归 npx/Hub，SQLite 与 OV 均为可重建派生层。
- 控制面：`remote_enabled` 总门控 + OV 独立 opt-in + 会话正文二次同意。
- 失败语义：OV 永远只增强；不可用、未启用或不兼容时，规则版完整运行。
- 安装语义：任何推荐最终仍经 `hub.rs` 沉淀、经 `npx.rs` 执行生命周期变更。
