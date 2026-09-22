# Forgejo Runner Backend（Pool 切片先行）

- Status: Draft（2026-09-11）；本规范是目标契约，implementation 与真实平台验收证据只记录在 [实现状态](../IMPLEMENTATION_STATUS.md)。
- Decision: [ARD-0033](../ard/0033-admit-forgejo-through-a-pool-backend-first.md)。
- Implements: [spec 0001](0001-shaula-runner-scale-set.md) 的 Fleet / Generation 模型在第二个 Runner Backend 上的最小落地；为 [spec 0004](0004-template-profile-runtime.md)、[spec 0019](0019-workflow-jobs-and-operation-logs.md)、[spec 0020](0020-official-container-runner-bootstrap.md) 增加 provider 维度，不修改其 GitHub 语义。
- Open decisions: 见 [文档索引](../README.md#仍需决定或冻结) 的 D5、D6、R4。
- 事实基线：Forgejo `e27d0384`、forgejo-runner `667c8d97`（module `code.forgejo.org/forgejo/runner/v13`）。规范中的 Forgejo 行为均以这两个 commit 的源码为准，不依赖第三方文档。

## 1. Scope and ownership

本规范拥有：Forgejo（服务端 ≥ 15）与 forgejo-runner（≥ v13）作为第二个 Runner Backend 的对外契约——Fleet 的 provider 维度、Pool Fleet 的容量与生命周期语义、控制面 adapter 的最小能力集、Runner Bootstrap Material、镜像准入、job 关联等级、Busy-safe removal 与验收。

本规范**不**拥有：Template 输入与发布（[spec 0004](0004-template-profile-runtime.md)）、容器 bootstrap 的 GitHub 契约（[spec 0020](0020-official-container-runner-bootstrap.md)）、Jobs 日志内容与保留（[spec 0019](0019-workflow-jobs-and-operation-logs.md)）、GitHub App 认证（[spec 0011](0011-multi-account-github-authentication.md) / [spec 0018](0018-github-app-only-authentication.md)）、crate ownership 与依赖方向（[spec 0007](0007-rust-workspace-architecture.md)）。

本规范的 provider 扩展 MUST NOT 改变既有 GitHub Fleet 行为。独立的跨后端 Runner 硬超时策略由 [spec 0001 §5.3](0001-shaula-runner-scale-set.md#53-runner-最大存活时间) 统一定义，不属于 Forgejo 专用语义。Forgejo 语义 MUST NOT 注入 `GitHubAccessPort` 或 `shaula-scaleset`；两者的 wire 类型与失败分类保持 GitHub 专用。

## 2. Provider 维度

Fleet Spec 增加判别式 `provider`：

- `kind: github` 沿用现有 `github` 段，语义不变。
- `kind: forgejo` 使用 `forgejo` 段：`instance_url`、`scope`、`auth_profile_ref`、`runner_name_prefix`、`labels`。`scope` 四选一：`instance`（global）、`organization`、`user`、`repository`；由 `(instance_url, scope)` 唯一确定控制面路由前缀，实例 URL 由操作者显式给出，不来自任何自动发现。

约束：

- Forgejo Fleet MUST NOT 声明 `runner_group`，MUST NOT 声称拥有 Scale Set，MUST NOT 参与 owned Scale Set 的创建、labels 收敛、route proof 或 Auth Handoff 流程。
- 当前 Forgejo Fleet MUST 使用单一 `template_profile_ref`；inline `template_pool` 和共享 `template_pool_ref` MUST 在准入时拒绝。Pool Fleet 是等待型 Runner 容量语义，不是加权 TemplatePool。管理 UI 按 Runner Backend 分支显示；模板列表 `runnerBackend` 来自 Active Revision 的实际选择，不来自 Candidate 或平台名称，未知时为 null。
- `runner_name_prefix` 是 ownership 的唯一本地依据：Shaula 创建的每个 runner 名字 MUST 以该 Fleet 唯一前缀开头（含 Generation 标识）。名字匹配只是**弱**所有权证据，MUST 与 inventory 中的 `ephemeral` 标记和 Fleet labels 联合判定；只凭名字冲突 MUST NOT 推断所有权。
- labels 是纯字符串，形如 `name[:backend-target]`（`backend-target` 为 `host`、`docker://image` 等）。Fleet labels 决定匹配集合，同时决定 runner 的执行后端；Template manifest MUST 接纳这些 target，无法接纳的组合在静态校验阶段拒绝。**Forgejo 的匹配规则是"runner 声明的全部 labels 必须是 job `runs-on` 的子集"**，因此 Fleet labels SHOULD 保持最小；文档与 UI MUST 明示该规则，要求 workflow 逐个列出。
- 与同一 scope 内既有持久 runner 共享 labels 属于显式配置错误：MUST 在接受前提示，并记录为"需求信号可能被外部 runner 消耗"的成本，不得声称需求与容量一一对应。

## 3. 控制面能力契约（adapter 最小集）

adapter 只依赖以下 Forgejo HTTP 面，全部要求有界超时、无重定向、请求体与响应大小上限，失败按 spec 0001 §6 的 access failure 分类（`Unavailable` / `RequestUncertain` / 权限与鉴权失败不得当作 absence 证明）：

| 用途 | 端点 | 关键事实 |
| --- | --- | --- |
| 预注册 | `POST {scope}/actions/runners`，body `{name, description, ephemeral: true}` → `201 {id, uuid, token}` | 记录**立即存在**（offline）；`token` 只在响应中出现一次 |
| 库存 | `GET {scope}/actions/runners` | 返回 `status`（`offline`/`idle`/`active`）、`labels`、`ephemeral`、`uuid`、`version` |
| 需求与观测 | `GET {scope}/actions/runners/jobs?labels=…` | 返回 `waiting` + `running` 的 job（`handle`、`attempt`、`status`、`runs_on`、`task_id`、`run_id`、`repo_id`、`name`） |
| 解除注册 | `DELETE {scope}/actions/runners/{id}` | **无**"job 正在运行"保护，见 §6 |

硬性要求：

- ephemeral MUST 由服务端强制。实例或 runner 版本不满足时（无 ephemeral 支持、`one-job` 不存在）MUST 在接受 Fleet 前拒绝，MUST NOT 降级为持久 runner 或猜测 host。
- 未知 `status` 值（未来新增）MUST 按**不可证明**处理：既不能当作 online，也不能当作 absent。
- 需求与库存快照是**覆盖式**快照，MUST NOT 被实现为累加计数。
- 轮询失败 MUST 保留上一份快照并记录 staleness；MUST NOT 把取不到快照解释为零需求。
- jobs 端点当前无分页、labels 过滤在服务端内存执行；adapter MUST 自带条数与频率上限，规模边界与最终 interval/backoff 归 D6。

## 4. Pool Fleet 生命周期

一个 Generation = 一次 ephemeral 预注册 + 一份模板资源（host / container / Pod / VM）+ 一个 `forgejo-runner one-job --wait` 进程。本切片**不传递 `--handle`**：runner 接受任意匹配 labels 的等待 job。

Create 顺序：

1. 锁定 Workspace、init、冻结输入，通过既有通用 plan/shape admission。
2. 生成 Runner Bootstrap Material 并预注册：`POST …/actions/runners`，name 使用本 Generation 的精确名字。响应 Uncertain 时按 §6 分类，不重复注册。
3. 仅执行一次 Terraform apply；引导材料以受保护文件投递（§5），runner 以 `--token-url file:…` 读取。
4. Readiness 由 inventory 驱动（沿用 [ARD-0030](../ard/0030-drive-generation-readiness-from-inventory.md)）：runner 出现且 `status=idle` 视为在线；容器 Running、Pod Ready、进程存活都不是 readiness。
5. Busy 观测：该 runner 的 inventory 状态变为 `active`，或同一 job 从 `waiting` 转 `running` 且时间窗与本 Generation 重合；两者都只能得到**未验证**关联（§6）。

容量语义：

- 需求 = scope 内按 Fleet labels 过滤的 `waiting` job 数。服务端不存在 GitHub 式预分配，因此每个等待 job 需要一个专门 runner：复用既有算术 `target = min(max_runners, min_runners + waiting_jobs)`，`min_runners` 的含义是**预热的等待 runner**。
- 空闲是**有成本的**：等待中的 runner 进程、容器或 VM 持续占用基础设施。空闲 Generation 超出 Fleet 级 idle deadline（provisional 默认 600 秒，最终值与硬上限归 D6）后 MUST 进入 Destroy，并删除其注册。
- 终结证据：ephemeral runner 在任务结束后由服务端删除，其名字从 inventory 消失即正常终结；进程/资源层证据（模板 Destroy）与注册层证据（inventory 缺失或已删除）必须分别记录，不得互相代替。

## 5. Runner Bootstrap Material 与模板

- backend 是现有 Template 的发布选项，不是新的 TemplatePlatform / source kind。Docker / Kubernetes 及显式接纳下述 VM bootstrap 契约的 manifest 可声明 `runner_backends: [github, forgejo]` 能力，publisher 用 `bindings.runner_backend` 选择，默认 GitHub，固定在不可变 Revision；Fleet 参数 MUST NOT 覆盖。旧的单 `runner_backend` manifest 保持原有固定语义。
- Fleet admission / Create MUST 验证 Revision 的 backend 与 Fleet kind 一致；follow 遇到不一致时保留旧 pin。`runner_image: auto` 按该选择解析官方 digest，显式跨 backend alias MUST 拒绝。VM 模板在没有 Forgejo bootstrap 契约前 MUST NOT 广告这个选项。

- Runner Bootstrap Material 是 provider 专用的一次性身份材料。GitHub 侧仍为 JIT config，字段与语义不变；Forgejo 侧为 `{instance_url, uuid, token, labels[]}`。spec 0004 的 `bindings_digest` 语义、envelope `shaula_result` 与本规范无关，不触发 D4。
- token MUST 以受保护文件（Secret / 平台等价物）投递，只被 runner 经 `--token-url file:…` 读取。token MUST NOT 出现于命令行 argv、环境、资源 tags/labels、Setup Info、Operation Log 或任何日志投影。Docker / Kubernetes 仍由 host 在 apply 后投递，MUST NOT 进入 Terraform 输入。
- **VM 显式例外**：Proxmox / AWS / TencentCloud / AliCloud 的 v1 VM-image manifest 可声明 `forgejo_vm_bootstrap_contract: shaula.forgejo-vm-cloud-init/v1`，允许系统字段 `shaula.forgejo_vm.token` 进入受保护 tfvars、plan/state 和 cloud-init user-data / NoCloud ISO。它 MUST 仅为 Shaula 预注册得到的单 Runner token，MUST NOT 是 management/registration scope token；GitHub 和容器输入 MUST 拒绝此字段。选择该发布选项即接纳与 VM JIT 相同的凭据存储边界，base64 / sensitive 不代表加密。来宾以 root 0600 文件接收，转入非 root Runner 私有运行目录；不在来宾二次注册，不重放一次性 seed。该例外不授权 post-apply Runtime hook 或 Setup Info。
- 容器镜像准入从"仅 `ghcr.io/actions/actions-runner`"扩展为"按 bootstrap kind 对应的官方镜像族"：Forgejo 容器族 MUST 使用官方 forgejo-runner 镜像并固定**内容 digest**，禁止自建或重打包镜像。上述 VM 则继承各平台 VM-image 信任契约，下载官方 native Runner，并用 artifact 内固定版本和 SHA-256 验证；不宣称 OCI 镜像 pin。exact registry / version / digest tuple 归 R4。
- labels 的 backend target 决定执行方式（`host` 直接执行、`docker://` 需要容器运行时）。Docker / Kubernetes / Proxmox / AWS / TencentCloud / AliCloud 的默认组合 MUST 在模板内显式声明并分别验收：选择容器后端时，模板 MUST 声明容器运行时与挂载风险；MUST NOT 假定与 GitHub 官方 runner"步骤直接跑在 runner 内"相同的模型。
- 引导材料属于 credential-grade：与 spec 0020 相同的"同 Runner Execution Domain 可读、控制面凭据永不进入"边界继续适用。

## 6. 关联等级与 Busy-safe removal

- 本切片 MUST NOT 产生 `Verified` 的 job ↔ Generation 关联。Forgejo 的公开 API 不暴露 task → runner，因此除"按 handle 定向"（本切片明确不做）外，任何关联只能是 `Unverified` 或 `Ambiguous`。API 与 UI MUST 呈现该等级，MUST NOT 复用 GitHub 的 assignment 文案，MUST NOT 把未验证关联呈现为"执行该 job 的 Runner"。
- 以下是未触发 [统一硬超时](0001-shaula-runner-scale-set.md#53-runner-最大存活时间) 的普通清退规则。硬超时允许中断任务，先销毁已证明归属的资源再清理精确注册；它不能作为 idle-safe 证据，也不取消所有权校验。
- `DELETE runner` 没有运行保护，因此 Busy 判定 MUST NOT 依赖远端拒绝。普通 Destroy 前必须由本地证据分类：
  - **已从 inventory 消失**（服务端在任务结束后删除）→ 安全，继续 Destroy。
  - **仍存在且 `idle`，且模板侧进程/资源证据表明未持有任务** → 允许先 `DELETE` 注册再 Destroy；删除返回 404/不存在视为已收敛。
  - **`active`、状态未知或证据矛盾** → MUST 保守延迟并记为 Busy，MUST NOT 删除注册、MUST NOT 销毁资源。
- 注册的 Uncertain（响应丢失）按精确名字**并联合证据**在 scope inventory 内分类：候选必须同时匹配 `ephemeral=true` 与 Fleet 声明的全部 labels；仅名字相同不足以证明归属。`ExactlyOne` → 效果已落地；由于 `token` 只在响应中出现一次、不可恢复，该注册 MUST 立即移除并使 Generation 进入 `CleanupRequired`（与 [spec 0025](0025-jit-mint-uncertainty-recovery.md) 同构）；`None` → 零资源，可直接重试新的 Generation/name；`Multiple`、查找失败或移除被 Busy 阻塞 → `Quarantined` 并保留 occupancy，由操作者处置。
- 凭据轮换：本切片**不实现 Auth Handoff**。轮换 = 新的 Fleet revision + 现有 Generation 收敛（未持有任务的先 Destroy 再以新凭据重建，Busy 的等其结束后销毁）。额外 destroy/create 是本切片的显式代价，MUST 在 UI 与文档中说明，不得宣称无缝切换。

### 6.1 Jobs 只读投影

`/api/v1/jobs` / `/api/v1/jobs/{id}` 聚合两个 backend 的只读历史，但不复用 GitHub message/session/assignment 身份。Forgejo 记录：

- 按 Fleet incarnation、instance URL、scope、Auth Profile，再加 `repo_id/job_id/attempt` 隔离；名称不参与去重。Forgejo ID 在读取面用十进制字符串，避免浏览器丢失 u64 精度。
- `backend: forgejo` 携带独立 `forgejo` 元数据，不生成虚假的 `scale_set_id`、GitHub URL、conclusion 或 Verified 关联。本切片不猜测候选 Generation，`generations` 为空；运行状态不依赖关联等级。
- `waiting` → `queued`，`running` → `running`；未知状态或从成功的完整快照消失 → `unknown`。保留 `last_reported_status` 与 `last_observed_at`，明确 `in_snapshot`。消失事件不等于完成或成功。
- `forgejo_observations` 保存状态/Task ID 变化与消失事件；相同重复轮询不增加事件。读取最多返回最新 1000 条，按既有 metadata retention 回收。
- 轮询失败只标记 `stale`，不重写此前成功快照和任务时间；读时超过 30 秒也标记 stale，避免 daemon 停止后永远显示 fresh。`not_listed` 表示最后成功快照已不再包含此任务，不承诺当前终态。
- 观测写入用当前 Fleet incarnation/revision/mutation fence 校验；旧 poll、旧 incarnation 和已删除 Fleet 不得刷新历史。Jobs 不授权任何生命周期操作。
- Jobs 历史是可降级的读取投影：历史写入失败或展示元数据超限 MUST NOT 阻断已独立验证的需求更新、inventory/readiness 或正常回收。控制路径仍独立检查当前 Fleet guard、快照条数、非零 repository/job ID 与去重身份，投影失败不能绕过这些校验。失败时记录有限 reason code 并尽力标记历史 stale；标记也不可写时，已有快照仍按读时 30 秒 TTL 过期。远端 jobs 轮询本身失败仍保留旧需求及时间并冻结普通容量副作用。

## 7. 认证与凭据

- Forgejo 没有 App 与 installation，因此本切片的凭据是独立的 profile kind（例如 `forgejo_token`），MUST NOT 复用 GitHub App 的 revision/binding schema，MUST NOT 作为 GitHub credential 的 fallback，也 MUST NOT 让 GitHub profile 借用 Forgejo token。
- 凭据按 scope 选择：instance scope 需要站点管理员 token；organization / repository / user scope 需要具备对应 owner 权限的 scoped token。exact 权限名与最小权限集合归 D5。
- profile 激活前 MUST 执行一次有界认证读（scope 内 runner 或 jobs 列表）；失败即 Rejected，成功记录 `checked_at` / `valid_until`，沿用既有正/负缓存窗口的时间语义但使用独立字段与语义，不共用 GitHub 的路线证明。
- 凭据只以受保护形式存储，读取面只返回存在性与验证状态，MUST NOT 回显 token。
- 现有 `/api/v1/github-auth-profiles` 路径兼容两种 kind。Forgejo 写入为 `{kind: forgejo_token, instance_url, scope, token}`，不发送 GitHub 的 `schema_version`/App/policy 字段；内部与读取面的 Forgejo revision schema 为 1。周期扫描只做结构检查，不能在在线 Forgejo probe 前按 GitHub 格式拒绝它。

## 8. 明确排除（与完整 provider 抽象的差距）

本切片**不包括**：`--handle` 定向、Verified 关联、Scale Set / runner group / installation / route proof、message session 与 ACK 检查点、Auth Handoff、按 job 选择 Template Profile、两条 driver 的端口归一、任何 GitHub 路径的行为变化。

## 9. 演进到 A 的边界与不变量

从 C 到 A（provider-neutral 控制面端口 + 单一 reconcile 路径 + handle 定向 + 归一化存储）时，下列不变量 MUST 在 C 阶段就成立，否则迁移会重写语义：

1. 需求是覆盖式快照，不是累加计数；快照缺失 = 保留旧值 + staleness。
2. Readiness 由 inventory 驱动，不由进程、Pod 或 apply 成功驱动。
3. 一个 Generation ↔ 一个可证的外部 runner 身份（名字 + uuid），注册与资源分别可证。
4. 效果不确定性一律以精确身份查找分类，绝不盲重试。
5. Busy 判定不依赖远端保护；远端无保护时以本地证据 + 保守延迟。
6. provider 维度只影响控制面交互与引导材料，不影响 Generation 状态机、worker fencing、state backend 与 operation log。

## 10. Acceptance

全部验收都必须记录为"本地运行 / 真实实例运行"的分离证据，不能以规范条款代替：

- **A1 闭环**：真实 Forgejo ≥ 15 + admin token：预注册 ephemeral → runner 以 uuid/token 启动 → Declare 后 inventory 显示 `idle` → 派发一个匹配 job → 任务完成后 runner 记录消失。
- **A2 labels 匹配**：声明 labels 必须全部包含在 job `runs-on` 中；不匹配的 job 不被取走（用两组不同 `runs-on` 的 job 验证）。
- **A3 需求快照**：`waiting` 变化正确驱动容量；`running` 变化不误增需求；轮询全部失败时保留旧快照且不授权普通缩容；独立硬超时不依赖需求新鲜度。
- **A4 Busy-safe**：未到统一硬超时时，对 `active` runner 发起普通 Destroy → 记为 Busy 并延迟，注册与资源都保留；对 `idle` 且无任务证据的 runner → 正常收敛。
- **A5 注册 Uncertain**：注入响应丢失，三类分类（命中 / 缺失 / 歧义）分别按 §6 收敛。
- **A6 关联等级**：Jobs 视图与 API 只显示 `Unverified`/`Ambiguous`，无任何 `Verified` 关联产生。
- **A7 泄漏扫描**：argv、资源 metadata、Setup Info、Operation Log 与普通日志中都不含 token。容器 Terraform 输入中也不得含 token；VM 仅允许 §5 的显式受保护 tfvars/plan/state/user-data 例外。
- **A8 无回归**：既有 GitHub Fleet 的本地测试与真实路径行为不因 provider 维度改变。
