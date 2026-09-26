# Production Lifecycle Worker and HTTP State Integration

- Status: Draft — implementation proposal; not a claim of completed integration
- Date: 2026-09-26
- Repository baseline: `93d607bb488d52b2e12e8537adc009fd3b1aad3d`
- Decision: [ARD-0042](../ard/0042-integrate-exec-workers-with-authoritative-http-state.md)
- Protocol owner: [spec 0010](0010-lifecycle-worker-and-http-state-backend.md)

本文定义如何把已接受的 Worker/HTTP-state 契约接入真实 `shaula serve`，以及旧部署如何安全切换。MUST、MUST NOT、SHOULD 是拟议实现要求，不是已实现或已验收声明。本文只拥有 composition、cutover 和集成验收；内部 wire/auth/lock/CAS、生命周期恢复不变量仍由 spec 0010 唯一维护。实现不得以本文省略某条协议为由放宽它。

## 1. 已核对的缺口与完成定义

以下是固定 baseline 的源码事实，不是从历史 README 推断：

| 入口 | 当前事实 | 必须补齐 |
| --- | --- | --- |
| `crates/shaula/src/main.rs` | 有 `serve`、`version`、Remote 管理命令和隐藏 helper dispatch；没有 `job` | 内部 Lifecycle Worker dispatch；保留远程 CLI 与 helper |
| `crates/shaula/src/serve.rs` | 构造 `TemplateRuntime::new`，传入 supervisor wiring | 先就绪内部服务，再构造 Executor/Worker supervision；不向生产 Generation supervisor 注入 local-state Runtime |
| `crates/shaula-template/src/runtime.rs` | `new` 是 local/static-validation 模式；`with_http_backend` 已存在 | Worker 内逐 Claim 构造 HTTP Runtime；不得复用一个跨 Generation 的 backend 配置 |
| `crates/shaula-http/src/state_backend/mod.rs` | 已有独立 loopback `StateServer`、16 个请求 permits、30 秒 timeout | 与 typed Worker control 一起接入启动、监督和 shutdown；不是合并进管理 router |
| `crates/shaula-store/src/http_state/admin.rs` | 已有新 Generation/state admission、revoke、Create 标记和 backend-only seal | 扩展共同事务入口；不能将 seal 与 terminal/容量释放分开提交 |
| `crates/shaula-template/src/runtime_backend.rs` | HTTP workspace 安装/校验路径存在，不隐式迁移旧工作目录 | 明确空 state 初始化、恢复 workspace 与 legacy import 路径 |

`note_create_starting` 不是 Fleet Create-start 握手，`seal` 不是资源清理证明。直接替换构造函数、仅增加子命令，或者把原 daemon 的每条 Terraform command 搬成一次 exec，都不构成修复。

**完成定义**：由正式打包二进制启动的 `serve`，通过唯一生产路径为新 Generation 派生同一二进制的 `job`；完整 Create/等待/Destroy 在 Worker 中顺序执行；正常 Terraform state 由内部 HTTP backend 写入 SQLite；断电/崩溃、旧 state 导入、安全门槛和管理面回归均有对应证据。历史测试和诊断投影不能替代这些证据。

## 2. 范围与不变量

本次交付仅实现本机 `exec` Executor、独立进程 Worker、内部 control/state 接线、必要的共同事务、明确迁移及部署验收。GitHub 和 Forgejo 两条已存在的生命周期入口都必须接入；具体资源平台的支持声明仍按 exact runtime tuple 单独验收。

不实现 Kubernetes/remote Executor、多主或 HA、新云平台、任意命令执行 API、新的多租户 sandbox、自动 force-unlock、无证据的 quarantine recovery，亦不解决 Forgejo busy-safe drain 的既有协议缺口。

必须保留以下所属契约，而非复制另一套状态机：

| 契约 | 集成约束 |
| --- | --- |
| [spec 0001](0001-shaula-runner-scale-set.md)、[spec 0002](0002-fleet-http-control-plane.md) | 不变的 Generation、容量占用、Fleet effect gates、Auth Handoff、Decommission；硬寿命仍从原资源创建时刻计算，重启不重置 |
| [spec 0004](0004-template-profile-runtime.md) | exact artifact/inputs/runtime tuple、locked init、saved-plan admission；Create/Destroy-only |
| [spec 0010](0010-lifecycle-worker-and-http-state-backend.md) | current Claim/epoch、分权 capability、state CAS、先 fence 再接管、原子 completion、应急证据 |
| [spec 0019](0019-workflow-jobs-and-operation-logs.md)、[spec 0020](0020-official-container-runner-bootstrap.md) | retained logs 与独立 Setup Info capability；固定宿主 bootstrap 属于同次 Create，不增加 apply |
| [spec 0026](0026-forgejo-runner-backend.md) | Forgejo 注册与删除的独立安全语义；精确 Task 结果不升级为已验证 Generation 关联 |
| [spec 0037](0037-shared-template-pool-resource.md) | shared pool 成员选择、全局成员限额及 exact selection provenance 与 Generation admission 保持原子性 |
| [spec 0039](0039-user-access-tokens-and-cli.md)、[spec 0041](0041-explainable-reconciliation-diagnostics.md) | 管理 OIDC/个人 Token、CLI/API 对等和证据驱动诊断；内部 capability 不成为管理凭据 |

## 3. Composition 与 crate 边界

预期调用关系（职责关系，不代表新增 API 已存在）：

```text
shaula serve
  +-- management listener -> existing control-plane services
  +-- private loopback listener -> WorkerControl + StateBackend
  +-- Fleet supervisors -> admission / runner-backend safety gates
  +-- ExecExecutor -> shaula job [protected inherited handoff]
                         +-- WorkerControl client
                         +-- TemplateRuntime::with_http_backend
                               +-- fenced Terraform/provider children
                               +-- fixed host bootstrap

Terraform HTTP backend -> private listener -> SQLite state/lock transactions
```

`shaula-core` 拥有小的 typed Executor/Worker-control ports、Claim 和有界 DTO；不得依赖 Axum、SQL、进程启动或平台 SDK。`shaula-daemon` 拥有 admission、安全决策、supervision 和 completion use cases。`shaula-store` 实现共同事务。`shaula-http` 仅做内部认证、decode/limits 和调用 use case。`shaula-template` 保留材料、Terraform 和固定 bootstrap 逻辑。

建议新增 `shaula-worker` 与 `shaula-executor` 两个 focused crate，前者拥有顺序生命周期与内部客户端，后者拥有本机进程监督；实际拆分须保持 [spec 0007](0007-rust-workspace-architecture.md) 的依赖方向。两个角色都由 `shaula` composition root 组装；Worker 不打开 SQLite，也不获取 daemon 的 bindings key 或 CI 管理凭据。

Executor 仅暴露 `launch`、`observe`、`stop_and_fence` 一类进程能力，不实现 GitHub/Forgejo removal 或 Terraform plan policy。`observe` 返回 `Live / Exited / Unknown`，`stop_and_fence` 返回带归属证据的 `Fenced / Unknown`；退出码不是 Destroy 结果。

`TemplateRuntime::new` 可以继续用于隔离的模板静态校验、测试和明确的旧版本维护流程，但不得出现在已切换部署的 Generation Create/Destroy/recovery 路径。GitHub 与 Forgejo 的 supervisor 都不得保留隐藏的 local-state fallback。

## 4. 启动、配置与就绪

### 4.1 有序启动

`serve` MUST 按以下依赖顺序启动；独立步骤可以并行，但不得颠倒依赖：

1. 校验 bootstrap、内部地址/限额和 exact executable；保持 mandatory OIDC 初始化失败即不开放生产能力。
2. 获取现有 data-dir ownership lock，验证路径 containment，打开 SQLite 并执行非破坏性 schema migration。
3. 检查 deployment storage-format/cutover 状态，扫描所有未终结 Generation、旧 Claim、spawn handover、lock 与应急材料。未完成迁移时进入 recovery-only，禁止 acquisition、新 claims 和外部 mutation。
4. 绑定实际 loopback socket，组装内部 control/state 服务并验证可服务；取得实际地址后才产生 launch envelope。不得下发仍未绑定的地址。
5. 完成旧执行者 fence/classification，再恢复允许的 workers；随后启动 Fleet reconciliation、diagnostics、管理 listener 和静态 validation。
6. 仅在两个 listener、store、migration gate、Executor 与 supervision 均健康时开放 mutation/admission readiness。不能沿用“scheduler 已启动就 set_ready(true)”的顺序。

内部 listener 退出必须被监督：降级 readiness、停止新 acquisition/副作用许可并进入有界停机/恢复，不静默切回本地 state。单个可解释的 quarantined Generation 不必阻塞管理 reads，但其 Occupancy 必须保留；无法分类的全局 cutover 则阻塞新 admission。

### 4.2 Bootstrap 提案

下列是拟议配置，不是当前可用配置：

```yaml
lifecycle:
  executor: exec
  internal_listen: "127.0.0.1:0"
  max_workers: 64
  recovery_reserve: 8
```

`internal_listen` 默认使用回环动态端口，由父进程的真实绑定结果交付；拒绝 wildcard、非 loopback、管理代理地址和不支持的 Driver。`max_workers`/`recovery_reserve` 在首次切换时要求显式配置，示例数值不是最终生产默认值；满足 `0 < recovery_reserve < max_workers`。恢复 worker 优先使用保留名额，新 Create 不得耗尽它们；额外排队不释放既有资源占用。

总 Worker 数与活跃 Create/Destroy budgets 独立；等待 Runner 数小时不得占 Terraform mutation permit。沿用既有 create/destroy 配置和跨 Fleet 公平性，permit 不通过长期 SQL transaction 持有。Worker 使用有界线程/runtime 资源，不能为每个等待者无条件建立一套按全机 CPU 数扩展的 Tokio pool。

State/Lock body 硬上限沿用 core 的 16 MiB/16 KiB；control envelope/请求建议限 64 KiB，大 artifact/inputs 用受保护 exact store 引用，不塞入普通 control body。日志沿用 spec 0019 的独立分块限额。内部处理必须区分 control/state/log 的有限队列，日志拥塞不得饿死 state write/UNLOCK。当前 state adapter 的 30 秒超时与 store writer 等待预算必须一起审查，不能让正常写锁排队稳定地触发假超时；丢响应仍按 uncertain/replay 处理。

最终 request/retry/shutdown/queue/concurrency 默认值、硬上限和负载证据归 [决策清单 D3](../README.md#仍需决定或冻结)，不能借示例数值宣称已冻结。发布门槛要求这些值在配置/schema/部署文档中一致并验收。

## 5. Admission、启动材料与 Worker 控制

### 5.1 原子 admission 与初始 state

新增 store use case 必须在同一短 writer transaction 中提交：Generation、原始 Fleet/Profile/Pool 选择和容量占用、storage mode、Worker Claim/attempt/epoch、启动事实、两种 capability verifier 及 authoritative 初始 state。先检查的 Fleet Revision、删除标记和 pool member 限额必须在提交时仍有效；不能分别调用一个已提交的 pool admission 和现有 `insert_generation`，造成部分成功或重新随机选模板。

生产新 Generation 在 Worker 获取材料前 MUST 已有可信空 state。不能假设 `terraform init` 一定会 POST 空 state。新增受控 initial-state builder 生成受支持 Terraform 格式的空 state（新 lineage、serial 0、无资源），在同一 admission transaction 写入 backend revision；格式/engine tuple 由真实 pinned Terraform 验收。它不是一般 state-push API，不能用于旧 Generation、可能已 Create 的记录或被删除的 state。这样 `note_create_starting` 的“已有 snapshot”前置条件不依赖偶然的 Terraform 持久化时机。

spec 0010 对尚未初始化的已准入 backend 的 404 仍是适配器协议；上述生产 admission 不暴露这段窗口。state 缺失不能使旧 Generation 回到 fresh 状态。

### 5.2 进程启动与分权

先持久化 `launch_pending`，再启动同一 exact Shaula binary 的内部 `job`。Worker 在登记进程归属、protocol version 和 Claim handshake 被 daemon 确认前不得 materialize 以外的外部副作用。即使 exec 成功但响应丢失，也必须按 pending launch 分类，不得直接再派一个 worker。

通过 inherited pipe/handle 一次性交付 Generation/Claim、协议版本、内部实际地址、材料引用、runtime tuple 与分权 capabilities。argv 只包含非机密标识；不能用任意模板路径、Shell string 或全量 daemon env 构造任务。Worker 检查 envelope 大小、来源、版本、identity 和材料摘要；失败即退出且无 JIT/注册/apply。

除明确授权的 handoff 外，关闭非必要继承 FD/handle；控制凭据不可继承给 Terraform、bootstrap CLI 或 Runner。Terraform 只获得该 Claim 的 state capability 和已批准的 provider 环境；`TF_HTTP_PASSWORD` 是 spec 0010 规定的专用例外，不能放入 HCL、backend-config、saved plan、日志或公共环境转储。两种 capability 不能互换，个人管理 Token 也不能调用内部接口。

### 5.3 控制接口与副作用 gate

以下是需要实现的 typed use cases，不新建一套 wire 协议；route/DTO 在 spec 0010 的版本化内部接口范围内落地：

| Use case | 必须验证与保留 |
| --- | --- |
| Claim handshake / renewal / observe | current Generation incarnation、epoch、attempt、protocol version；只续期同一 Claim；observation 是 level-triggered |
| 准备 CI bootstrap material | daemon 执行 GitHub JIT 或 Forgejo 注册；durable intent/result、exact backend/auth refs 和现有 uncertain 分类；不下发管理 credential |
| 请求 Create-start | exact plan/input/runtime digest、当前 Fleet gates、可信空 state、command budget；一个不可重复的 Create 事实 |
| spawn acknowledgement / command-ended | 匹配已准入的 handover/执行身份；丢响应不推断为未启动；只保留必要事实，不持久化逐条 init/inspect 命令 |
| 请求 retirement / safe removal | daemon 按原 backend gate 判断 Busy、ownership、hard-lifetime 例外和删除结果；worker 不自判安全 |
| completion / receipt replay | 见 §7；state seal、terminal fact、容量释放为同一事务 |
| operation-log delivery | 只写本 Claim 的合法 operation attempt；沿用 spec 0019 的独立权限/限制，不复用 state route |

所有 mutating use case 使用稳定 request identity；同 ID 同内容重放返回原结果，同 ID 不同内容拒绝。令牌、lock ID 和 Worker 提供的 boolean 都不是独立的副作用证明。

Create-start 必须把现有 Fleet effect gate 与 backend `create_started` 事实结合：在允许可能 spawn 前持久化，并在 handover 未解决时禁止冲突 DELETE/retirement 越过。事务不跨 IPC/网络/进程等待；长等待由 use-case gate 协调。只有确认已经 spawn，或 Executor 证明旧任务已经不能再 spawn，才能解决 handover。超时、撤销 token 或丢失连接都不足以证明不会迟到 spawn。DELETE commit 之后不得出现此前悬而未决的 Create spawn。

## 6. Worker 生命周期与后端兼容

Worker 按 spec 0010 的完整顺序执行 materialize、locked init、bootstrap material、saved Create plan、start gate、一次 apply、固定 bootstrap、等待 retirement、安全删除 gate、saved delete-only plan、Destroy、completion 和本地 cleanup。daemon 只下达目标/安全许可，不逐条派发 init/plan/show/apply 命令。

新 Worker 恢复旧 Generation 时必须使用保留的 immutable inputs/artifact/runtime tuple，不重新选择最新 TemplatePool member；不重新 mint 已不确定的注册材料；Create 已可能启动时只能观察或 cleanup-only。原 pinned engine/trust tuple 不兼容新 runtime 时不伪造兼容性，按 §9 清退或隔离。

固定宿主 bootstrap、Setup Info 安全投影及启动门槛在 Worker 中运行，仍受 same-Create ownership/fence 约束。GitHub App credentials、Forgejo 管理 token 和 OIDC material 留在 daemon；唯一 Runner 所需的 JIT/注册材料按原平台协议交付。

普通 Busy removal 返回时 Destroy 调用次数必须为零。已记录的硬寿命到期是现有策略例外，Worker 拆分不得新增例外或重置计时；missing Scale Set、未知所有权、Forgejo ambiguous registration/drain 继续阻塞或隔离。不得把 child exit、Runner 容器退出或短暂 idle 观察作为删除许可。

## 7. 完成提交、日志与可观测性

新增共同 completion transaction 必须校验 current Claim、无未解决 handover/活跃 Terraform command/lock、daemon-owned removal/终态分类证据、exact backend revision 及可信 empty state。一次提交同时 seal state、写不可变 completion receipt、置 Generation terminal 并释放 Fleet/Pool Occupancy。现有 backend-only `seal()` 必须抽为可参与此事务的内部原语，不能先 seal 后独立更新 Generation。

区分两种完成：正常 provider cleanup 使用 Destroy 结果和 exact empty-state proof；从未可能 Create 的 Generation 沿 [ARD-0040](../ard/0040-destroy-generations-that-never-started-a-create-apply.md) 证明无资源/无未解决 CI 副作用后终结，不伪造一次成功 Destroy，也不能由于 backend 尚未 `create_started` 而永久无法释放。已有 operator attestation 终结语义不得被自动 cleanup 冒充。

Worker 只有拿到 durable receipt acknowledgement 后才能清理普通 workspace；lost response 可使用已绑定同一 Claim 的有界 completion replay 权限取回原 receipt，不能重开 state 写入。worker crash 于 ack 后、cleanup 前时由受控 reaper 幂等清理；未上传 state、input/artifact 或未知路径绝不能被普通 reaper 删除。

日志、Jobs、Setup Info 和 diagnostics 应保持现有 IDs/权限/retention；新 Worker attempt 与原 Operation attempt 不得混为一谈。新增 Worker/Backend 原因仅进入 spec 0041 的统一 catalog 与源事实生产点，不在 HTTP UI 重建第二套判断逻辑。诊断写入失败不改变 admission/removal/完成结果；不得靠投影表恢复 Worker authority。

安全状态摘要至少可区分 migration-required、launch unresolved、worker fencing unknown、state unavailable、cleanup-only 和 terminal receipt replay；原始 state、lock metadata、Authorization、query ID、inputs 与应急文件均不进入普通管理 reads、traces 或 metrics labels。

## 8. 进程 fence、恢复与 shutdown

v1 restart 策略明确选择 **先 fence 旧 Worker，再分类并替换**，不在此版本增加任意存活 Worker 的网络重附着。state lock、capability 撤销、heartbeat timeout 或 daemon ownership lock 都不是 provider fence。

ExecExecutor 必须在允许执行前建立 Worker 及所有 Terraform/provider/宿主 bootstrap descendants 的可验证归属，记录适用的 host/boot identity、进程启动 identity、guardian/containment identity，而不是仅记录 PID。复用现有 `engine_supervisor` 的防护，同时验收新增父子层级；不得假设杀死 Worker 就一定终止全部孙进程。

Windows 的 Job Object、Unix 的 guardian/process group 等当前机制需要各自的 nested-worker/crash 证据；仅 process group leader 退出、PID 不存在或发送 kill 成功不足以产生 FenceReceipt。实现选择更强 OS containment 时必须在部署要求中明确。未实现或不能验证的宿主能力 fail closed；恶意/高权限 IaC 逃逸仍属于既有 trusted-host 风险，不因独立进程化宣称已隔离。

| 故障点 | 恢复要求 |
| --- | --- |
| admission committed，spawn 未知 | 检查 launch facts/guardian；不能因内存 child list 为空再派发 |
| Create-start 响应丢失或可能已经 apply | 保留 handover 与占用；旧树 fence 后 cleanup-only，不再 Create apply |
| Worker 退出而 provider 仍活着 | 不换 epoch、不抢锁、不释放 command budget/占用，直到可信 fence |
| HTTP backend 中断、Terraform 有外部效果 | 保留 Workspace、`errored.tfstate` 等 emergency bytes，禁止 fallback、cleanup 和盲目 re-apply |
| DB state 与 emergency evidence 不同 | 隔离；不以 serial 较大自动当完整，也不以清理已知子集后的空 state 消除未知资源 |
| 完整可信 DB state/inputs/artifact 在，普通 workspace 丢失 | 验证旧执行者已 fence 后重建独占副本，使用 HTTP backend cleanup-only |
| completion committed，响应/Worker 丢失 | 重放原 receipt；保持 sealed，幂等回收普通 workspace |

新 epoch 的签发、旧 capability 撤销和 orphan lock 处理在可信 FenceReceipt 后原子完成。旧进程仅在网络层失权不等于其 provider 已停止。已发给云服务的异步请求也可能晚到；fence 不能代替资源/state 完整性分类。恢复不能证明全部副作用已被覆盖时仍保留 Occupancy。

Graceful shutdown 先停止管理 mutation、新 claims/acquisition 和新 effect permits；随后通知 Worker quiesce，保留内部 state/control、日志接收和 DB，允许已运行 command 在有界期限内完成 state write、UNLOCK 和 receipt。超时后执行 fence 并保留不确定材料；在确认请求/进程终止或已持久化隔离之后才关闭内部 listener、DB，最后释放 ownership lock。不能先 revoke 活跃 state credential 再要求 Terraform 上传最终 state。普通服务停机不触发全 Fleet Destroy。

## 9. Legacy local-state 迁移与回滚

### 9.1 存储格式与维护入口

新增持久化 `storage_mode` 和 deployment cutover marker；缺失字段的历史记录必须映射为 legacy/needs-classification，不能默认变成 HTTP fresh。schema migration 不删除 operation ledger、历史 state、attestation/activation refs 或 receipt。数据库表结构升级与外部资源/state 迁移是两个阶段。

提供显式的离线维护 use case（拟议 CLI 名称：`shaula maintenance lifecycle-state inspect|apply`）。它不属于 remote client，不暴露 HTTP state-push/force-unlock，不启动 acquisition、JIT、注册或 Terraform mutation。必须独占同一 data-dir lock，验证旧进程树已静止；拥有本机受保护数据访问权限是其信任边界，不绕过日常管理 OIDC。

`inspect` 生成有限、无秘密的迁移计划：baseline database identity/版本、exact Generation、源 state/input/artifact commitments、资源副作用分类、预期目标 mode。`apply` 必须重读并校验这些 commitments，不接受任意路径或 stale plan；两阶段之间任何变化使计划失效。

### 9.2 分类与导入

1. 停止新 claims/acquisition，停止旧 daemon 并证明 descendants 静止；既有 Runner 可继续工作，不以迁移为由删除 Busy 资源。保留一致性备份和数据锁。
2. 对每个非终结 Generation 检查 exact ownership/inputs/artifact/runtime tuple，以及主 state、backup 和 emergency 文件。不得任意选“最新文件”、合并 lineage 或伪造空 state。
3. 完整可信的 local state 按原始 bytes/lineage/serial 导入；同一事务写 state、storage-mode、迁移 receipt、旧执行权失效与 cleanup-only 约束。新 backend revision 独立于 Terraform serial，不改写 inputs/selection provenance。
4. 无 Create 可能性且所有 CI 副作用已解决的 Generation 可以走 never-started completion。state 丢失、损坏、覆盖不完整、来源不明或 runtime 不兼容则记录隔离，不创建可返回 fresh 404 的 backend。
5. Worker 重建新的独占 HTTP workspace，不在旧 workspace 上做隐式 `init -migrate-state`，不复用旧 saved plan；保留旧目录证据直至受控 retention。需要 cleanup 时从固定材料和已导入 state 重新生成 delete-only plan。
6. 迁移过程逐 Generation 原子、整体可重入；相同 commitment 重试返回原 receipt，不重复导入。crash 后只有已完整提交的记录为 HTTP。全部 legacy 记录被导入、可信终结或显式隔离后才允许写 deployment activation marker；任何未分类记录阻止新 admission。隔离记录继续占用容量。

不能安全导入的旧 runtime tuple 应在未激活新执行模式前由原受控版本清退，或保留隔离；不得在新 daemon 内长期保留自动 legacy mutation fallback。迁移会延迟 cleanup，须在维护窗口中明确说明并保留原硬寿命时间戳；不承诺停机期间仍能按时回收。

### 9.3 回滚与备份边界

激活 HTTP execution 之前，只有证明 checkpoint 后无新外部效果、所有旧/新执行者已 fence，且完整一致性集合可恢复时，才允许恢复原数据目录和旧二进制。激活后禁止把同一 Generation 切回 local state、用旧快照覆盖新 state，或只降级二进制；采用 forward-fix/受控恢复。

新启动器/NixOS 部署应校验 storage-format compatibility 并拒绝不支持的包回滚。但**不能假定历史二进制理解新 marker**；直接手工执行旧二进制访问已迁移目录属于不支持操作，runbook 必须明确禁止，不能把 marker 宣称为对所有旧程序的强制防护。

备份一致性集合沿用 spec 0010：SQLite 的一致快照、immutable artifacts、retained protected inputs/所需 key，以及所有未解决 emergency state。工作副本/cache 可重建，但不是全部 workspace 都可丢弃。恢复必须结合 checkpoint 之后的外部效果与旧执行者 fence 分类；任意旧备份不等于当前资源真相。所有 state/WAL/备份按 credential-grade 管理，不宣称新增了静态加密。

## 10. 实施分解与退出条件

| 阶段 | 实施内容 | 退出条件 |
| --- | --- | --- |
| P1 | core ports/typed control、共同 admission/initial-state/completion/migration 事务、schema | SQLite 并发、crash、replay、never-started、pool 原子性测试通过；不改默认执行路径 |
| P2 | `job` dispatch、exec supervision、protected handoff、Worker 顺序运行和日志 adapter | 真实子进程 fake-backend 测试证明无 daemon 逐命令调度，无重复 Create，跨 Claim 拒绝 |
| P3 | private listener/startup/readiness、GitHub/Forgejo 两条 wiring、graceful shutdown | 正式 `serve` subprocess 使用真实 pinned Terraform/SQLite HTTP backend，覆盖全部 §11 本地集成项 |
| P4 | inspect/apply migration、recovery-only、旧 tuple 处理、backup/rollback guard、NixOS 配置 | legacy fixture、逐事务中断、state 丢失/分歧、格式不兼容均 fail closed |
| P5 | default cutover、真实后端证据、运维文档与回归 | 完整验收矩阵与 D3/R1/R3 配置冻结；`IMPLEMENTATION_STATUS.md` 按精确范围更新 |

可分 PR 开发，但不得在 P1/P2 adapter 测试通过后把生产路径切为默认。切换发布必须移除所有生产 Generation 的 local fallback；历史静态验证/迁移用途保留时要有负向 composition 测试。相关部署、jobs/logs、OIDC、Runner-lifetime 文档按实现变化更新，而不是在本 Draft 中提前宣布可用。

## 11. 验收矩阵

以下均为**待实现/待执行**的要求。本文件没有新增运行时代码或测试结果。每项证据要绑定 commit/binary、OS/fence、Terraform/provider lock、Runner Backend、Template Platform tuple；不把 mock、本地真实 Terraform 和真实平台验收混为一项。

| ID | 场景 | 必须断言 |
| --- | --- | --- |
| LW-01 | 正式 `serve` 产生正式 `job` | exact executable、受保护 handoff、独立进程完整生命周期；生产路径无 local Runtime |
| LW-02 | 手工 `job`、缺/错 envelope、协议不兼容 | 零 JIT/注册、零 apply、无任意文件/命令执行 |
| LW-03 | 内部 bind/服务退出、store 不可用 | readiness 降级，零新 claims/副作用许可；绝不 local fallback |
| LW-04 | GitHub/Forgejo × single/shared pool 并发 admission | 原始选择/占用/Claim/state 同事务；同 Generation 唯一；失败无部分 reservation |
| LW-05 | 新 Generation 空 state 初始化 | pinned Terraform init/plan/apply 成功，Create gate 不依赖 init 的偶然 POST；空 state 不能用于旧 Generation |
| LW-06 | 真实 Terraform HTTP 协议 | 原始 JSON、POST `?ID=`、LOCK/UNLOCK、无自定义 If-Match；继承 spec 0010 的全部 wire/CAS tests |
| LW-07 | 写入/解锁/换锁/换 epoch 并发和丢响应 | 事务校验、不接受无锁/旧 Claim 写；相同内容重放不重复推进 revision；迟到 UNLOCK 不删新锁 |
| LW-08 | lineage/serial、body limit、state 缺失/损坏 | conflict/拒绝无改写；既有 state 缺失不返回 fresh 404；HTTP/store 双层限额 |
| LW-09 | 管理/OIDC/个人 Token/control/state 交叉认证 | 两 listener 和两 capability 相互隔离；跨 Generation/epoch 拒绝；认证先于 body |
| LW-10 | secrets/FD/env/log/Runner 检查 | control/key/管理凭据不进入 IaC/Runner；state 密码只在允许通道；内部 capabilities 不进入日志、plan 或 HCL；已有受保护 provider/Runner 材料仍按原 credential-grade 契约处理 |
| LW-11 | exec 前后 crash，spawn 响应丢失 | launch_pending 有证据分类；无并行 replacement；占用保留 |
| LW-12 | Create-start 与 Fleet DELETE/Auth Handoff 并发 | 未解决 handover 阻挡冲突提交；DELETE commit 后零迟到 Create spawn |
| LW-13 | Create 可能启动后 Worker/daemon crash | 先 fence，cleanup-only，Create apply 总次数不增加；注册不盲重放 |
| LW-14 | 孙进程存活、PID reuse、guardian 死亡、daemon SIGKILL | 无伪造 FenceReceipt/超时抢锁；按宿主 tuple 验证或明确隔离 |
| LW-15 | backend outage 与 emergency state | 不清 workspace、不 re-apply；DB/应急分歧不自动择一；未知副作用未被空 state 掩盖 |
| LW-16 | 普通 workspace 丢失而 DB/materials 完整 | fence 后重建 HTTP workspace，exact tuple cleanup-only，无再选择/再 Create |
| LW-17 | Busy、unknown ownership、missing Scale Set | 普通 Destroy 次数为零；原始安全原因保留 |
| LW-18 | 原硬寿命到期、重启和迁移 | 使用原创建时间/原例外，仍经过 ownership/fence，不重置为新 Worker 启动时间 |
| LW-19 | Forgejo ambiguous register/drain、Task 完成 | 不推断安全删除/已验证关联，不把拆进程宣称为 drain 修复 |
| LW-20 | 同次 Create 的 container bootstrap/Setup Info | exact gates、retained log、启动门槛保持；无第二次 apply、capability 不混用 |
| LW-21 | Destroy 失败与重试 | 上一树已结束/fenced、原材料和完整 state、新 delete-only saved plan；占用不提前释放 |
| LW-22 | seal/terminal/容量释放各提交点 fault injection | 只有共同事务全成或全不成；零“已释放但可写 state”窗口 |
| LW-23 | never-started 与 operator-attested 历史终结 | 分类准确、不伪造 Destroy，不因 `create_started=false` 卡住合法终结 |
| LW-24 | completion 响应丢失、ack 后 cleanup crash | 原 receipt 幂等重放；sealed 永不重开；reaper 不删除未知/应急材料 |
| LW-25 | legacy 非空 state、损坏/缺失、旧 tuple 不兼容 | exact 导入或隔离；不能 fresh 404/造空 state；保留 provenance、占用和证据 |
| LW-26 | inspect/apply 间文件变化、逐 Generation 导入 crash | stale plan 拒绝；receipt/mode/state 原子；可重入且无隐式 Terraform mutation |
| LW-27 | 迁移激活、旧配置、二进制/包回滚 | 未分类 legacy 阻挡新 admission；激活后不支持旧目录语义；无 dual-writer |
| LW-28 | HTTP-state backup/restore 与外部晚到效果 | 一致性集合完整、旧执行者先 fence；不以旧 DB snapshot 覆盖新证据 |
| LW-29 | graceful shutdown、截止时间、最后 POST/UNLOCK | backend/DB 最后关闭；超时保留 uncertainty；普通 stop 不全量 Destroy |
| LW-30 | Worker 上限、recovery reserve、日志/SQLite 负载 | 等待不占 mutation permit；恢复不饥饿；有界 RSS/线程/队列；state 不被日志饿死 |
| LW-31 | 管理 API/CLI/UI、Jobs/log retention、diagnostics 失效 | 权限/ID/行为不退化；诊断开关/失败不改变实际 effect trace |
| LW-32 | 真实环境端到端 | 至少 GitHub 与 Forgejo 各一组完整生命周期，并覆盖 Docker/Kubernetes；每个宣称支持的平台另有相同路径 smoke/evidence；未覆盖组合不宣称通过 |

LW-06–LW-08 需要真实 pinned Terraform + SQLite + HTTP，不能仅手写 HTTP 请求；LW-01/03/11–16/29 需要真实 `serve`/`job` 子进程与 fault injection，不能只实例化 Runtime adapter。LW-32 中 Forgejo 正常完成清理或硬寿命例外须分开记录，不构造不存在的 busy-safe 保证。

实现 PR 还必须运行仓库要求的 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo nextest run --manifest-path Cargo.toml --workspace test`；另运行不带名字过滤的 workspace suite，避免把尾部 `test` 过滤结果当全量验收。涉及 UI/client 时运行相应 Web/CLI 回归，正式 Nix 打包与 NixOS service shutdown 亦须覆盖。

## 12. 来源与记录边界

本提案的 baseline 源码依据是 §1 的文件，以及 [core state contract](../../crates/shaula-core/src/state_backend/mod.rs)、[engine supervisor](../../crates/shaula-template/src/engine_supervisor.rs)、[实现状态](../IMPLEMENTATION_STATUS.md)。Git 历史中的固定 baseline 是复核依据；工作树链接随实现演进。

外部协议以 HashiCorp 的 [HTTP backend](https://developer.hashicorp.com/terraform/language/backend/http) 和 [state locking](https://developer.hashicorp.com/terraform/language/state/locking) 文档为参考，检索日期 2026-09-26；它们不替代仓库固定 Terraform 版本的运行验证。

仍需冻结的跨文档运行策略/发布 tuple 只见 [docs 决策清单](../README.md#仍需决定或冻结)。本文和 ARD 的合并不改变 `IMPLEMENTATION_STATUS.md` 的未集成结论；实现与迁移证据必须在后续实现 PR 中更新。
