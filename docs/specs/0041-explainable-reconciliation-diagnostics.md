# Explainable Reconciliation Diagnostics

- Status: **Accepted contract**. 实现与本地/真实平台验收边界见 [实施状态](../IMPLEMENTATION_STATUS.md#explainable-reconciliation-diagnostics-2026-09-26)。
- Date: 2026-09-25
- Inspected baseline: `5aaee9/shaula@881022ea9de104494788bd5f199565695acc8bfb`
- Decision: [ARD-0041](../ard/0041-project-diagnostics-from-reconciliation-evidence.md)
- Extends: [Fleet control plane](0002-fleet-http-control-plane.md)、[Web UI](0008-embedded-web-ui.md)、[Jobs/logs](0019-workflow-jobs-and-operation-logs.md)。
- Preserves: [Worker/state](0010-lifecycle-worker-and-http-state-backend.md)、[follow latest](0023-fleet-template-follow-latest.md)、[Forgejo](0026-forgejo-runner-backend.md)、[finalize](0028-quarantined-generation-finalize.md)、[inline pool](0029-weighted-template-pool.md)、[shared pool](0037-shared-template-pool-resource.md)。

本文中的 MUST / MUST NOT / SHOULD 分别表示必须 / 禁止 / 建议。除 §2 代码基线外，类型、接口、字段、默认值和 UI 描述目标契约。本文只拥有**诊断投影**，不重新定义资源准入、调度、删除许可或 Change 完成条件。

## 1. Outcome and scope

操作者能够从同一份服务端结构化解释回答：

| 问题 | 首发入口 | 必须解释的区别 |
| --- | --- | --- |
| 为什么没有继续创建 Runner？ | Fleet | 没有需求、容量已足够、占用上限、池成员限制、实际执行准入受阻、观察不可用 |
| 为什么资源创建后 Runner 尚未可用？ | Generation | 已授权 apply、资源创建成功、bootstrap、等待库存确认、不确定结果不是一回事 |
| 为什么尚未清理？ | Fleet / Generation | 正常忙碌、缺少安全证据、Destroy 失败、远端注销未完成、隔离 |
| 为什么配置没有生效？ | Fleet | desired/observed、认证交接、模板/池 follow lag、输入不兼容、等待零占用 |
| 为什么这个 Job 尚未运行？ | Job | 已观察任务状态、关联可信度、Fleet 容量背景、无法证明具体分配原因 |

首发只交付 Fleet、Generation、Job 三类只读诊断。Profile/Pool 依赖只作为该对象的受控证据；独立 Profile/Pool/Change explain 接口、全局历史搜索、告警投递、预测 ETA、自动修复和新调度策略不在首发范围。

不依赖 spec 0010 的生产 Worker 集成或 spec 0039 的 Token/client 先完成。当前 runtime 可以产生解释；未来 Worker 接入同一投影契约。CLI 对等是 spec 0039 实现后的增量验收，不得在其缺席时宣称命令已可用。

## 2. Inspected baseline and actual gaps

以下结论来自固定 commit 的静态源码阅读，非动态缺陷复现。源文件索引见 §12。

| 代码事实 | 本提案的改动依据 |
| --- | --- |
| `ReconcileReport` 只有一个 `blocked` 和可选 `reason`；执行上下文未就绪可仅设置 blocked。`observe_reconcile` 将无 reason 的 blocked 回退为 `OwnershipProofFailed`（S02/S03）。 | 在真正的退出分支产生有类型原因；不要从 phase 或通用错误回推唯一根因。 |
| Fleet status 组合多个读取，公开三个 boolean conditions 和四个容量数；`dependencies.template` 为 None（S04）。 | 新响应记录依据、时效和一致性范围；保留旧 boolean wire，不偷换成三态字符串。 |
| `fleet_set_observed` 同时推进 Fleet observed head 和 Changes，并检查 incarnation、revision、mutation fence、session epoch（S05）。 | 诊断不能调用此方法来宣告 Ready/Change Succeeded，也不能把它的失败隔离边界当作普通日志。 |
| pool admission 的多种退出都返回 `None`；shared pool 与历史 inline pool 的 cap 规则不同（S06）。 | 补充结构化 admission outcome；记录实际候选集，不在 GET 中重新抽样。 |
| follow cascade 的输入不兼容、backend 不兼容、无 Active、占用和竞争分支主要返回 false / WARN / DEBUG（S07）。 | 在这些实际分支记录 rollout 原因，包括未到达后续门槛的情况。 |
| Forgejo 独立观察 waiting/running、inventory；失败的 demand 读取不刷新成功时间，Jobs history 失败不自动阻止生命周期（S08/S09）。 | 不把 GitHub assigned demand 和 Forgejo waiting jobs 混为一谈；保留非阻塞的观察失败。 |
| Jobs 有 Unverified / Verified / Ambiguous，Generation 有 subphase；HTTP 将日志正文权限与普通 history 读取区分（S10/S11）。 | 不发明 Job→Generation 关系，不通过解释接口绕过 `logs.read`。 |
| UI 展示 phase、reason 和 lastError；通用 badge 使用字符串匹配选择样式（S12）。 | 新诊断使用明确 outcome、severity 与模板化消息，旧展示保持兼容。 |

另外，[ARD-0040](../ard/0040-destroy-generations-that-never-started-a-create-apply.md) 已接受“未开始 Create apply 且注册消失得到证明”可直接终结的规则。本提案必须解释该路径，不能把一切 plan/JIT 失败重新标成需要隔离。历史隔离记录也不能因诊断回算而自动解除。

## 3. Non-negotiable boundaries

1. **只解释，不授权。** Diagnostic、suggestion、trace、日志、heartbeat 均不是 Create、Busy-safe removal、Destroy、finalize 或释放 occupancy 的许可。现有 domain ledger 和准入事务继续唯一拥有这些决定。
2. **不运行第二套控制器。** HTTP GET 不调用 GitHub/Forgejo/provider，不执行 Terraform、库存刷新、认证探测、重试、随机选成员或 mutation。页面刷新不是 reconcile trigger。
3. **来自实际路径。** 决策时解释取自执行该分支的类型化结果；只读账本计算只能标为 `ledger_projection`，不能冒充“控制器刚刚作出的决定”。
4. **没有证据不是正常。** 未评估、数据过期、能力不支持、权限不足、观察失败分别表示，不填虚构的零值、成功或删除许可。
5. **诊断失败不能破坏业务。** 新增投影写入/渲染/导出失败不得取消已提交操作、改变原控制流或阻塞正常回收。原有业务数据库、安全记录和 `fleet_set_observed` 的错误语义不因本提案减弱。
6. **后端与对象身份不混用。** Fleet incarnation、Generation ID、Job attempt、pool revision、认证上下文和 session/worker epoch 均按所属契约绑定；同名替换对象不能继承解释或处置建议。

## 4. Public explanation model

### 4.1 Envelope and questions

新增 `DiagnosticReportV1`，字段采用 camelCase，与新接口单独版本化：

| 字段 | 契约 |
| --- | --- |
| `schemaVersion` | 固定整数 1；未知主版本不能按 v1 解释。 |
| `subject` | `kind`（fleet / generation / job）、稳定 `key` 或 `id`、已知 `fleetIncarnation`。不能从名字猜补历史缺失身份。 |
| `generatedAt` | 此次本地读取时间，RFC3339 UTC 毫秒；**不是**证据刷新时间。 |
| `questions` | 适用于该对象的有限问题；问题 ID 固定为 `scale_up`、`readiness`、`cleanup`、`rollout`、`job_dispatch`。 |
| `related` | 有权限且身份绑定正确的 typed resource references；不是任意 URL 或隐含关联证明。 |
| `truncated` | 超过预算时为 true，同时相关 question.coverage 不得为 complete。 |

每个 question 包含：

- `outcome`: `satisfied` / `progressing` / `blocked` / `unknown` / `not_applicable`。satisfied 只针对该问题，不代表 Fleet 全局健康。
- `coverage`: `complete` / `partial` / `none`；说明适用证据的覆盖程度，不是置信度分数。
- `basis`: `kind`（`recorded_decision` / `ledger_projection` / `unavailable`）、可空 `observationId`、`observedAt`、`validUntil`、`freshness`、`subjectRevision`。freshness 是 `fresh` / `stale` / `missing` / `not_applicable`。
- `stages`: 固定 stage ID、`evaluation`（`passed` / `blocked` / `pending` / `not_evaluated` / `unknown` / `not_applicable`）、对应 reason IDs。stage 是解释视图，不是新增生命周期状态机。
- `primaryReasonId`: 仅从当前、已授权可见、确实阻塞该问题的原因中选择；否则 null。
- `reasons`、`evidence`、`suggestions`: 见下文；可空 `capacity` 见 §6。

Fleet 固定返回 scale_up、cleanup、rollout；Generation 返回 readiness、cleanup；Job 返回 job_dispatch。所需来源缺失时保留问题并标 unknown，而不是省略问题。非适用问题可以明确 not_applicable。

known blocker 与 partial coverage 可以共存：已经在 occupancy 门槛停止时，可以明确说明这一阻塞，并把未到达的 pool/bootstrap 阶段列为 not_evaluated。不得把未到达的阶段写成 passed。

没有当前有效 blocker、但所需证据不完整时 outcome 必须是 unknown，不因为 `reasons=[]` 显示 satisfied。progressing 也必须有当前进度/等待证据，不从“没有错误”推断。

### 4.2 Reasons, evidence and message safety

一个 `DiagnosticReason` 含：`id`、稳定 `code`、`stage`、`severity`（info / warning / error）、`effect`（blocking / informational）、有限类型 `parameters`、`evidenceIds`、`firstObservedAt`、`lastObservedAt`。

code 属于独立 `DiagnosticReasonCode` catalog；不扩充或替代现有 mutation `ReasonCode` 的含义。每个 code 的适用对象、参数 schema、默认 severity、消息模板、证据要求和允许的 suggestions 必须集中维护并测试。

证据必须区分：

| kind | 可以证明 | 不能证明 |
| --- | --- | --- |
| `ledger_fact` | 已提交的 intent、result、状态、pin、回收 checkpoint | 仅凭 ApplyStarting 证明子进程还在运行 |
| `controller_observation` | 指定 revision/epoch 下某次实际门槛结果、重试计划、等待状态 | 未来仍然满足该门槛 |
| `backend_observation` | 指定 authority、采集时刻和成功范围内的库存/任务事实 | 404 必然不存在；idle 必然可安全删除 |
| `derived_calculation` | 从列出的同一输入组得到的容量差额或版本 lag | 创建已获准、任务一定会分给某个 Runner |

每项 evidence 包含公开 ID、kind、observedAt、freshness、可空 validUntil，以及其 code 允许的结构化数据。受保护的 authority/fence 用于服务端验证，不通过任意 metadata map 向外输出。

MUST NOT 复制 `error.summary`、provider response body、Terraform plan/state、shell/env、Token、bootstrap、原始日志或用户提交的任意文本来生成 reason。不得把正则脱敏当作唯一防线。用户名、URL、输入字段值不得进入自由拼接的修复命令。消息由代码内 allowlist 模板产生并按文本渲染，不能解释 HTML/Markdown。

未知 code 对旧 client 保留字面 code 并显示“当前客户端不认识此原因”；未知参数不驱动行为。未知 outcome/freshness 按 unknown 保守展示。服务端未知错误映射到 `observation.unclassified_failure`，而不是推断网络故障或凭据过期。

### 4.3 Primary reason, severity and ordering

primary 不是“唯一根因”。实际 producer 在该问题中先遇到的有效阻塞为优先；来自其他 lane 的同时原因仍分别保留。没有实际顺序时，按固定 `stage` 次序、code、稳定证据 ID 排序，不能使用自由文本或随机顺序。

`scale_up` 的展示门槛次序为 authority → demand → capacity → pool → execution_admission；`rollout` 为 dependency_resolution → input_compatibility → occupancy → commit_fence。readiness 使用 create_effect → bootstrap → runner_inventory；cleanup 使用 intent、safe_removal、execution_admission、resource_cleanup、registration_cleanup、completion；job_dispatch 使用 job_observation → association → execution_context。cleanup 的排列只是稳定显示顺序，各项 evaluation 必须来自实际 backend/lifecycle 路径，硬寿命的先资源后注销不能被改成普通 busy-safe 删除顺序。

severity 不决定删除许可。正常容量满足是 info；等待 Busy 完成可以是 info；失去安全证据可以是 warning/error。不得把所有非终态或正常等待染成错误。

同一个问题可以包含非阻塞观察错误。例如 Job history 不可用时仍可能正常清理，不允许它覆盖 cleanup 的真实进度。

## 5. Acquisition, persistence and freshness

### 5.1 Capture where the decision happens

生产者在原有分支形成轻量 `DecisionObservation`：捕获 subject、question/lane、实际使用的输入、结果、时间和内部 fences。它应当与原返回值一起产生，或来自同一 pure classification result；不能维护一个“据说等价”的前端/HTTP 分支树。

需要新增观察点的最小集合：supervisor 构造/准入失败、auth handoff、listener/demand、容量计算、pool admission 的每种 None 分支、create/destroy semaphore 等待、apply/bootstrap/readiness、普通清理/硬超时/注销 checkpoint、follow cascade 和 Jobs association。没有构造出 supervisor 的 Fleet 也需要由 assembly 路径说明依赖失败；不能永远只显示“无诊断”。

`admitted`、`started`、`completed` 必须在对应事实提交/确认后记录。提交前的计划只能叫 `planned` / `pending`；丢失外部调用响应只能说明 outcome unknown。不能为了补齐解释额外读取平台或延迟副作用。

### 5.2 Read projection, not an event-sourced scheduler

首发增加独立的**最新诊断快照投影**，每个 `(subject identity, question, producer lane)` 只保留最新一条；不记录每个 tick 的全量历史，不建通用 event bus。

producer lane 由有限 enum 定义，不能按 tick/request/operation ID 动态创建新 lane；每个 subject 的 lane 数量由 catalog 固定约束。

逻辑字段包括 payload version、subject identity、捕获的 domain guard、observer epoch、递增 sequence、observedAt/validUntil、类型化 payload 和 reason-transition fingerprint。内部 guards 不属于公共响应。

业务路径通过有界 nonblocking sink 发布。提案默认：全进程待写队列最多 1024 个条目，同一 key 可合并；单条编码上限 64 KiB。饱和、超限、序列化或写入失败只增加有限诊断降级计数，并使受影响 lane 的当前性失效；无法细分 key 时保守失效该 observer epoch。不能继续把旧记录当作 fresh 的“全都正常”。

快照在**独立短事务**落库，不把可选诊断持久化放进现有 mutation、ACK、安全记录或 completion 的提交前置条件。权威账本事实仍可在 GET 的一致性读取中直接派生。新 sink 不可持有 effect gate 等待写入。

快照写入失败后的 API 可以返回 partial/unknown；业务数据库本身不可用时仍按 §8 返回 503。两者不能混为“所有 Store 错误都忽略”。

### 5.3 Fencing and out-of-order completion

snapshot writer 必须检查 subject identity、当前 domain guard 和 producer ownership。Fleet guard 至少包括 incarnation、desired revision、mutation fence，以及适用的 session/auth context；Generation 还需其固定 pins、operation attempt 或 worker claim；Job 必须保留 backend/scope/incarnation/attempt 的既有 namespace。

producer 注册获得只用于诊断的 observer epoch；sequence 在同一 epoch 的工作**开始**时单调分配。CAS 只接受该 lane 当前 epoch 中更新的 sequence。相同 sequence 的重复内容可幂等；冲突内容拒绝并暴露 unknown。不能按 HTTP 请求完成时间或 wall clock 大小决定谁更新。

旧 supervisor、过期 session、旧 worker、旧 Fleet revision 的异步结果不能覆盖新快照。可选投影注册失败只让该 producer 暂无快照，不改变其业务准入。服务重启后，旧 semaphore/进程存活等 process-local 证据立即失效；历史 domain terminal fact 不因重启消失。

### 5.4 Coherent basis and expiry

同一份容量解释必须使用控制器实际传给计算函数的 policy/demand/counters，标为 recorded_decision；不得把旧 demand、新 policy 和随后读出的 occupancy 拼成一份“当时决定”。异步观察不是分布式原子快照，响应不得宣称所有平台同时一致。

HTTP 派生账本解释时，相关本地行在短只读 snapshot 中取得；用 `ledger_projection` 标明这是读取时判断，不是执行回执。GET 无法得到一致的必要行时返回该问题 unknown，不能以无限重试维持请求。

时效规则：

- Durable ledger fact 在绑定的版本/身份仍匹配时有效，validUntil 可以为 null；其 currentness 依赖一致性读取，而非最近写入时间。
- 非终态 runtime/backend 观察的诊断有效期默认为 observedAt 后 30 秒；所属协议已有更短时限时取较短者。该默认值只控制**展示可信度**，不修改实际 demand/Runner 寿命或安全策略。
- producer 有可靠 lease/heartbeat 截止时间时，validUntil 还必须受它限制；缺少证据不能凭空产生 heartbeat。
- GET、失败 poll、serializer、metrics export 都不能刷新 last-success 时间。新失败观察可以是当前的“读取失败”，但携带的旧成功值仍旧 stale。
- daemon 停止采集、队列丢记录、revision 变化或 source 不兼容时，读时降级，不依赖 producer 再写一条失败消息。
- observedAt 晚于可信读取时间、时钟回退或顺序冲突时，不产生负 duration 或虚构 fresh；输出 `observation.clock_invalid` / unknown。

firstObservedAt 表示当前原因**连续得到确认**的起点。仅参数计数或 heartbeat 变化不重置；原因解除、身份/版本更换、观察断档或时效失效后，不能继续声称中间一直受阻，下一次确认重新计时或用 null。迁移不回填虚构起点。

### 5.5 Retention and recovery

live subject 的快照可更新或因预算被丢弃，丢弃后显式 missing；诊断可用性不能决定资源能否被回收。terminal subject 的诊断快照最多保留 7 天，且不超过其原始可读记录的保留期。诊断 GC 只删自身投影，不延长/缩短 Jobs、logs、state、credentials、pins、tombstones 或 audit 的现有规则。

快照全部丢失时，恢复仍应由原账本完成，诊断重建为 ledger_projection/unknown 并由后续观察补齐；它不是灾难恢复必需材料。该机制不宣称实现了新 Worker backend 或备份迁移。

## 6. Capacity and backend-specific explanations

### 6.1 Exact arithmetic, explicit provenance

`capacity` 可包含以下数字，全部使用规范十进制字符串（或 null）以避免 i64/u64 经 JavaScript 丢精度：min、max、demand、target、effective、occupancy、deficit、occupancyHeadroom、arithmeticCreateAllowance、actuallyAdmitted。

GitHub `demandKind=github_total_assigned_jobs`；Forgejo `demandKind=forgejo_waiting_jobs`，running count 是另一个事实。未知/缺失 demand 为 null，不能因为旧 status 内部使用 unwrap_or(0) 就向新诊断声称真的没有任务。

在输入已知有效时复用现有 arithmetic（含 checked/saturating 边界）：

```text
target = min(max, min + demand)
deficit = max(target - effective, 0)
occupancyHeadroom = max(max - occupancy, 0)
arithmeticCreateAllowance = min(deficit, occupancyHeadroom)
```

该数是容量算术允许的上界，不是当前可以成功创建的数量；pool、authority、semaphore 和事务准入还可能拒绝。actuallyAdmitted 只来自已确认的 admission 结果，不从 allowance 复制。

分项状态计数复用 `counts_effective` / `counts_occupancy` 所属规则，绝不只数 Idle/Busy。已销毁基础设施但远端注销尚未完成时，仍按原协议计入占用。当前账本计数与 decision 时计数分别标记，不用后者伪装实时值。

### 6.2 Pool restrictions

诊断记录实际 routing mode（single / inline / shared）、使用的 pool revision、已检查成员、原始权重、cap、对应占用作用域、排除原因和真实 selection outcome。

shared pool 的 cap 作用域按现有契约为同一 pool revision 的所有引用 Fleet；不得顺手改成跨 revision 全局配额。shared pool 的 cap exclusion 与历史 inline backpressure 的差异保留。权重不是任务分配保证，也不输出随机数或声称精确任务比例。

GET 不能调用 `generation_admit_pool`，不能消耗 RNG、预占容量或生成一次假选择。未到达选成员步骤时显示 not_evaluated。平台健康没有实际探测/分类证据时不能把成员标成 unhealthy，也不能因没有错误就标为健康。

### 6.3 Readiness, cleanup and hard expiry

必须分开显示：apply 已获准/可能启动、资源创建成功、Runner 上线、实际任务执行和清理完成。`ApplyStarting` 既没有结束证明、也没有可信的当前执行观察时使用 `lifecycle.apply_outcome_unknown`；“正在执行”需要当前执行观察，不能从 intent 推断。

普通 cleanup 的 busy-safe gate 与硬寿命路径分别标记 `cleanupMode=ordinary|hard_lifetime`。硬寿命按已生效 server policy 和实际 Create success 起点解释；不能假定是 job timeout，也不能承诺控制器停止时仍能准时删除。硬寿命回收可能中断 Busy Job，必须明示。

Forgejo idle 不是 acquisition fence。[drain 边界](../forgejo-drain.md)未解决时显示 `cleanup.safe_drain_unavailable`，不能由重复 idle、进程退出或诊断按钮生成删除许可。

`Destroyed` 账本状态与人工 finalize 回执必须标明原有证据来源。finalize 成功不是再次执行 provider 删除，也不证明外部核验内容由 Shaula 自动验证。ARD-0040 路径可解释为“未获准启动 Create apply，且注册消失已确认”，而不是“Terraform destroy 成功”。

### 6.4 Rollout and job association

rollout 必须并列展示 desired/observed、旧 pin 和候选 pin（受权限控制）。没有 Active、输入不兼容、backend 不兼容、等待零占用与 commit fence 竞争分开说明；不能让 UI 一律提示排空就能修好。不提供 revision 选择或第二种 pinned 模式。

Job 解释只允许在现有 association=Verified 且当前绑定身份匹配时将指定 Generation 的证据称为执行关联。Unverified/Ambiguous 时仍可展示相同 incarnation 的 Fleet 容量背景，但必须标为 **fleet_context_not_job_cause**，不能说“这个 Job 正在等待某台机器”。Forgejo 精确 Task result 不自动提升关联可信度。

未被 Shaula 观察到的 Job 返回原有 NotFound/retention 语义，不臆测 GitHub 的队列顺序、标签错误、调度优先级或工作流依赖。任务消失也不是成功完成。

## 7. Initial reason catalog

以下是首发应覆盖的有限语义；catalog implementation 必须把每行落到实际 producer 或 ledger predicate。没有对应证据时用 unknown，而不是为填满表格假实现。

| Code | 问题 / 证据要求 | 展示或建议边界 |
| --- | --- | --- |
| `observation.missing` | 必要来源尚未建立或历史字段缺失 | 未观察，不是资源不存在 |
| `observation.stale` | 来源超过其诊断有效期 | 保留旧值和原采集时间；不给当前阻塞结论 |
| `observation.conflict` | 来源身份/序列/内容矛盾 | unknown，不能选择有利的一份 |
| `observation.clock_invalid` | 时间无法可靠比较 | 不给负耗时或 ETA |
| `observation.unclassified_failure` | 已捕获但未分类的失败 | 不输出底层错误文本 |
| `control.auth_context_pending` | 实际上下文/交接尚未就绪分支 | 不回退为 ownership failure |
| `control.dependency_unavailable` | 依赖解析/验证失败 | 未授予详情权限时只说明 Fleet 层影响 |
| `control.ownership_unproven` | 原安全分类无法证明 ownership | 不建议跳过检查 |
| `control.listener_not_ready` | GitHub listener 当前观察 | Forgejo 上为 not_applicable |
| `control.demand_unavailable` | 需求读取失败、缺失或不受信任 | 不显示“任务为零” |
| `control.inventory_unavailable` | 所需 inventory 读取失败/不可信 | 不宣称全部 Runner 已消失 |
| `control.rate_limited` | 已分类 RateLimited | nextAttemptAt 仅在真实调度器提供时展示 |
| `control.decommissioning` | 已提交 deletion marker | 不再要求补充新 Runner |
| `capacity.target_satisfied` | 有效同组输入且 deficit=0 | 只说明无需新增，不保证 Job 立即开始 |
| `capacity.policy_ceiling` | min+demand 高于 max 的实际截断 | policy 限制，不保证增加 max 可解决其他门槛 |
| `capacity.occupancy_limit` | deficit>0 且 occupancyHeadroom=0 | 展示保留占用分类，优先检查 cleanup |
| `pool.member_at_cap` | 实际 cap 检查及正确作用域 | 可只是 informational exclusion |
| `pool.no_eligible_member` | 实际候选集合为空 | 不推断是云平台故障 |
| `pool.inline_backpressure` | 历史 inline policy 实际停止准入 | 不按 shared pool 规则重新解释 |
| `execution.create_slot_wait` | 真实 create semaphore 等待观察 | 不从 allowance>0 推断已入队 |
| `execution.destroy_slot_wait` | 真实 destroy semaphore 等待观察 | 等待不是删除失败 |
| `lifecycle.apply_outcome_unknown` | 已可能开始，缺少确定结果 | 保留原 recovery/ownership 边界 |
| `lifecycle.operation_failed` | 某次 invocation 的有类型失败结果 | operation/phase 参数有限枚举；不解析日志猜根因 |
| `lifecycle.bootstrap_pending` | 已到达实际 bootstrap 门槛 | 不把创建资源等同 Runner 上线 |
| `lifecycle.awaiting_online` | readiness 还未确认 | 不把 provider running 当成 Runner online |
| `lifecycle.readiness_timeout` | 现有控制器确已分类超时 | 诊断 30 秒 TTL 不生成此 code |
| `cleanup.job_busy` | 当前普通 busy-safe gate 返回 Busy | 等待任务结束，不能建议 force |
| `cleanup.safe_drain_unavailable` | 所属 backend 无可用安全清退证明 | 不声称硬寿命等价于无损 drain |
| `cleanup.registration_pending` | 资源 checkpoint 与远端注销未闭合 | 可以资源已删但 occupancy 仍保留 |
| `cleanup.quarantined` | 当前账本隔离与有限缺失证据分类 | 人工核验，不是自动 finalize 建议 |
| `cleanup.hard_lifetime` | 已生效策略和真实 expiry intent | 明示可能中断 Busy 任务 |
| `cleanup.no_create_effect` | 原控制器按 ARD-0040 完成分类 | 不从错误名反推无副作用 |
| `cleanup.completed` | domain terminal fact 和处置来源 | provider_cleanup / never_started / operator_attested / unknown 来源分开 |
| `rollout.no_active_revision` | 实际依赖解析分类 | 不保证排空有效 |
| `rollout.inputs_incompatible` | 原输入准入分类 | 显示允许公开的字段名/约束类别，不显示值 |
| `rollout.backend_incompatible` | 实际 backend/labels 校验分类 | 不替用户改变 backend |
| `rollout.waiting_zero_occupancy` | pin lag 且实际 occupancy 门槛未满足 | 不自动删除 Busy Runner |
| `rollout.commit_deferred` | 该 rollout 的提交竞争/门槛结果 | 只说明未提交，不声称持久失败 |
| `job.association_unverified` | 既有 association | Fleet 背景不升级为逐 Job 因果 |
| `job.association_ambiguous` | 既有 association | 不选择第一台 Runner 當真关联 |
| `job.dispatch_not_observable` | 没有足以解释上游分配的证据 | 明示观察边界，不制造排队位置 |

## 8. HTTP, authorization and compatibility

### 8.1 New read routes

| Operation | Minimum scope | Proposed client / CLI |
| --- | --- | --- |
| `GET /api/v1/fleets/{fleetKey}/diagnostics` | `fleet.read` | `fleets().diagnostics` / `shaula fleets explain <key>` |
| `GET /api/v1/generations/{id}/diagnostics` | `fleet.read` | `generations().diagnostics` / `shaula generations explain <id>` |
| `GET /api/v1/jobs/{id}/diagnostics` | `fleet.read` | `jobs().diagnostics` / `shaula jobs explain <id>` |

首发无 query-triggered refresh、include-secrets、force 或执行建议接口。未知 query 参数拒绝为 400。原 OIDC/session/Bearer 路由隔离保持；本规范不新增 credential kind 或 scope。

对象存在、但数据过期/未采集/局部投影不可用时返回 **200** 和对应 unknown/partial；无法从权威存储确定对象和授权边界时返回 **503 DiagnosticsUnavailable**。未授权按现有 401/403，已不存在/已清除的对象按既有 404/410 规则。不要通过不同的无权限错误暴露对象存在性。

所有响应（包括错误）使用 `Cache-Control: private, no-store`。不返回可用于 mutation 的 ETag / `shaula-resource-version`，不在此接口支持 304；客户端执行其他操作前仍从原 resource GET 获取其真实条件版本。observationId 不是 If-Match token。

返回体上限 256 KiB，总 reasons<=64、evidence<=128、related<=32；按稳定顺序截断并标记 partial，每个问题的主阻塞和截断标记必须保留。bounded aggregate queries 代替加载全部 Generation/Job 后在 HTTP 内计算；所有 Fleet 列表不得形成“每行每秒调用三次 explain”的 N+1 轮询。

### 8.2 Permission projection

基础 `fleet.read` 只获得该 Fleet 运行影响、允许公开的状态/计数及原有可读身份。跨 Auth/Profile/Pool 的详细事实分别要求 `auth.read` / `template.read`；日志正文仍要求 `fleet.read` + `logs.read`。

缺少详情权限时保留安全概括（如“认证依赖尚未就绪”），不暴露 credential 是否过期、隐藏 account/target、未授权 profile 数据或日志片段。未授权 evidence IDs/links/parameters 必须剔除并用公开概括替换，不能留下悬空 evidenceId；响应可以标 `detailsAvailability=requires_permission`，不返回隐藏条目数量。

读取同名但新 incarnation 的 Fleet 不能给旧 Job/Generation 补新关联。旧对象仍可显示自己的 retained evidence；其 parent Fleet 已清除时相关当前上下文不可用，而不是链接到重建后的 Fleet。

### 8.3 Suggestions are guidance, not commands

建议是有限 `suggestionId`、本地模板说明和可选 typed reference，例如 view_generations、view_generation、view_invocations、review_auth_dependency、review_template_inputs、wait_for_reconciliation、review_external_resources。

需要显示重试时间时只用可空 `nextAttemptAt`（RFC3339 UTC 毫秒），并绑定实际调度计划证据；没有计划则为 null，不计算一个假定 ETA。

不返回 shell、SQL、Terraform 命令或可由 provider 注入的外部 URL。UI 只导航到已有受权限保护的查看/编辑流程；点击前重新检查权限，真正 mutation 仍经过既有 scope、CSRF、条件写入、幂等和业务门槛。

不提供“重试整个 Create”“清空 state”“force unlock”“自动 finalize”一键修复。进入既有人工 finalize 流程时必须另行提示其外部核验要求和账本语义，不能因为这份诊断说 Quarantined 就预填 attest 为真。

### 8.4 Existing surfaces remain authoritative

现有 `/status`、conditions boolean、lastError、Jobs JSON、Change state、readyz/livez 和 mutation 回执保持不变。诊断不是 readiness probe；diagnostics unavailable 不自动令 readyz 失败。

旧 server 没有该 route 时，新 client 显示“服务端不支持诊断”，并允许查看旧 status；不能把 fallback HTML 或路由 404 当成目标资源已经删除。验证响应 Content-Type/schema 后再解析。

## 9. Example

下例是合成协议示例，不是生产观测。它描述“目标 7，有效容量 4，但 10 个资源槽全部占用；尚未进入选成员阶段”。所有数值均来自同一记录的 decision 输入；对象标识均为示例。

```json
{
  "schemaVersion": 1,
  "subject": {"kind": "fleet", "key": "linux-ci", "fleetIncarnation": "example-incarnation"},
  "generatedAt": "2026-09-25T21:00:10.000Z",
  "questions": [{
    "question": "scale_up",
    "outcome": "blocked",
    "coverage": "partial",
    "basis": {
      "kind": "recorded_decision",
      "observationId": "example-observation-42",
      "observedAt": "2026-09-25T21:00:05.000Z",
      "validUntil": "2026-09-25T21:00:35.000Z",
      "freshness": "fresh",
      "subjectRevision": "12"
    },
    "stages": [
      {"id": "authority", "evaluation": "passed", "reasonIds": []},
      {"id": "demand", "evaluation": "passed", "reasonIds": []},
      {"id": "capacity", "evaluation": "blocked", "reasonIds": ["r-capacity"]},
      {"id": "pool", "evaluation": "not_evaluated", "reasonIds": []},
      {"id": "execution_admission", "evaluation": "not_evaluated", "reasonIds": []}
    ],
    "primaryReasonId": "r-capacity",
    "reasons": [{
      "id": "r-capacity", "code": "capacity.occupancy_limit", "stage": "capacity",
      "severity": "warning", "effect": "blocking", "parameters": {},
      "evidenceIds": ["e-capacity"],
      "firstObservedAt": "2026-09-25T21:00:05.000Z",
      "lastObservedAt": "2026-09-25T21:00:05.000Z"
    }],
    "evidence": [{
      "id": "e-capacity", "kind": "derived_calculation",
      "observedAt": "2026-09-25T21:00:05.000Z", "freshness": "fresh",
      "validUntil": "2026-09-25T21:00:35.000Z", "dataRef": "capacity"
    }],
    "capacity": {
      "demandKind": "github_total_assigned_jobs", "min": "2", "max": "10",
      "demand": "5", "target": "7", "effective": "4", "occupancy": "10",
      "deficit": "3", "occupancyHeadroom": "0", "arithmeticCreateAllowance": "0",
      "actuallyAdmitted": "0"
    },
    "suggestions": [{"suggestionId": "view_generations", "reference": {"kind": "fleet_generations", "key": "linux-ci", "fleetIncarnation": "example-incarnation"}}]
  }],
  "related": [],
  "truncated": false
}
```

生产 Fleet 响应还包含适用的 rollout/cleanup 问题；此例只展开 scale_up。generatedAt 超过 validUntil 后，即使没有新写入，也必须变成 stale，并停止把 r-capacity 作为当前 primary blocker。

## 10. Web, CLI and telemetry

Web 在 Fleet/Generation/Job 详情添加“为什么”区域，先展示该问题的结论、原采集时间、时效和 primary 原因，再展开证据与建议。允许多个问题同时有不同结论。not_applicable、not_evaluated、缺权限、暂不可用和“不支持”有独立呈现，不能仅靠颜色表达。

正常轮询提案为页面可见时每 5 秒一次；后台/折叠后停止，失败指数退避至 30 秒，取消卸载请求，尊重 server rate limits。刷新只重读投影。切换 incarnation/revision 后清空错误关联的旧解释；异步返回按 subject 和观察身份去重，不能覆盖更新内容。

现有表单、待提交编辑、Change 等待和受保护日志页面不因诊断失败被清空。新 reason 的字符串不能落入通用 badge 正则后被误判成 success；使用明确 severity/outcome。

spec 0039 的 Rust client 使用同样 typed response；三个 explain 命令提供 table/json 以及只读 `--watch`。业务 blocked/unknown 的 HTTP 200 是成功取得诊断，不是命令执行失败；默认退出 0，但必须显示状态，不把它写成资源已收敛。通信/认证错误沿用 client 退出码契约。此提案不新增“等到修好”或自动执行 suggestion。

延续 [ARD-0032](../ard/0032-process-telemetry-uses-otlp-http.md) 的 OTLP/HTTP 边界，不引入 Prometheus endpoint、新的 exporter 依赖或诊断权限旁路。诊断投影不是 OTel backend 查询的代理。

首发至少增加有界投影丢弃/写入失败计数；扩展业务指标时只能使用有限 backend/question/reason-class/result 枚举，不能以 Fleet/Job/Generation ID、URL、actor、错误文本为 metric label。计数发生于 producer 的事件/状态转换，不发生于 GET，避免刷新页面改变业务指标。duration 只来自同一身份的真实开始/结束事实，未完成为 unknown，而不是零。原始 ID 只可按现有安全规则用于本地查询/trace 关联。

## 11. Implementation and acceptance

### 11.1 Ordered increments

| 批次 | 范围 | 退出条件 |
| --- | --- | --- |
| A — 语义与纯模型 | typed catalog、严格参数、现有分支 inventory、capacity 基础与不确定性模型 | 每个 code 有 producer/predicate 映射；旧判断不被另一套 evaluator 替换 |
| B — 生产者与存储 | 实际返回分支补 observation、bounded sink、独立 projection store、epoch/sequence、migration | 新旧程序在相同故障输入下业务 effect trace 一致；诊断持久化故障不改变安全事实 |
| C — HTTP 与权限 | 三个只读 route、快照读取、时效、预算、字段级权限投影 | 实际 HTTP + SQLite 验证，非仅 mock response |
| D — Web / client | 三类详情解释、source-aware 展示、条件导航；0039 可用后补 client/CLI | stale/partial/无权限/旧 server/异步乱序都有前端或 CLI 验收 |
| E — 真实场景 | 选定 GitHub / Forgejo 与真实资源故障注入 | 保留脱敏的诊断响应、domain 对照事实及故障结果，不以文档示例代替 |

建议代码 ownership：`shaula-core` 拥有纯诊断 DTO/predicate 和 domain read ports，不重新引入 exporter telemetry port；`shaula-daemon` 在真实 supervisor/operation/cascade 分支分类并组装读取；`shaula-store` 拥有可丢弃投影、CAS/有界查询与 forward migration；`shaula-http` 只授权/序列化；Web/client 只解释协议。遵守现有 crate 依赖方向和 Rust 文件规模限制，不为了诊断引入新的远程客户端。

迁移只增加派生投影，不改旧状态或凭据格式。旧记录没有证据时默认 unknown，不批量补成“已通过”。旧 server/client 继续使用旧接口；本轮不借此修复其他 wire 不一致或实现 Worker/Token。实现 PR 必须更新实施状态，而本设计 PR 不标记功能完成。

### 11.2 Required test matrix

| ID | 场景 | 必须成立 |
| --- | --- | --- |
| DX-01 | max=10、demand=5、min=2、effective=4、occupancy=10 | target=7、allowance=0；occupancy 阻塞，不是没有需求 |
| DX-02 | 缺 demand 与真实零 demand | null/unknown 与 0 明确不同 |
| DX-03 | GitHub assigned 与 Forgejo waiting/running | demandKind 与公式对应原 backend，不混合队列 |
| DX-04 | 起始输入与 GET 时计数不同、revision 变化 | recorded_decision 不拼接新旧数据；旧依据降级 |
| DX-05 | allowance>0，但真实 pool/admission 返回 None | 不声称实际创建；具体已分类 gate 可见 |
| DX-06 | shared cap、历史 inline backpressure、跨 Fleet 同 pool revision | 保留原作用域和排除规则；GET 不消耗 RNG |
| DX-07 | auth context 未就绪、listener 缺失、构造 supervisor 失败 | distinct code；没有证据不伪装 OwnershipProofFailed |
| DX-08 | 早期 gate 返回、后续 gate 未执行 | not_evaluated，不是假 passed 或不存在的第二原因 |
| DX-09 | semaphore 等待与真正已启动 operation | pending 与 started 不混同；重启使旧进程证据失效 |
| DX-10 | ApplyStarting 后响应丢失或进程状态不可知 | outcome unknown，不显示确定 running/completed |
| DX-11 | Create 成功但 Runner 不在线 | bootstrap / readiness 依据分别显示 |
| DX-12 | 普通 Busy 删除、Forgejo idle 无 fence、硬寿命 Busy 回收 | 三种语义不同；诊断没有新增删除许可 |
| DX-13 | 基础设施已删、远端注销失败 | remaining registration 和 occupancy 保留一致 |
| DX-14 | ARD-0040 终结与 operator finalize | 不宣称两者执行过 Terraform destroy；历史隔离不自动改判 |
| DX-15 | follow 输入不兼容、backend 不兼容、有占用、提交竞争 | distinct rollout code；不能都建议排空 |
| DX-16 | Job Unverified / Ambiguous、Forgejo 精确结果 | Fleet 背景不冒充具体因果；不提升关联可信度 |
| DX-17 | 同 key 新 incarnation、旧 Job、旧 worker/session、异步乱序 | 不覆盖/关联新对象；CAS 拒绝迟到 observation |
| DX-18 | 同 sequence 不同内容、时钟回退、时间在未来 | unknown/conflict，不能 last-write-wins 或负 duration |
| DX-19 | poll 失败、daemon 停止、GET 重复刷新 | 原成功时间不更新；按读取时钟自动 stale |
| DX-20 | sink 溢出、写 projection 失败、exporter 不可达 | 业务 effect trace 不变；旧快照不继续冒充 fresh |
| DX-21 | domain Store 失败 | 保持原业务失败语义；HTTP 503，不是假空数据 |
| DX-22 | fleet.read 无 auth.read/template.read/logs.read | 仅安全概括；无隐藏对象/日志/凭据泄漏或悬空引用 |
| DX-23 | 只有写权限、无效/混合 credential、撤销 session | 与既有 guard 一致，无诊断登录后门 |
| DX-24 | provider/log/输入中带 secrets、HTML、控制字符、shell 文本 | 不进入公开原因/命令；无注入，无日志文本根因解析 |
| DX-25 | 所有 suggestion 导航与 stale permission | 只读导航；mutation 重新检查原权限/版本/业务 gate |
| DX-26 | 响应/队列超限、10 万占用聚合 fixture、列表页面 | 有界截断/查询；不全表加载、不 N+1 轮询 |
| DX-27 | 旧 server、未知 code/schema、乱序 HTTP response | 不误报已删除/已收敛，不覆盖新对象视图 |
| DX-28 | reason 参数计数变化、解除再出现、采集间断 | firstObservedAt 语义正确，不虚构连续受阻时间 |
| DX-29 | diagnostic GC / 删除全部快照后恢复 | 仅丢解释，不丢 state/pin/audit，不改变资源恢复 |
| DX-30 | 实际端到端 no-create / waiting-online / destroy-failure / rollout-lag | UI/HTTP 与 domain ledger 对照；保留脱敏可复现证据 |

DX-26 的规模是测试 fixture，不是并发平台容量承诺。真实场景证据必须注明 backend/platform/version tuple；不能将一条 Docker 验收扩展成六平台保证。

## 12. Source index and design references

以下链接全部固定到被检查的 commit。只证明相应基线接口/分支，不证明本提案已实现。

| ID | 已阅读代码 | 用途 |
| --- | --- | --- |
| S01 | [capacity.rs][s01] | 目标容量、有效容量、占用和允许创建数 |
| S02 | [supervisor.rs][s02] | 实际 gate 次序、early returns、capacity decision |
| S03 | [supervisor_status.rs][s03] | 单一 reason 与默认回退 |
| S04 | [service_fleet_ops.rs][s04] | 当前 status 聚合与 boolean conditions |
| S05 | [fleet_observations.rs][s05] / [runtime_port.rs][s05b] | domain status 与 Change 的 fenced 写入 |
| S06 | [registry_impl/lifecycle_impl.rs][s06] | pool admission None 分支、inline/shared cap、真实抽样 |
| S07 | [service_follow_cascade.rs][s07] | follow lag 与输入/占用/提交分类 |
| S08 | [forgejo_supervisor.rs][s08] | Forgejo target、inventory 与资源 effect gate |
| S09 | [forgejo_supervisor_observation.rs][s09] | 成功/失败采集时效和 Jobs 非阻塞边界 |
| S10 | [jobs/mod.rs][s10] | Job/Generation 读模型与 association |
| S11 | [router/jobs.rs][s11] / [router/mod.rs][s11b] | 现有路由、日志授权、finalize 与回执 |
| S12 | [status.tsx][s12] | 字符串 badge 与通用错误展示 |

外部参考仅用于设计对照，不引入新依赖或声称采用其全部规范：Kubernetes [PodCondition](https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/) 区分 reason/status/observedGeneration/transition time；[RFC 9457 §5](https://www.rfc-editor.org/rfc/rfc9457.html#section-5) 提醒错误详情与链接的泄漏风险；[OTel Metrics SDK](https://opentelemetry.io/docs/specs/otel/metrics/sdk/#cardinality-limits) 说明属性基数边界。Shaula 保留自身 bool status 兼容性、鉴权和 ARD-0032 exporter。

[s01]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-core/src/capacity.rs
[s02]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-daemon/src/supervisor.rs
[s03]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-daemon/src/supervisor_status.rs
[s04]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-daemon/src/service_fleet_ops.rs
[s05]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-store/src/fleet_observations.rs
[s05b]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-core/src/registry/runtime_port.rs
[s06]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-store/src/registry_impl/lifecycle_impl.rs
[s07]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-daemon/src/service_follow_cascade.rs
[s08]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-daemon/src/forgejo_supervisor.rs
[s09]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-daemon/src/forgejo_supervisor_observation.rs
[s10]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-core/src/jobs/mod.rs
[s11]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-http/src/router/jobs.rs
[s11b]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/crates/shaula-http/src/router/mod.rs
[s12]: https://github.com/5aaee9/shaula/blob/881022ea9de104494788bd5f199565695acc8bfb/web/src/components/status.tsx
