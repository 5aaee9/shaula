---
status: proposed
date: 2026-09-26
relates_to: ["0014", "0024", "0040"]
---

# Integrate exec workers with authoritative HTTP state

- Specification: [spec 0042](../specs/0042-production-lifecycle-worker-integration.md)
- Existing protocol/architecture: [spec 0010](../specs/0010-lifecycle-worker-and-http-state-backend.md) / [ADR-0014](0014-run-lifecycle-workers-with-a-database-http-state-backend.md)
- Repository baseline: `93d607bb488d52b2e12e8537adc009fd3b1aad3d`

本 ARD 提议已接受架构的生产接线和迁移选择，不重新接受一次 Worker/state 架构，不取代 spec 0010 的唯一协议所有权，也不宣称任何尚未执行的验收已通过。状态保持 `proposed`，由维护者审阅决定。

## Context

当前项目已经有不少关键构件：独立回环 `StateServer`、SQLite state/lock 事务、分 Generation 的 state capability、显式 HTTP Runtime、saved-plan 检查，以及现有 Terraform descendants 的 fence supervisor。但 `serve.rs` 仍把 `TemplateRuntime::new` 注入 Generation supervisor，`main.rs` 没有 `job`。最新 CLI 已包含远程管理命令，因此“公开 CLI 只有 serve/version”不是当前基线事实。

问题不是缺少一个 `backend "http" {}` 文件，而是已有组件之间没有形成同一生产所有权链：谁原子创建 Generation/state/claim，谁授权可能的 Create，谁证明旧 provider 停止，谁最终确认资源已清理并释放容量，尚不能用单个 adapter 的测试回答。

源码还明确留出了两处边界：`note_create_starting` 只是 backend 的 missing-state fence，不是 Fleet 的 Create-start 握手；`seal` 只封存 backend，不验证所有资源清理证据、不释放 Occupancy。把这些已提交的独立方法依次调用，会留下跨事务中间状态。HTTP Runtime 的 `prepare_create` 也不能被当作“Terraform init 必然建立 authoritative 空 state”的证明。

此外，已有部署持有本地 Terraform state、immutable inputs、历史执行/所有权记录。任何只面向 fresh install 的接线都会在升级时留下危险的双重 authority：旧 workspace 看起来存在资源，新 HTTP backend 却可能表现为 fresh 404。

源事实见 [main](../../crates/shaula/src/main.rs)、[serve](../../crates/shaula/src/serve.rs)、[Runtime](../../crates/shaula-template/src/runtime.rs)、[state admin](../../crates/shaula-store/src/http_state/admin.rs)、[HTTP server](../../crates/shaula-http/src/state_backend/mod.rs)。固定 baseline 用于复核，不把路径后续变化误作当时事实。

## Decision

### 1. 交付一条完整路径，而不是可选构造函数

提议在通过迁移和发布 gate 后，生产新 Generation 只使用 `serve -> ExecExecutor -> shaula job -> HTTP Runtime -> SQLite state`。保留 local Runtime 供静态 validation、测试和明确的历史维护，不保留自动 fallback，也不让 GitHub 和 Forgejo 各自长期维护一种 execution mode。

第一版只做本地 `exec`。一个 Worker 运行整个 Generation 的顺序生命周期；它不是 Terraform command RPC server。daemon 继续拥有容量、Fleet/session、CI 管理凭据、安全删除和 terminal authority。Worker 不直接打开数据库。

这样既兑现 ADR-0014 的原始分工，也使故障恢复只需围绕 Claim、可能的副作用和 state 事实工作，不再创造新的逐命令 durable workflow engine。

### 2. 复用领域规则，补共同事务

通过 store 内可组合的事务原语实现 admission、Create-start facts、terminal completion、epoch replacement 和 migration，而不是在 HTTP handler 里编排多个已经 commit 的方法。

Generation、shared-pool reservation/选择来源、Claim、初始 state 和 capability verifier 一次准入；完成时 seal、receipt、terminal 和占用释放一次提交。Pool 随机选择在原 admission 位置发生，Worker 不再次选择，也不因为 HTTP-state admission 重试而改变结果。

对正常 provider cleanup 与 [ARD-0040](0040-destroy-generations-that-never-started-a-create-apply.md) 的 never-started 终结分别验证。后者不能被新 backend 的 `create_started` 前置条件卡住，也不能伪装为运行过成功的 Terraform Destroy。

### 3. 新 Generation 在 handoff 前建立空 state

选择受控 initial-state builder 在新 admission transaction 中初始化 authoritative 空 state，而不是依赖引擎 init 的偶然写入，也不让 Worker 用无权限限制的 state-push 自行补洞。

只对确定新建、无资源的 Generation 使用受支持格式、新 lineage 和 serial 0；固定 Terraform tuple 的真实测试决定该格式是否兼容。既有/已可能 apply/丢失 state 的 Generation 没有此能力。该选择不修改 spec 0010 的 HTTP 方法、lock 或未初始化 GET 语义，而是使生产路径在 worker handoff 前跨过初始化窗口。

代价是需要一个很小但必须严测的格式兼容点；收益是 Create-start 的安全前置条件明确，不会直到第一次真实 apply 才发现 snapshot 不存在。

### 4. 内部 listener 与 Worker 一起接受监督

内部 control/state 共用受保护的回环服务边界，但保持能力分权和有限队列；管理面仍使用现有 OIDC/个人 Token 模型，不增加匿名例外。Worker 的 control token 不进入 Terraform，Terraform 的 state token 不成为 control 或 Runner credential。

内部服务先 bind 并准备处理请求，再发 launch envelope；其故障使 admission 停止，而不是切回 local state。停机次序反向依赖：先停新副作用，最后才关 state backend/DB。请求超时、SQLite 排队和 engine retry 需要作为一组预算验证，不能机械沿用 adapter 的当前常量。

动态回环端口通过受保护父子 handoff 下发，不持久化为下一进程必须重用的地址。新 Claim 的 runtime 配置属于该 Claim，不能把 A Generation 的 state 配置复用于 B。

### 5. 选择先 fence 后替换，不新增存活 Worker 重附着协议

v1 daemon restart 先处理旧 Worker/descendants 的可验证终止，再签发新 epoch。当前 guardian/process tree 机制是可复用基础，但新增的多层进程关系仍要分别验收；PID、超时、撤销凭据和 state lock 都不能代替 FenceReceipt。

Create 只要可能开始，就不再在原 Generation 重做 Create apply；replacement 只能观察或 cleanup-only。对未上传 state、异步云端效果或未覆盖的资源保持 uncertainty。Workspace 只有在它不再承载唯一恢复证据时才可重建或清理。

不选择任意重附着，是为了缩小首版认证、session 连续性与旧进程归属的组合空间。代价是服务重启可能中断正在进行的 Terraform 操作并增加隔离/恢复工作；不能因此承诺无损重启，也不等于主动停止已有 Runner 工作。

### 6. 显式离线导入，单向激活

选择 `inspect -> validate -> apply -> activate` 的离线维护流程，复用 data-dir lock，禁止 migration 自身执行 CI/基础设施 mutation。schema 升级不自动将 legacy 标为 HTTP；迁移 receipt 记录 exact inputs/state/ownership commitment，单 Generation 导入和 mode 切换原子且可重入。

既有可信 state 原样导入；不可信记录显式隔离并保留占用。旧 immutable runtime 不兼容新 worker 的，在激活前由原受控运行时清退或隔离，不静默“升级”材料。全部记录已分类后才激活新 admission。

激活后不提供 per-Generation 切回 local 的开关，不允许旧数据库快照覆盖新 state。维护者需要 forward-fix 或受控恢复。激活前的 rollback 也要求完整一致性备份、无 checkpoint 后外部效果和可信 fence，不能只切换包版本。

storage-format marker 让支持它的新 launcher/NixOS 集成拒绝不兼容部署；它不能约束不认识 marker 的历史二进制。因此直接让旧二进制接管已迁移目录必须在 runbook 中明确列为不支持操作，而不是宣称靠一个 DB 字段就彻底防止降级。

### 7. 以真实 composition 和故障证据作为完成门槛

P1/P2 可以先合入原语和 Worker，不提前切换默认生产路径。切换必须等待 [spec 0042 §11](../specs/0042-production-lifecycle-worker-integration.md#11-验收矩阵) 的真实子进程、Terraform HTTP、migration/crash、backend safety 和部署矩阵。

验收同时覆盖 GitHub 和 Forgejo lifecycle 入口、single/shared-pool admission、现有日志/Setup Info/诊断及远程 CLI。某个 Docker 场景通过不代表六种平台通过；Forgejo 硬寿命清理通过不代表普通 busy-safe drain 已解决。

## Alternatives considered

| 方案 | 取舍与结论 |
| --- | --- |
| 仅在 `serve` 调用 `with_http_backend` | 可以验证适配器，但仍由 daemon 运行完整 Terraform 操作，缺 Worker supervision/控制面分权/迁移，不能作为最终修复 |
| `job` 每次只代理一条 Terraform 命令 | 形成 daemon-owned durable command scheduler，重复原复杂度；拒绝，Worker 必须拥有顺序生命周期 |
| HTTP-state 错误时自动 local fallback | 一个 Generation 形成两个 authority，可能重复 Create 或丢失销毁依据；拒绝 |
| 长期双执行模式，旧 Generation 自动留在 daemon | 渐进部署方便，但扩大互斥/预算/恢复/安全 gate 的测试矩阵；不作为本次目标，允许激活前原版本清退，不允许静默降级 |
| 在线无停机自动 import / `terraform init -migrate-state` | 难以证明所有旧 descendants 和写者已静止，且新旧材料/state 权威切换复杂；首版选择离线受控导入 |
| 从最大 serial 的本地文件自动恢复 | serial 不证明该文件来源、完整性或所有异步副作用已覆盖；拒绝自动采信，保持隔离 |
| heartbeat TTL 自动接管、自动 force-unlock | 只能说明消息未到，不证明 provider 不能继续变更资源；拒绝 |
| 把原始 CI 管理 credential 下发 Worker | 省去控制调用但扩大受信面，且可能扩散到 IaC/Runner；拒绝 |
| 立即实现 Kubernetes Job/remote Executor 或 HA | 引入远程网络、材料交付、分布式归属和 fence，未解决当前 exec 接线反而扩大范围；推迟 |
| 重写所有现有 lifecycle 状态和丢弃历史 ledger | 破坏迁移/诊断/精确来源；拒绝。保留事实，调整执行所有权，而非清空历史 |

## Consequences

### 正向影响

一条生产执行路径可以直接验收，进程边界与职责边界对齐。SQLite state 和 lifecycle facts 能在关键点共同提交；普通 materialized workspace 可重建，恢复不再仅依赖一个可丢失的本地 state 文件。能力分权、严格 handshake 和既有 Runner safety gates 能形成完整的授权链。

### 成本与限制

需要新增 Worker/Executor 的进程、内部协议、限额、共同事务和维护入口，不能以低风险的一行配置变更上线。一个 Generation 的等待进程可能持续数小时，必须明确总进程、线程/RSS、recovery reserve 与 Terraform mutation budgets；总 Worker 数不等于允许同时 apply 的数量。

HTTP state 增加对 daemon/SQLite 可用性的依赖。backend 中断可能留下唯一 emergency state，必须保留并恢复，而不是因为“用了 remote backend”就允许删工作目录。停机、systemd/NixOS 包装和 upgrade 的顺序成为正确性的一部分。

本方案仍是 single-writer daemon，不交付 HA；它也不是不可信 Terraform 的 sandbox。state、plan、inputs、数据库/WAL 和备份可能含凭据，本次不增加静态加密承诺。相同 OS identity 或具有宿主管理能力的 IaC 仍处于既有受信边界内。

离线迁移需要维护窗口，并可能保留无法自动处理的隔离对象。无法完整覆盖副作用时宁可占用容量，也不能为了恢复扩容而猜测资源不存在。硬寿命计时保持原值，但服务停机期间无法保证实际执行回收。

回滚变为有条件的恢复操作，而非任意切回旧二进制。此限制应在上线前沟通，不能等第一次升级失败才发现。

### 不改变的事情

现有 Profile 激活策略、Pool 选择、RunnerBackend 协议、Busy-safe 语义、两小时默认硬寿命及其明确例外、管理 OIDC/个人 Token 授权、Setup Info 与 operation-log 隔离均不因拆进程改变。诊断仍是证据投影，不成为新的恢复 authority。

## Rollout and review

实现顺序及退出条件只在 spec 0042 §10 维护。每个实现 PR 应说明它完成哪个阶段、哪些 LW 场景实际执行、哪些仅新增测试，以及是否触及默认生产路径。默认切换 PR 必须附迁移与备份恢复演练、正式 binary/OS/Terraform/backend/platform tuple 和未覆盖范围。

跨文档尚待冻结的数值与发布事实继续使用 [D3/R1/R3](../README.md#仍需决定或冻结)，本 ARD 不另建一份 open-decisions 清单。`IMPLEMENTATION_STATUS.md` 仍是进度权威；接受本决定不等于宣布 Worker/control/state 已集成。
