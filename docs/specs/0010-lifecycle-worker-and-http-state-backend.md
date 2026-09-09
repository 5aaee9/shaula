# Lifecycle Worker and Database-backed Terraform HTTP State

- Status: Accepted design; implementation/migration evidence lives in [implementation status](../IMPLEMENTATION_STATUS.md)
- Decision: [ADR-0014](../ard/0014-run-lifecycle-workers-with-a-database-http-state-backend.md)
- v1 Executor Driver: `exec` only

本规范是 Lifecycle Worker、Executor Interface、内部控制通道、Terraform HTTP backend 和 worker recovery 的唯一协议 owner。它修订旧的 daemon-owned Terraform operation orchestration 与 local-state authority；[spec 0004](0004-template-profile-runtime.md) 继续拥有模板、inputs/outputs 和 saved-plan admission。

## 1. Ownership

一个 Runner Generation 对应一个完整生命周期任务，而不是每个 init/apply/destroy 命令各自派发一个任务。

| Module | 职责 |
| --- | --- |
| `shaula serve` / Fleet supervisor | HTTP/SQLite desired state、GitHub Auth/Profile、Scale Set/session、需求与容量、Generation admission、worker supervision、GitHub safety gates |
| Executor Module | 启动、观察、停止并证明 Lifecycle Worker 及其 descendants 的进程归属；v1 的 `exec` Adapter 启动同一 binary 的 `shaula job` |
| `shaula job` Lifecycle Worker | 独占 Workspace、materialize、Terraform init/plan/apply、等待 Retirement、Destroy 和本地清理的顺序控制 |
| Template Runtime Module | worker 内的模板与 Terraform 机制；不认识 Executor Driver，不链接平台 client |
| State Backend Module | Generation-scoped state/lock 的认证读取和原子写入；SQLite 是 authoritative Terraform state store |

Runner Resource 中的 `Runner.Listener`/workflow 与 Lifecycle Worker 是不同的进程及信任域。Kubernetes Runner Pod 不是未来承载 `shaula job` 的 Kubernetes Job。

v1 不交付 Kubernetes/remote Executor Driver，不引入 Kubernetes client、远程调度、distributed lease 或 HA。未来 Driver 必须另行验证制品/inputs 交付、网络认证、persistent state、descendant fencing 与 shutdown；不能仅凭一个 Driver enum 宣称可支持。Executor 的 stop/exit 不是 GitHub removal 或资源 Destroy 的证明。

## 2. Worker launch and retained facts

`shaula job` 是内部执行子命令，不是绕过 Registry 的 operator Create/Destroy CLI。未经 daemon 准入的调用无权通过内部 Interface 获取材料、JIT、state 或副作用许可；直接手工执行不能创建第二个 Generation 或绕过 Fleet fences。

启动前 daemon 在一个短事务中记录 Generation identity、创建时的 Fleet Revision、exact Template/activation provenance/input refs、stable Runner name、Resource Occupancy slot 与 Worker Claim。兼容的 `attestation_id` 等历史字段承载 [spec 0017](0017-automatic-template-activation.md) 定义的 opaque activation ID；自动静态激活不伪造 conformance record。Claim 含单调递增的 worker epoch、唯一 attempt、Executor identity 和可验证的进程归属；worker epoch 与 Fleet session epoch、Terraform lock ID 是不同身份。

v1 通过受保护的 inherited pipe/file descriptor 或等价 exec handoff 交付启动材料：Generation/epoch、批准的 artifact/input locations、内部地址及分权 capabilities。argv 只能包含非 secret identity；不能放 credential、JIT、bindings 或 token。不从 caller 提供的 argv/env/path 选择可执行模板。

**不要求** SQLite 镜像 worker 的每一步 init/plan/inspect、进程内等待或每条 Terraform 命令。**仍须**保留足以阻止重复副作用的事实：

- Generation/Worker Claim、不可变材料引用和容量占用；
- daemon-owned GitHub mutating intents/results、exact Runner/Scale Set/Auth identities；
- Create 是否已经授权且可能启动、尚未解决的 spawn handover；
- GitHub safe-removal 结果、database state/version/lock、terminal completion receipt；
- 不能自动恢复的原因及必要的进程/应急 state 证据。

Coarse status/heartbeat 用于观测，不是可重放的命令日志；heartbeat timeout 不能证明 worker 或 provider 已死。

## 3. Full worker lifecycle

1. 获取并验证 exact Generation 材料；在独占 Workspace 用 filesystem reflink/CoW materialize immutable Template。CoW 不支持时普通复制；禁止可写 hardlink、共享 `.terraform`/state、path escape 或先删除遗留 Workspace。副本必须与 admitted artifact 匹配。
2. 写入 worker-owned、非 secret 的 `backend "http" {}` 配置，并用最小 per-child env 指定本规范的 backend。仅此固定 backend 文件可作为 admitted artifact 外的系统文件；Profile 不得自选 backend、覆盖内部地址或注入 hook。运行 locked `terraform init`，不使用 `-upgrade`。
3. 通过 daemon 控制通道获取 JIT。daemon 验证 Fleet/worker/session fences、durably 记录 intent/result；GitHub credential 不返回给 worker。JIT uncertain response 沿 stable Runner identity lookup/remove/fresh-Generation 规则处理。
4. 保存 exact protected input；生成并验证 Create saved plan，遵循 spec 0004 的 empty-prior-state、create-only、shape/cardinality policy。获取 daemon 的单次 Create-start authorization 后仅执行该 exact plan。
5. Create-start handover 是短暂的 side-effect gate：daemon 在准许可能的 spawn 前持久化事实，并阻止冲突 Fleet mutation 越过未解决 handover；worker 确认 spawn 或“不能再 spawn”后才释放。事务不跨 IPC/network/process wait。丢响应、crash 或 timeout 必须由 Executor 证明旧进程不能继续后分类，不能当作未启动。由此保留 DELETE commit 后不出现迟到 Create spawn 的保证。
6. 验证受保护的 `shaula_result`，由 daemon 的 GitHub inventory 判断 online/busy。worker 等待 daemon 的 level-triggered observation/Retirement 指令，不把单个 JobCompleted hint 或 provider running 状态当作删除许可。
7. 请求 daemon 执行 GitHub removal safety gate。`JobStillRunning`、ownership ambiguity 或 `ScaleSetMissingWithResources` 都不准 Destroy；重试期间资源继续占用容量。Decommission 使用相同 cleanup-only gate，不重新 acquiring。
8. 获准后从原始材料和 database state 生成 delete-only saved destroy plan，执行 `terraform plan -destroy -out=...`、policy inspection、`terraform apply <saved-plan>`。这是 Destroy operation 的固定实现，不执行未经检查的目录式 apply/destroy；不对原 Generation 做 Update 或第二次 Create apply。
9. Destroy 成功后通过 backend 读取的 state 运行 `terraform state list` 验证为空，并提交绑定当前 state version/Worker Claim 的 completion。daemon 原子确认 safe-removal、当前 worker、无 active command/lock、可信 terminal classification 与该 exact empty state version，然后 seal state writes、写 terminal receipt 并释放 Occupancy。仅无资源的初始 state 不证明曾可能启动的 Create 已清理。
10. worker 收到 durable completion acknowledgement 后才清理本地 Workspace。Lost-response 重试返回原 completion；不得 reopen sealed state。数据库中的空 state、terminal receipt 和引用按 retention policy 清理，不靠 worker 的无条件 finally 或 backend DELETE 清除。

Create 一旦可能启动，恢复不再执行第二次 Create apply。Destroy 可以在前一 child 已确认结束/被 fence 且原始 state 可用时，重新生成新的 delete-only plan 重试。worker 可顺序控制这些步骤，不需要中央调度器逐命令派发。

## 4. Internal HTTP and credentials

管理 UI/API/health listener 仍遵循 [spec 0009](0009-mandatory-openid-connect.md) 的 mandatory OIDC。内部 worker/control/state 使用**独立、仅 loopback 的 HTTP listener**，不由管理 reverse proxy 暴露；默认拒绝未知 route/method，且所有请求在读取 body 或 state 前认证。它不是管理面匿名例外或 legacy backend-token fallback。两个 listener、SQLite 和恢复入口在 worker launch 前就绪。

每个 Worker Claim 使用 daemon 生成的高熵、受保护且可撤销 capabilities，绑定完整 Generation incarnation/ID、worker epoch 和有限操作权限：

- **Worker control capability**：只访问该 Generation 的 bootstrap/JIT、Create-start handover、observation/retirement、completion；不访问任意 Fleet/Profile mutation 或原始 GitHub credential。
- **State backend capability**：只访问该 Generation 的 Terraform state/lock，不能调用 worker control、签发 JIT 或操作其他 Generation。

控制通道是版本化、strict typed 的请求/响应 Interface：按 Generation/epoch 授权 JIT、Create-start 及其 spawn acknowledgement、有限命令预算、level observation/retirement、completion/credential renewal。mutating 请求使用稳定 request identity 并绑定现有 Generation 的 GitHub/start/terminal facts；重试不能产生第二个 JIT/Create 或另一 Runner。读请求/heartbeat 不逐条持久化；body 不能提交任意 command、GitHub URL、Profile 或 state key，claim/version mismatch fail closed。内部 route/DTO 的具体名称不是 operator API，两个执行角色必须验证协议兼容性。

控制通道使用其专用 bearer capability；Terraform 使用 Basic authentication，固定 username `shaula-state`，password 为专用 state capability。两类 token 不可互换。Cookie、普通管理 OIDC token、body identity、`Who`/`Path` lock metadata 和旧管理 backend token 都不能替代内部 capability。新的内部认证只由本规范授权，不改变管理面认证。

Credential 由 daemon 通过受保护 handoff 交付。Terraform 的 password 仅通过它的 `TF_HTTP_PASSWORD` env 提供，不放在 HCL、`-backend-config`、argv 或日志；worker control token 不进入 Terraform env。这是 narrow state-credential exception，不允许 GitHub/OIDC credential 或 daemon 全量 env 进入 IaC child。所有内部凭据都不进入 Runner Execution Domain。

凭据以受保护 verifier/记录绑定 Claim；进程重启不能凭任意旧 token 生成新 Claim。续期需要验证同一 current Claim；expiry/revocation 阻止后续请求，但不构成旧 provider 停止的证明，也不自动释放锁。Terminal 后只允许有界、幂等的 completion receipt 重放，不再授权 backend write 或新副作用。

内部 traffic、state、lock info、Authorization、query lock ID、protected inputs 和 failure bodies 均不进入通用日志/metrics。所有响应 `private, no-store`；ordinary management reads 只返回非 secret state/worker status，不返回 state bytes。内部 limits/credentials 与 public HTTP 配置分离，具体运行限额由决策清单 D3 冻结。

## 5. Terraform HTTP backend wire contract

[Spec 0019](0019-workflow-jobs-and-operation-logs.md) 的 Operation Log 使用独立存储接口，不复用本节 state route、lock 或 state capability。未来 worker 只可向 daemon 交付本 Claim/Generation 的日志；Runner 仅可通过另一独立 capability 读取 Create 安全投影，仍不得访问本规范的内部 capabilities/listener。

State key 由 daemon 的 Generation identity 派生，不是 caller 的任意 workspace/path。v1 对一个 URI 提供以下固定协议，无需额外 method aliases：

`/internal/v1/generations/{generationId}/state`

| Method | 行为 |
| --- | --- |
| GET | 返回原始 Terraform state JSON bytes；不是管理 JSON envelope |
| POST | Terraform 默认 update method；携带 `?ID=<lock-id>`，按 §6 原子提交 state |
| LOCK | body 为 Terraform LockInfo，含非空 `ID`；成功 200，竞争 423 并返回有界的持有者 LockInfo |
| UNLOCK | body 含匹配的 LockInfo `ID`；仅释放当前 Claim 所持有的 exact lock；成功 200，其他持有者 423 |
| DELETE / 其他 | 405；state purge 不属于 worker 权限，不能靠 force-unlock/state-rm 或删除 backend 绕过 cleanup |

设置 `TF_HTTP_ADDRESS`、`TF_HTTP_LOCK_ADDRESS`、`TF_HTTP_UNLOCK_ADDRESS` 为同一固定 Generation URI；显式设置 update/lock/unlock methods 为 POST/LOCK/UNLOCK。禁止 `-lock=false`、跳过 TLS certificate validation、Profile override 或本地 state fallback。

只有**已准入且尚未初始化 state** 的 Generation 可以在 GET 返回 404，让 Terraform 初始化。未知/无权限 identity 不被当作新 workspace；state 曾存在或 Create 可能启动后的缺失/损坏是 operational failure，返回拒绝/5xx 并阻止 effect，不伪装成 404/empty state。正常 Destroy 写入有效的空 state，而非移除 state row。

Terraform 标准 HTTP backend 在 update query 传 lock `ID`，不提供通用 `If-Match`/expected-version 协议。因此不能要求原版 Terraform 发送其不支持的 CAS header。本规范的 CAS 是服务端对锁所有权和 state 记录的原子条件更新，加上 Terraform lineage/serial 与 saved-plan checks；不宣称提供任意无锁 client 的 optimistic-CAS Interface。

## 6. Database lock and state CAS

每个 Generation 有一份 state record 和至多一份 lock record；可以合表，物理表形态不是协议。State 保存原始 bytes、backend revision、首次接受的 Terraform lineage 与 serial、初始化/封存事实。Lock 保存 current Worker Claim/epoch、LockInfo ID 和受保护的 bounded metadata。Backend revision 是服务端单调版本，不是 Fleet Revision、worker epoch 或 Terraform serial。

所有 LOCK、UNLOCK、state write 与 completion seal 通过同一 SQLite writer/事务 discipline 排序；不得先在事务外检查锁，再独立 UPDATE state。需要 CAS 的读取/写入在同一受保护事务中，数据库唯一约束保证并发创建空锁只有一胜者。

- **LOCK**：认证并验证 current Claim、Generation 未 sealed、该 worker 无冲突 Terraform command；只有无锁时能原子安装新 ID。同 Claim/ID 的相同请求可幂等成功；另一 ID 或 epoch 返回 423，不能覆盖 lock metadata 或靠相同字符串冒充持有者。
- **POST**：即使当前没有锁也拒绝无锁写。事务内校验 capability/epoch、unsealed、query ID 等于当前 lock ID，以及当前 backend revision；验证有效、有界且受支持的 state header 后更新 bytes、serial 和 backend revision。条件更新必须确认唯一目标行匹配；零行/版本冲突不能返回成功。已验证的同内容 replay 除外，不重复改写。失败不改变 state 或锁。
- **Lineage/serial**：首次 state 写入固定 lineage；后续 lineage 不变，serial 不倒退。相同 serial 仅允许相同 bytes 的幂等重放；相同 serial 不同内容、旧 serial、其他 lineage 都返回 conflict。更高 serial 可推进，不要求固定 +1，以兼容 Terraform 的合法持久化行为；不同 generation 不共享 lineage authority。
- **UNLOCK**：只删除匹配 current Claim/epoch/ID 的锁，不修改或删除 state。已认证 current Claim 的有效 UNLOCK 在无锁时幂等返回 200，不修改任何数据；事务中若已有后来安装的锁，仍必须匹配其 ID，不能误删。LockInfo 的 Who/Path/Operation 不是访问控制事实。
- **Lost response**：state commit 成功但响应丢失时，相同持有者/ID/serial/bytes 重试返回成功且不重复推进 backend revision。若其锁已失效或 epoch 已换代，拒绝迟到写，不能为方便重放绕过当前 ownership。

锁通常由 Terraform 在每次需要它的命令期间持有，不贯穿数小时的 Runner 等待；覆盖整个 Generation 的互斥由 Worker Claim 提供。数据库 state 锁只能阻止 state 覆盖，不能阻止已经启动的 provider 修改基础设施。

Lock 不设置“超时即自动接管”语义。恢复必须先通过 Executor 验证旧 worker **及其 Terraform/provider descendants** 已退出或不能再修改资源，再原子更换 Worker Claim/epoch、撤销旧能力并有审计地处理 orphan lock。PID 不存在、连接断开、lease/heartbeat 超时都不是单独充分证明。无法证明时保持 Occupancy 并 Blocked/Quarantined；不提供普通 HTTP force-unlock。

## 7. Recovery and backup

- daemon restart 从 Generation/Claim/backend state 恢复 supervisor；不能把内存 worker list 清空当作没有资源。旧 worker 仍活着时先恢复其可信控制关系或终止/fence，不能并行启动 replacement。
- worker crash 后可重建本地普通副本和 backend 配置，但只能使用 retained exact artifact、inputs 和数据库 state。Create 未授权且 GitHub effects 可证明未开始时可继续原 intent；Create 已可能开始后，replacement worker 只能观察/cleanup/Destroy，不能重做 Create。
- GitHub known Busy 的既有 Runner 可以继续运行；replacement worker 通过 daemon observation/removal gate 等待，不因失去原 worker 而立即删除资源。
- Backend outage 时 worker 不继续启动新的 mutating command。已经运行的 Terraform/provider 可能完成外部效果却无法上传 state；必须保护并保留引擎产生的 emergency local state（例如 `errored.tfstate`）和 Workspace，报告 uncertain outcome，不能 cleanup、宣称 Destroyed 或盲目 state-push/re-apply。
- Database state 与 emergency local state 不一致时保持隔离；只有经受控恢复证明的 exact state 才能用于 cleanup。禁止用较旧 DB snapshot 覆盖可能更新的应急证据。
- 若不能证明 state 覆盖了全部可能发生的资源副作用，清理已知子集后得到空 state 也不能消除 uncertainty。初始空 state、no-op Destroy 或新 backend revision 都不是该证明；Generation 继续 Quarantined/占用容量，等待明确恢复证据。
- 已存在完整、可信的数据库 state 和 frozen inputs 时，丢失普通 CoW/copy Workspace 不是 state loss；可以重建。丢失唯一未上传 state、原始 inputs 或 artifact 则不能假定可恢复。

Backup consistency set 是 SQLite（含 state/locks/Generation/worker/GitHub facts）、immutable artifact/retained protected-input store，以及所有尚未上传/解决的 emergency state。普通 materialized copy 和 provider cache 可重建。备份与恢复要同时处理正在进行的 backend commit/worker effects，不支持只恢复部分成员。任意旧 snapshot 不是当前外部资源真相：若 checkpoint 后还可能产生效果，必须保留后续证据并分类，否则保持隔离。恢复不能复活旧 capability/自动抢占 Claim，仍先证明旧执行者已被 fence；DB/WAL/SHM、备份、state、JIT、bindings 和 emergency files 都是 credential-grade。

从旧 local-state 实现迁移时先停止新 claims 并证明旧 Terraform descendants 静止，再将每个 exact Generation 的原始 state/lineage/serial 和 inputs/ownership 一致导入。未迁移的 Generation 必须 fail closed，不能在新 backend 中作为 fresh 404 重新 Create。导入不能改写原 Profile/activation provenance/input 身份，历史 attestation pin 也保持原样。若新 worker 无法遵守旧的 pinned runtime tuple，必须由原 Runtime 安全清退或保持隔离；不能伪造兼容性。对新 runtime/trust policy 声称原 conformance 保证需要对应新证据，但 conformance 不控制 spec 0017 的自动激活。本轮文档不授予丢弃或重建旧 ledger 的许可；实现迁移及其 crash tests 是发布门槛。

## 8. Scheduling and shutdown

一个 worker 可等待一个 Runner 数小时；worker 总数/资源限额与活跃 Terraform Create/Destroy command budgets 分开。等待 GitHub 不占用 apply worker slot；Create/Destroy 保留独立预算和跨 Fleet 公平性。同 Generation 至多一个 current Worker Claim、同 Workspace 至多一个 Terraform command。预算 permit 位于短控制请求和命令执行之间，不通过长期 SQL transaction 实现。

Graceful daemon shutdown 先停止管理 mutation/new claims，再停止 acquisition，要求 worker 在 bounded deadline 内暂停到可恢复状态或停止其 descendants。内部 control/state listener 和 DB 必须在 worker 最后 state write/UNLOCK/receipt 之后关闭；不得先关闭 backend 再要求 Terraform 保存 state。超时不能假报 child dead 或 Destroyed。

普通 daemon shutdown 不触发 fleet-wide Destroy；Runner/Scale Set 与恢复材料保留。强制终止后的 orphan locks/worker facts 走 §7，不能凭 daemon ownership lock 自动抢占仍可能运行的 provider。

## 9. Acceptance

新架构未通过以下场景前不能替换原实现的兼容性声明：

1. 同一 binary 的 `serve` 经 exec Driver 启动一个 `job`，完整执行两种 bundled Profile 生命周期；daemon 不再逐命令调度 Generation 的 init/plan/apply/destroy；隔离的 Profile 静态 validation 不是 Generation lifecycle。
2. CoW 与普通复制等价且隔离；一个 Workspace 修改不污染 artifact/其他 worker；禁止 writable hardlinks 和 path escape。
3. 真实 pinned Terraform HTTP backend 完成 init、plan/apply、read、destroy；抓包证明 POST 带 `?ID=`、LOCK/UNLOCK body/status 与标准协议相符，无自定义 If-Match 依赖。
4. 并发 LOCK 一胜一 423；无锁/错误 ID/旧 epoch 写拒绝；锁检查与 UPDATE 间插入并发 unlock/relock 不能越权写；wrong UNLOCK 不释放新锁。
5. state lineage/serial、同内容重放、响应丢失、重复 UNLOCK、sealed writes、未知 Generation 与既有 state 丢失的区别均覆盖事务/crash tests。
6. kill worker、kill daemon、主机恢复及 orphan Terraform/provider 注入证明无双 worker/并发 apply；锁超时不自动接管；Create uncertainty 不 re-apply，Destroy 可经新 delete-only plan 重试。
7. Busy removal 使 Destroy call count 为零；missing Scale Set/unknown ownership 保持 Blocked；worker exit 不提前释放 Occupancy。
8. Backend 中断保留 emergency state/Workspace；旧 DB state 不覆盖新应急证据；空 state completion/seal、lost completion response 与 cleanup 顺序经 fault injection 验证。
9. OIDC 管理 listener 与内部 capability listener 相互拒绝错误凭据；control/state tokens 分权且跨 Generation/epoch 拒绝；tokens/JIT/provider/GitHub credential 不进入 Runner 或日志。
10. Same-Profile credential rollout 与 Decommission cleanup 仍由 daemon GitHub gate 按 exact Auth refs 执行；worker 不持有原始 GitHub credential。
11. HTTP-state backup/restore 与旧 local-state migration 均通过 interrupted-transaction/worker tests；普通 Workspace 可重建不被误判成权威 state 丢失。
12. Future Executor 只能通过相同 Interface 契约验收；v1 配置拒绝未实现的 Driver。

## References

- [Terraform HTTP backend protocol](https://developer.hashicorp.com/terraform/language/backend/http)
- [Terraform state locking](https://developer.hashicorp.com/terraform/language/state/locking)
- Design reference: `sdwan-terraform-backend`, repository `https://git.wanix.net/backend/sdwan-terraform-backend.git`, inspected commit `a678b34d88d431a1a4de2446ff4c77da5d1dda9c`; local checkout under `references/sdwan-terraform-backend` is reference-only, not a build/runtime dependency.
- Reference paths: `pkgs/http/tfbackend/{backend,lock,state,auth}.go`, `pkgs/database/models/state_locks.go`, `cmd/runner/{run,terraform}.go`, `pkgs/executor/executor.go`. Shaula borrows the HTTP/DB/worker separation, not its unlocked-write allowance, separate lock-check/state-update operations, raw output logging, implicit init upgrade or destructive workdir recreation.
