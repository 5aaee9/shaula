---
status: accepted
date: 2026-09-25
---

# Project diagnostics from reconciliation evidence

- Contract: [spec 0041 — Explainable Reconciliation Diagnostics](../specs/0041-explainable-reconciliation-diagnostics.md).
- Inspected baseline: `5aaee9/shaula@881022ea9de104494788bd5f199565695acc8bfb`.
- Implementation boundary: 只读诊断投影已进入本地实现；实时平台验收独立记录于 [实施状态](../IMPLEMENTATION_STATUS.md#explainable-reconciliation-diagnostics-2026-09-26)，不得由设计或本地测试推定完成。

## Context

Shaula 已经拥有持久化生命周期、Fleet conditions、Jobs association 和执行日志。问题不是完全没有状态，而是多个不同的运行事实被压缩成单个 phase/reason，或者只出现在日志中；操作者仍难以回答“为什么没扩容、为什么没上线、为什么没清理、为什么更新没生效”。

固定基线的静态检查发现：supervisor 可以只设置 `blocked`，status writer 将缺失 reason 回退为 `OwnershipProofFailed`；pool admission 多种不同拒绝都返回 None；follow cascade 的不同延后原因主要留在 WARN/DEBUG；Fleet status 组合多个读取并输出 boolean conditions。这些现象支持新增解释契约，但不是已经完成动态缺陷复现。具体代码链接与边界集中在 [spec 0041 §2/§12](../specs/0041-explainable-reconciliation-diagnostics.md#2-inspected-baseline-and-actual-gaps)。

同一 Fleet 可以同时存在多种事实：listener 已连接、占用已满、一个 Generation 正在等待上线、另一个等待注销、模板升级等待排空。单一“健康/不健康”不能表达这些事实之间的关系。更危险的是，旧 inventory、一个 ApplyStarting intent、一次日志错误或 Job 与 Runner 的弱关联，很容易被 UI 展示成当前因果或安全删除许可。

因此需要一个面向问题、具有证据来源和有效期、只读且可降级的诊断面；不是另一个调度器、自动修复器或运行日志搜索服务。

## Decision

### 1. Use question-oriented reports instead of another global status

首发为 Fleet、Generation、Job 提供独立诊断读取，分别回答扩容、上线、清理、配置收敛和任务分配的可观察部分。每个问题有自己的 outcome、coverage、stages、reasons 与 evidence。

理由：一个正常等待可以与另一个真正阻塞共存；未评估、未知、数据过期、功能不适用不应该被挤进 boolean false。primary reason 只是当前问题最直接的已知阻塞，不是系统找到的唯一根因。

代价：客户端必须展示多个局部结论，不能再只依赖红绿 badge。保留现有 status/conditions/Change 语义，不为此进行破坏性字段转换。

### 2. Capture typed outcomes at the real decision boundary

真实 supervisor、admission、Runner operation 和 follow cascade 在执行原分支时生成类型化观察。容量解释使用实际计算的原输入组；只读账本推导另标 `ledger_projection`。HTTP 和前端不复制一套准入规则，不解析自由日志推导原因，也不运行“只为了看看会怎样”的随机选择。

理由：只有原判断位置知道该次操作在哪个 gate 停止、后续 gate 是否执行、提交是否成立。独立 shadow evaluator 会在并发、重试、不同 backend 与版本演进中产生漂移。

内部返回类型可增加丰富 outcome，但原有分支、capacity reservation、随机抽样次数、guard 和副作用次序不能因此改变。新增观测缺口显示 unknown，而不是用另一个平台调用填补。

### 3. Keep a bounded, disposable latest projection outside correctness-critical commits

首发保存每个 subject/question/producer lane 的最新诊断快照，通过有界、非阻塞 sink 交付到独立短事务。它不是每轮 reconcile 的历史事件日志，不新增通用 event bus，不是 Worker checkpoint 或资源清理证据。

理由：解释需要在页面访问时可读，且某些 early-return 原因无法从最终账本重建；但诊断磁盘写入和展示失败不应该阻塞 Runner 清理或导致重复副作用。

代价：诊断可能有短暂缺口、被合并或被丢弃，必须诚实展示 partial/unknown。队列满、写入失败时不能继续把旧快照作为当前正常证明。原有权威数据库、安全记录、ACK 与 `fleet_set_observed` 的错误语义保持不变；“诊断可丢弃”不适用于这些业务数据。

在这一取舍下，全量删除诊断投影不应改变任何资源恢复结果。只有诊断不再完整，而不是系统因此失去销毁能力。

### 4. Bind identity, producer order and freshness explicitly

快照绑定现有 incarnation/revision/fences、适用的 session/auth/worker 身份，以及独立的 observer epoch/sequence。sequence 以工作开始顺序分配，不能让慢请求的迟到响应覆盖新观察。跨 question/lane 不冒充一个分布式原子快照。

HTTP 的 generatedAt 与证据 observedAt 分开；GET 或失败 poll 不刷新成功时间。非终态观察按诊断时效降级，process-local 证据在重启后失效；仍由当前账本支持的 terminal fact 不因 observer 重启被抹去。时间不可信、身份冲突或数据缺失均降低结论强度。

理由：显示一个旧 blocker 比显示“unknown”更像有帮助，但可能引导操作者修改无关配置或误清资源。带来源和当前性约束的局部事实优于无条件的完整叙述。

这些 epoch 和 TTL 只影响解释，绝不成为新的副作用 lease、worker fencing 证明或 Runner timeout。

### 5. Preserve backend, pool and cleanup semantics

GitHub assigned demand 与 Forgejo waiting demand 分开；shared pool 与历史 inline pool 的 cap/backpressure 规则按原契约解释。权重仍控制 Runner 模板选择，不保证精确 Job 比例。未实际检查的 pool 成员不能被诊断成 unhealthy。

普通 busy-safe cleanup 与明确允许中断任务的 hard lifetime 分开；资源销毁 checkpoint、远端注销和 occupancy 释放分开。Forgejo idle 不提供 acquisition fence，诊断不能绕过 [现有 drain 边界](../forgejo-drain.md)。

Job 只有满足既有 Verified association 和身份绑定条件，才能引用指定 Generation 的执行关联。其他情况下提供 Fleet 背景，但必须明确不是该 Job 的已证实等待原因。

[ARD-0040](0040-destroy-generations-that-never-started-a-create-apply.md) 的 never-started 清理规则保持，[ARD-0035](0035-finalize-quarantined-generations.md) 的人工账本处置也保持。解释不能将两者伪装为一次真实 Terraform destroy，更不能改变历史隔离记录。

理由：诊断应帮助理解已有安全模型，而不是成为重新定义生命周期的隐蔽渠道。

### 6. Add versioned read surfaces with existing authorization

采用三个新增 GET，而不是重写现有 `/status` 或嵌入无限增长的动态数据。新 envelope 明确 schema、partial/unknown、预算与 Content-Type。基础读取仍需 `fleet.read`；Auth、Template/Pool 详情与日志正文分别沿用对应权限。

建议由有限 catalog 生成，只导航到现有受保护的查看/编辑流程。它不是 shell command、任意 URL 或可直接执行的修复脚本；真正操作仍重新获取原 resource version，并经过原授权、CSRF、幂等和业务门槛。

理由：诊断是新的信息聚合路径，也是新的泄漏面。持有 Fleet 读取权限不应该通过聚合接口获得日志正文、凭据、隐藏目标或任意 provider 错误。

不增加匿名诊断端点，不改变 OIDC 启动要求，不将诊断快照 ID 当作 mutation 的 If-Match。缺少诊断功能的旧 server 应被识别为不支持，而不是误报目标资源已删除。

### 7. Deliver independently of Worker, Token and telemetry rewrites

当前生产 runtime 可以接入诊断，不必等待 spec 0010 的 Worker/state 集成。spec 0039 的 client/CLI 可用后，增加同语义 explain 命令与验收；此前不宣称 CLI 已交付。

延续 [ARD-0032](0032-process-telemetry-uses-otlp-http.md) 的边界：`shaula-observability` 继续拥有 exporter，core 不新增 telemetry port，OTLP 故障不影响资源生命周期。诊断接口不依赖外部日志/指标平台；有限降级计数可以辅助维护，但不是事实来源。

理由：本功能的关键难点在证据语义、源头覆盖、权限和并发，而不是替换 exporter 或扩展控制平面部署拓扑。

## Alternatives considered

| 方案 | 不选择的原因 |
| --- | --- |
| 只在 UI 翻译 lastError / 按日志正则分类 | 不能恢复已经丢失的分支语义；字符串变化、敏感文本和未知状态容易产生错误结论。 |
| GET 时实时调用 GitHub/Forgejo/provider 做诊断 | 把页面轮询变成探测负载与权限旁路；调用失败又会混淆控制器本身的历史决策。 |
| 运行一次 dry-run scheduler 来预测下一步 | 读取期间的并发状态不同；可能消耗 RNG、误推成员或变成第二套安全门槛。 |
| 一个全局 rootCause / health score | 正常等待、未知、历史错误和多个独立问题不可压缩成单一正确结论。 |
| 在所有业务事务中强制记录完整诊断事件 | 将解释持久化变成业务可用性前置项，扩大写入和故障面；本功能不需要事件溯源重放。 |
| 只使用 OTLP / 日志平台回答 why | 需要外部服务与日志权限；丢失导出、采样和不可用时不能代替原账本。 |
| 同时引入因果图数据库或 AI 根因推断 | 当前证据覆盖不足，先建设可验证的事实投影更直接；推断不能替代删除许可。 |
| 根据诊断提供自动 retry / force cleanup | 旧快照和弱证据不能授权新副作用；必须另行设计和验收恢复操作。 |

## Consequences

收益是把已存在的控制器判断变成稳定、可检验、跨 UI/API/client 共用的解释；同时使未知与协议局限成为显式产品行为，而不是隐藏在日志中的例外。

成本是各真实分支需要维护观察点，新增投影 schema/CAS 与 read API，客户端需要处理 partial/stale 和权限遮蔽。新增 reason 的 code、参数和 producer 映射构成长期兼容性责任，不能任意复用已有 code 表达不同含义。

本设计不保证总能找到根因，不预测 Job 排队位置或准确启动时间。它保证的是：有证据的解释可以追溯，没有证据的部分明确保留为未知；诊断丢失不会改变资源处理结果。

首发不提供全历史 timeline、全局诊断查询、告警发送、独立 Profile/Pool explain、自动修复或改变调度策略。后续确有需求时，应以此 read model 为基础另立契约，而不是扩大现有字段的含义。

## Implementation and acceptance

按 spec 0041 的 A–E 批次实现：纯模型与 catalog → 实际 producer 与投影 → HTTP/权限 → Web 与后续 client → 真实场景。协议字段、限额、保留期与 30 项 DX 验收以 [spec 0041](../specs/0041-explainable-reconciliation-diagnostics.md) 为唯一 owner，本 ARD 不维护第二份数值清单。

核心发布要求是相同故障输入下，启用/禁用诊断或让诊断写入持续失败，原资源副作用和安全结果不变；同时旧 epoch 不覆盖新观察、GET 无外部副作用、所有可见原因都有来源与当前性边界，所有聚合字段都遵守原读取权限。

本 PR 的检查只验证 Markdown/JSON 示例、文档链接、目录编号和文档差异；不是 Rust、HTTP、Terraform 或真实平台验收。实现进度仍由 `IMPLEMENTATION_STATUS.md` 维护，不能因本决定文档存在就改为已实现。
