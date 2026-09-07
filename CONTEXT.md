# Shaula

Shaula 管理多个 GitHub Actions Runner Scale Set 的容量，并通过基础设施模板创建和销毁短生命周期 Runner。本词汇表统一规划、实现和运维中使用的领域语言；协议、决定和实现进度的权威位置见 [文档索引](docs/README.md)。

## Language

**Fleet**:
Shaula 中独立收敛的控制单元，由一个 GitHub Actions Scale Set、一个 GitHub Auth Profile、一份选定的 Template Profile Revision 和其全部 Runner 组成。
_Avoid_: Cluster, pool, daemon

**Fleet Key**:
一个 Fleet 在 Shaula 中的稳定本地身份。
_Avoid_: Scale Set ID, template name

**Fleet Spec**:
一个 Fleet 的完整期望状态，包含 GitHub Target、GitHub Auth Profile、Template Profile Revision 与容量策略。
_Avoid_: Bootstrap configuration, Terraform plan, current state

**Fleet Revision**:
Fleet Spec 经规范化和校验后形成的不可变版本；Fleet 的 desired head 指向当前期望 Revision。
_Avoid_: Runner Generation, mutable record

**Fleet Status**:
一个 Fleet 当前所有权、listener、容量和收敛结果的只读观测；已观测 Fleet Revision 不代表容量已经收敛。
_Avoid_: Fleet Spec, desired state

**Fleet Change**:
一次已接受 Fleet mutation 的异步进度记录；它描述 Fleet Revision 的处理结果，不是 Runner Operation。
_Avoid_: Runner Operation, request, IaC run

**Fleet Mutation Fence**:
随 Fleet desired mutation 单调递增的并发令牌，用于阻止过期 Create 在新的期望状态下产生资源。
_Avoid_: Runner Operation lease, ETag

**Scale Set**:
GitHub Actions Service 中接收同一组路由标签并共享容量信号的一组 Runner。
_Avoid_: Fleet, runner group

**GitHub Target**:
一个 Scale Set 在 `github.com` 上的注册目的地；v1 的 Target 是 organization 或 repository。
_Avoid_: Config URL, tenant, runner group

**GitHub Auth Profile**:
一个可由多个 Fleet 引用的稳定 GitHub 控制面身份与 Target policy；其认证方式只能是 GitHub App 或 PAT。
_Avoid_: GitHub token, inline credential, fallback chain

**GitHub Auth Revision**:
一个 GitHub Auth Profile 下不可变的 credential 版本；Profile 的 active head 指向通过验证后供 Fleet 使用的 Revision。
_Avoid_: Fleet Revision, installation token, mutable secret

**GitHub Auth Revision Ref**:
Fleet 对一个精确 GitHub Auth credential 的完整引用 `(profile_key, revision)`；desired 与 observed 必须始终作为完整 tuple 比较、持久化和释放。
_Avoid_: revision number alone, current credential, implicit profile key

**Auth Handoff**:
Fleet 在切换 desired GitHub Auth Revision Ref 时持久化执行的 quiesce 与 access/ownership classification；它只推进 observed ref，不创建或采用 Scale Set、不绑定 ID，也不建立 session。
_Avoid_: Runtime fallback, credential retry, Scale Set reconcile

**Control-Plane Credential**:
GitHub App private key、installation/admin token 或 PAT 等只供 Shaula 调用 GitHub 管理面使用的凭据；它们不属于 Runner 或 workflow 身份。
_Avoid_: Runner token, bootstrap token, workflow secret

**Workflow Credential**:
GitHub 为单个 job 提供的 `GITHUB_TOKEN`，或 workflow 作者显式注入的其他 Actions Secret；它不属于 Shaula 的 Scale Set 管理身份。
_Avoid_: PAT by default, control-plane credential, runner registration

**Template Platform**:
承载 Runner Resource 的基础设施环境类型；Kubernetes 与 Docker 是 v1 的两个 Template Platform，其身份只能从 admitted Template Artifact manifest 派生。
_Avoid_: Template Provider, executor, backend

**Runner Execution Domain**:
一个 Runner Resource 内 bootstrap shim、`Runner.Listener` 与 job process 共享的执行信任边界。v1 不保证对具备同域进程检查能力的 workflow 隐藏 JIT，但 GitHub Control-Plane Credential、Platform Provider Credential 与 sensitive Template bindings 永不进入该域。
_Avoid_: Tenant sandbox, credential broker, control-plane trust domain

**Template Profile**:
一个可被 Fleet 选择的稳定 Runner 基础设施契约，其各个 Revision 对同一组输入定义可复现、能力同质的 Runner；static validation 只产生 `Ready`，exact conformance attestation 才能产生 `Active`。
_Avoid_: Provider, image, template directory

**Template Profile Revision**:
一个 Template Profile 的不可变版本，固定其 Template Artifact、manifest contracts、bindings、输入契约和被证明的 compatibility tuple，但不内嵌后置 attestation；只有 current Active Revision 可接收新的 Fleet 引用，旧 exact pin 可供已准入 Fleet 正常 reconcile/Create/Destroy/recovery。
_Avoid_: Runner Generation, mutable profile, latest template

**Template Artifact**:
实现一个 Template Profile Revision 的不可变基础设施模板包，其内容身份在整个 Runner Generation 中保持稳定。
_Avoid_: Local template path, mutable directory, Runner Workspace

**Profile Change**:
一次已接受 Template Profile 或 GitHub Auth Profile mutation 的异步进度记录。
_Avoid_: Fleet Change, Runner Operation, request

**Runner**:
在 Scale Set 中注册且至多执行一个 GitHub Actions job 的短生命周期执行者；其执行环境由一个 Runner Resource 承载。
_Avoid_: VM, Pod, container

**Runner Generation**:
一次不可变 Runner 创建尝试所产生的身份；更换模板或输入会产生新的 Generation，而不是更新原 Generation。
_Avoid_: Version, revision

**Lifecycle Worker**:
由 `shaula job` 承载、负责一个 Generation 从 materialization/Create 到等待安全清退、Destroy 和清理的顺序执行者；它向 daemon 请求 GitHub 安全授权，不持有 GitHub Control-Plane Credential。
_Avoid_: GitHub workflow job, Runner.Listener, one Terraform command

**Executor Driver**:
启动、观察并停止/fence Lifecycle Worker 及其 descendants 的执行 Adapter；v1 只有本地 `exec`，未来 Kubernetes Job 是另一种 Driver，不是 Runner Resource。
_Avoid_: Template Platform, Terraform provider, Runner executor

**Worker Claim**:
一个 Generation 当前唯一被准许执行的 Lifecycle Worker 归属；worker epoch 标识新旧 attempt，恢复必须先证明旧执行者不再能产生副作用。
_Avoid_: Terraform lock ID, session epoch, heartbeat alone

**Runner Registration**:
Runner 在 GitHub Actions Service 中的身份及其一次性 JIT bootstrap payload。
_Avoid_: Runner Resource, JIT token

**Runner Workspace**:
一个 Generation 的 Lifecycle Worker 独占的本地执行目录，包含 CoW/copy 的 Template、受保护输入和运行证据；authoritative Terraform state 在数据库 HTTP backend，未上传的 emergency state 仍须在此保留。
_Avoid_: Terraform named workspace, shared workspace, sole authoritative state store

**Terraform State Backend**:
daemon 提供的内部 Generation-scoped HTTP 存储边界；通过数据库事务保存 Terraform state、验证 current Worker Claim/lock 并处理 LOCK/UNLOCK。其 state lock 不证明 provider 已停止。
_Avoid_: Runner platform, general-purpose remote backend, infrastructure fence

**Runner Resource**:
Template Platform 中承载一个 Runner 的 generation-scoped 资源集合，不限定为 VM、Pod、container 或其他具体资源类型。
_Avoid_: Runner, executor Job

**Kubernetes Runner Resource**:
Kubernetes Template Platform 对一个 Runner Generation 的资源形态，由一个 Pod 和一个 JIT bootstrap Secret 构成，并位于预先创建的 namespace。
_Avoid_: Deployment, executor Job, namespace

**Docker Runner Resource**:
Docker Template Platform 对一个 Runner Generation 的资源形态，由一个 generation-scoped container 构成。
_Avoid_: Compose project, Swarm service, host daemon

**Kubernetes Generation Resource Key**:
由 exact Kubernetes target binding、namespace name、resource kind 与 exact generation-scoped `metadata.name` 组成的 Shaula 逻辑身份；同一 Generation 中稳定，不同 Generation 不复用或碰撞同一个 key。它不是 Kubernetes `metadata.uid`，也不证明 target、namespace 或对象未被外部 actor 同名重建/重指。
_Avoid_: Kubernetes UID, metadata.uid, reusable name, proof against out-of-band replacement

**Runner Operation**:
作用于一个 Generation 的 Create 或 Destroy 生命周期行为；必要的外部副作用意图/结果持久化，但不是由 daemon 逐 Terraform 命令派发的 durable task。
_Avoid_: Update, worker process, one Terraform command

**Quarantine**:
Shaula 无法证明某个 Runner Generation 的资源身份、状态归属或安全销毁条件时进入的持久化隔离状态；该 Generation 继续占用容量，直到显式、可审计的恢复流程解决。
_Avoid_: Retry loop, Destroyed, forgotten resource

**Assigned Demand**:
GitHub Actions Service 为一个 Scale Set 最近一次报告的已分配 job 总量，是可覆盖的当前状态而非可累加事件。
_Avoid_: Queue length, event count

**Job Observation**:
Shaula 收到的 JobStarted 或 JobCompleted 提示；它可能重复、乱序或缺失，不是审计日志。
_Avoid_: Durable event, source of truth

**Retirement**:
Runner 不再承接新 job、等待安全注销并最终销毁 Runner Resource 的单向过程。
_Avoid_: Update, shutdown

**Decommission**:
一个 Fleet 永久停止新 acquisition/Create、安全清退全部已知 Runner 并留下 tombstone 的终态过程；期间只允许 non-acquiring cleanup Auth Handoff，v1 保留其空 GitHub Scale Set。
_Avoid_: Force delete, record purge, daemon shutdown

**Adoption**:
Shaula 在验证身份和兼容性后继续管理已存在的 Scale Set，而不是创建新的 Scale Set。
_Avoid_: Import, takeover

**Effective Capacity**:
一个 Fleet 中正在创建、等待上线或仍可服务 job 的 Runner Generation 数量；Busy Runner 计入其中。
_Avoid_: Resource count, max capacity

**Resource Occupancy**:
一个 Fleet 为尚未完成 Destroy 的 Runner Generation 保守预留的容量槽数量。
_Avoid_: Effective Capacity, platform resource count

**Session Epoch**:
一个 Fleet 每次安装新 Scale Set message session 时单调递增的持久化代次；session-effect gate 与 durable CAS 共同禁止过期 epoch 的 poll task ACK、Acquire、修改当前 demand/observation 或唤醒 lifecycle。
_Avoid_: Message ID, Fleet Revision, process generation

**Template Bindings**:
Template publisher 为连接预先存在的平台环境而提交、并冻结到 Template Profile Revision 的有界输入；其 schema 由 Template Artifact 声明。标记为 sensitive 的 member 是 write-only，不能通过管理读取面取回。
_Avoid_: Provider Target, GitHub Target, arbitrary tfvars

**Protected Bindings Commitment**:
绑定一个 exact immutable Template Profile Revision 的不透明 equality token，wire name 为 `bindings_digest`；它不得是 sensitive binding plaintext 的 unkeyed digest 或 credential fingerprint，且不得让读取者离线验证 secret guess。
_Avoid_: Credential hash, plaintext checksum, secret verifier

**Template Conformance Attestation**:
引用 Template Profile Revision/canonical subject、证明其精确 artifact、dependency lock、engine binary/provider、Protected Bindings Commitment、runtime/trust policy、Runner image、manifest contracts 和测试套件组合满足平台契约的独立 immutable record；它不修改 Revision，activation transaction 冻结其 ID，并以此作为 Template `Ready -> Active` 的必要门禁。
_Avoid_: Static validation, latest-provider promise, platform implementation

**Runner Consistency Set**:
安全恢复 Generation 所需的 SQLite（含 Terraform state/lock、worker/GitHub/terminal facts）、冻结 artifact/inputs，以及未解决的 emergency state 集合；普通 materialized copy 与 provider cache 可重建。
_Avoid_: State file alone, database backup alone, best-effort cache

**Create Start Authorization**:
daemon 针对 current Worker Claim 和 Fleet Mutation Fence 记录的单次 Create-start 许可；未解决的 spawn handover 必须保守视为 Create 可能已开始，不能通过重启获得第二次 Create apply。
_Avoid_: Terraform lock, PID alone, retry counter
