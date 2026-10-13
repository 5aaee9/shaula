# GitLab Runner Backend（共享 Pool driver）

- Status: Draft（2026-10-13）；本规范是目标契约，implementation 与真实平台验收证据只记录在 [实现状态](../IMPLEMENTATION_STATUS.md)。
- Decision: [ARD-0043](../ard/0043-admit-gitlab-as-the-second-pool-backend.md)。
- Implements: [spec 0001](0001-shaula-runner-scale-set.md) 的 Fleet / Generation 模型在第三个 Runner Backend 上的落地；复用 [spec 0026](0026-forgejo-runner-backend.md) 的 Pool Fleet 语义，只定义 GitLab 差异。不修改 GitHub 与 Forgejo 语义。
- Open decisions: 见 [文档索引](../README.md#仍需决定或冻结) 的 D7、R5。
- 事实基线：GitLab CE `19.4.2`（revision `ecff27ae760`）、gitlab-runner `19.4.1`（`3c39fceb`），由 [协议实验](../evidence/gitlab-pool-spike-2026-10-13/README.md) 在一次性实例上观测。规范中的 GitLab 行为以该实验与上游文档为准；未观测的版本不得声称兼容。

## 1. Scope and ownership

本规范拥有：GitLab（自托管 CE/EE）与 gitlab-runner 作为 Runner Backend 的对外契约——Fleet 的 `gitlab` 段、需求 Anchor、控制面 adapter 最小能力集、Runner Bootstrap Material、执行器边界、关联等级、清退顺序与验收。

本规范**不**拥有：Pool Fleet 的通用容量与生命周期语义（[spec 0026 §4](0026-forgejo-runner-backend.md#4-pool-fleet-生命周期)，抽为 provider-neutral 后由该节继续维护）、Template 输入与发布（[spec 0004](0004-template-profile-runtime.md)）、Jobs 日志与保留（[spec 0019](0019-workflow-jobs-and-operation-logs.md)）、统一硬超时（[spec 0001 §5.3](0001-shaula-runner-scale-set.md#53-runner-最大存活时间)）、crate 依赖方向（[spec 0007](0007-rust-workspace-architecture.md)）。

GitLab 语义 MUST NOT 注入 `GitHubAccessPort`、`shaula-scaleset` 或 Forgejo adapter。

## 2. Provider 维度

Fleet Spec 的 `kind` 增加 `gitlab`，使用 `gitlab` 段：`instance_url`、`scope`、`auth_profile_ref`、`runner_name_prefix`、`tags`、`run_untagged`（默认 `false`）。

- `scope` 首个切片二选一：`group`（`group_id` 或完整路径）与 `project`（`project_id` 或完整路径）；`instance` 保留，在 R5 冻结前 MUST 拒绝。实例 URL 由操作者显式给出，MUST NOT 自动发现。
- project scope 的注册一律 `locked=true`，不得被分配到其它项目。注册 `access_level` 默认 `not_protected`；`ref_protected` MUST 由 Fleet 显式声明，Shaula MUST NOT 静默改变该值。
- `tags` 至少一个，除非 `run_untagged=true`；二者都为空的 Fleet MUST 在接受前拒绝，避免领取 scope 内全部未打标签的 job。
- 名字与所有权：每个注册的 `description` MUST 为 `{runner_name_prefix}-{generation}`，`maintenance_note` MUST 写入 `shaula:fleet={id};generation={id};incarnation={n}`。GitLab 不保证 description 唯一（实验中重复 description 被接受），因此所有权 MUST 由 description、`maintenance_note`、tags、`run_untagged`、runner type 与 scope 联合判定；任一不符即非本 Generation。
- Template 来源三选一与 TemplatePool 规则同 spec 0026 §2；每个成员 MUST 接纳 GitLab backend 与 Fleet 的执行器选择（§5）。

## 3. 控制面能力契约（adapter 最小集）

adapter 只依赖以下 GitLab 面；超时、无重定向、大小上限、失败分类同 spec 0026 §3：

| 用途 | 端点 | 关键事实 |
| --- | --- | --- |
| 预注册 | `POST /api/v4/user/runners`，body `{runner_type, group_id \| project_id, description, maintenance_note, tag_list, run_untagged, locked, paused}` → `201 {id, token, token_expires_at}` | 注册**立即存在**（`never_contacted`）；`glrt-` token 只在响应中出现一次 |
| 需求 | GraphQL `jobs(statuses: [PENDING], compatibleRunnerId: <anchor>) { count }` 与同查询内 `runner(id: <anchor>) { userPermissions { updateRunner } paused tagList runUntagged }` | 服务端按 runner 属性匹配；**无权时返回 0 而非错误** |
| 库存 | `GET /api/v4/{groups\|projects}/:id/runners?type=…`、`GET /api/v4/runners/:id` | `status`（`online`/`offline`/`stale`/`never_contacted`/`paused`；暂停的注册无论联系状态都报告 `paused`）、`job_execution_status`（`idle`/`active`）、`paused`、`maintenance_note`。`online` 表示最近 2 小时内有过联系，进程退出后仍会保持 `online` |
| 关联 / Busy | `GET /api/v4/runners/:id/jobs?status=running` | 返回该注册正在执行的 job，含 project、pipeline、job id |
| 暂停 | `PUT /api/v4/runners/:id {paused: true}` | 阻止后续领取，不中断进行中的 job |
| 解除注册 | `DELETE /api/v4/runners/:id` | 不中断进行中的 job；删除后该 job 的 `runner` 变为 `null` |

硬性要求：

- 需求查询 MUST 在同一 GraphQL 请求中证明 Anchor 存在、`userPermissions.updateRunner=true`、`paused=true` 且 tags / `run_untagged` 与 Fleet 当前 revision 一致。任一不满足时本轮需求为**不可证明**：保留上一份快照并记录 staleness，MUST NOT 解释为零需求。仅凭 runner 可见不足以证明（instance runner 对非管理员可见）。
- 未知 `status` / `job_execution_status` 值 MUST 按不可证明处理。
- 需求与库存是覆盖式快照；`count` 超过 Fleet 上限时只按上限使用，不分页拉取 job 明细作为容量依据。
- 每轮最多一次需求查询与一次库存分页遍历；interval、backoff、分页上限归 D7。

### 3.1 需求 Anchor

- 每个 Fleet revision 一个 Anchor：`description={runner_name_prefix}anchor-r{revision}`，`maintenance_note=shaula:fleet=<key>;role=anchor;name=<description>`，`paused=true`，属性与该 revision 相同。响应中的 token MUST 立即丢弃，MUST NOT 持久化、记录或交付给任何进程。
- Anchor 不是 Generation，不占容量，不参与 readiness。Anchor 的创建不确定性按 description + `maintenance_note` 精确查找分类：恰好一个则采用；`None` 则重建；`Multiple` 时不得任选其一，本轮需求保持 stale，全部删除后重建。因为 Anchor 从无 token 持有者，删除任何带本 Fleet anchor 标记的注册都不会中断任务。
- Anchor 缺失、被取消暂停或属性漂移时，Shaula MUST 先把需求标为 stale，再以新 Anchor 收敛并删除可证明属于本 Fleet 的旧 Anchor。被取消暂停的 Anchor 没有持有 token 的进程，不构成执行风险，但仍按漂移处理。
- Fleet Decommission MUST 删除 Anchor；删除失败保留重试，不阻塞 Generation 的清退证明。

## 4. Pool Fleet 生命周期差异

通用顺序、容量算术 `target = min(max_runners, min_runners + pending_compatible)` 与 idle deadline 同 spec 0026 §4。GitLab 差异：

1. 预注册：每个 Generation 一个独立注册，`paused=false`，属性与 Anchor 相同。
2. Terraform apply 仅一次；Runner Bootstrap Material 以受保护文件投递（§5）。
3. Readiness：注册 `status=online`、`job_execution_status=idle` 且 `paused=false` 视为在线；`never_contacted` 不是 readiness。Idle 的注册若变为暂停、`offline` 或 `stale`，即不能再服务，直接进入清退。容器 Running、Pod Ready、进程存活都不是 readiness。
4. 领取：`/runners/:id/jobs?status=running` 出现 job 时，Shaula MUST 在同一 tick 内持久化 `Verified` 关联（project id、job id、pipeline id、观测时间），随后 `PUT paused=true`。注册的 token 可被多个进程复用（实验中同一注册并发执行 2 个 job），暂停是阻止第二次领取的唯一服务端手段。
5. 终结证据：进程以 `--max-builds 1` 自然退出，**退出码不区分**“执行完一个 job”与“等待超时”（二者都为 0），不得作为证据。注册层证据为：已记录关联的 job 进入终态（`success`/`failed`/`canceled`）或注册的 running jobs 为空且 `job_execution_status=idle`（均在暂停之后观测）。adapter E2E（[证据](../evidence/gitlab-adapter-e2e-2026-10-13/README.md)）中，进程退出后第一次读取即为 `status=paused`、`job_execution_status=idle`（滞后 0 s）；实现仍 MUST 把 `active` 当作 Busy 重试，而不是假定无滞后。
6. 进程级保险：`--wait-timeout` MUST 设为不超过 Fleet idle deadline 的值，使未领取任务的进程自行退出；这只减少空转，不构成清退证据。

## 5. Runner Bootstrap Material、执行器与模板

- Runner Bootstrap Material 为 `{instance_url, runner_id, token, runner_name, executor}`。它以单个受保护的 `config.toml` 投递，进程以 `gitlab-runner run-single --config <path> --runner <runner_name> --max-builds 1 --wait-timeout <s>` 启动。token MUST NOT 出现于 argv、环境（含 `CI_SERVER_TOKEN`）、资源 tags/labels、Setup Info、Operation Log 或日志投影。
- gitlab-runner 进程日志打印 token 的 short token 子串（实验中为完整 token 的 9 字符子串）。runner stdout/stderr 属于 credential-adjacent：MUST NOT 进入 Operation Log 或 Setup Info，平台日志采集按凭据边界处理。
- 资源 MUST 禁止任何重启策略（Docker `--restart no`、Kubernetes `restartPolicy: Never`、systemd 不自动重启）：重启后的进程会以同一 token 领取第二个 job。
- 执行器：
  - 容器模板（Docker / Kubernetes）MUST 使用 `shell` executor。`kubernetes` executor 会在 Generation 之外创建 Pod（违反 [ARD-0006](../ard/0006-realize-each-kubernetes-runner-generation-as-one-pod-and-one-bootstrap-secret.md)），`docker` executor 需要宿主 socket（违反 Docker 默认不挂 socket 的基线），二者 MUST 在静态校验阶段拒绝。
  - `shell` executor 忽略 job 的 `image:` / `services:`；UI 与文档 MUST 明示，不得暗示与 GitLab 共享 runner 相同的语义。
  - VM 模板首个切片同样只接纳 `shell`；VM 内 `docker` executor 需要单独的模板契约与验收，不在本切片。
- 模板发布选项：manifest 的 `runner_backends` 增加 `gitlab`；Revision 的 backend 与 Fleet kind 一致性规则同 spec 0026 §5。
- 镜像准入：容器族 MUST 使用官方 `gitlab/gitlab-runner` 镜像并固定内容 digest，禁止自建或重打包。VM 下载官方 gitlab-runner 二进制并按 artifact 内固定版本与 SHA-256 验证。exact tuple 归 R5。
- VM 显式例外：可声明 `gitlab_vm_bootstrap_contract: shaula.gitlab-vm-cloud-init/v1`，允许 Runner token 进入受保护 tfvars、plan/state 与 cloud-init user-data，边界与 spec 0026 §5 的 Forgejo VM 例外相同；它 MUST 仅为 Shaula 预注册得到的单注册 token，MUST NOT 是 Auth Profile 的访问令牌。

## 6. 关联等级与清退

- GitLab Fleet 的 job ↔ Generation 关联为 `Verified`：证据是该 Generation 注册的 `/runners/:id/jobs?status=running` 或 `job.runner.id` 等于注册 id。删除注册后 `job.runner` 变为 `null`，因此关联 MUST 在删除前持久化；删除后读取到的 `null` MUST NOT 覆盖已记录的关联。
- 普通清退（未触发统一硬超时）的顺序：
  1. `PUT paused=true`；返回 404 视为注册已不存在，跳到第 4 步。
  2. 等待至少一个库存周期后复核：running jobs 为空且 `job_execution_status=idle` → 可继续；存在 running job、状态未知或证据矛盾 → 记为 Busy，保持暂停，MUST NOT 销毁资源。
  3. `DELETE` 注册；404 视为已收敛。删除不中断任务，因此不需要 Busy 前置；但第 2 步的复核 MUST 在删除之前完成，因为删除会抹掉关联查询入口。
  4. 模板 Destroy。注册层证据与资源层证据分别记录，不得互相代替。
- 统一硬超时：允许中断任务；先暂停注册并记录可见关联，再销毁已证明归属的资源，最后删除精确注册。
- 注册 Uncertain（响应丢失）：在 scope 库存内按 description **并联合** `maintenance_note`、tags、`run_untagged`、runner type 分类。`ExactlyOne` → 效果已落地，token 不可恢复，MUST 删除该注册并使 Generation 进入 `CleanupRequired`（与 [spec 0025](0025-jit-mint-uncertainty-recovery.md) 同构；不得使用 `reset_authentication_token` 补救，以免同一注册出现两代 token）；`None` → 零资源，可重试新的 Generation；`Multiple`、查找失败或删除失败 → `Quarantined`。

### 6.1 Jobs 只读投影

`/api/v1/jobs` 增加 `backend: gitlab` 记录：按 Fleet incarnation、instance URL、scope、Auth Profile 与 `project_id/job_id` 隔离；ID 用十进制字符串；携带 `gitlab` 元数据（project 路径、pipeline id、job 名称、`web_url`），不生成 `scale_set_id` 或 GitHub 文案。`generations` 只包含 §6 的 `Verified` 关联。状态映射：`pending` → `queued`、`running` → `running`，`success`/`failed`/`canceled`/`skipped` 只在读取到 job 自身终态时设置；其余与 spec 0026 §6.1 的 stale / `not_listed` 规则相同。Jobs 投影失败 MUST NOT 阻塞需求、readiness 或清退。

## 7. 认证与凭据

- 独立 profile kind `gitlab_token`，写入 `{kind: gitlab_token, instance_url, scope, token}`；MUST NOT 复用 GitHub App 或 `forgejo_token` 的 schema，MUST NOT 作为任何其它 provider 的 fallback。
- 最小权限：group scope 使用 Owner 角色的 group access token，project scope 使用 Maintainer 及以上的 project access token；scopes 为 `create_runner`、`manage_runner`、`read_api`（实验中以 group Owner 令牌完成注册、库存、需求、暂停与删除）。个人访问令牌可用但不推荐。
- 激活前 MUST 做一次有界只读探测：读取 scope 的 runners 列表与 GraphQL `currentUser`；失败即 Rejected。只读探测不证明写权限，首次 Anchor 创建失败按 Fleet 接受失败处理。
- 令牌的 `expires_at` MUST 读取并展示；距过期不足 Fleet 轮询预算时进入告警，过期后需求与库存为不可证明。
- 凭据轮换同 spec 0026 §6：不实现 Auth Handoff，轮换 = 新 Fleet revision + Generation 收敛；Anchor 用新凭据重建。

## 8. 明确排除

instance scope、VM 内 `docker` executor、`image:` / `services:` 语义、GitLab.com SaaS 的共享 runner、Runner Operator / fleeting autoscaler、Runner controllers、job webhook、按 job 选择 Template Profile、GitHub / Forgejo 行为变化。

## 9. 与 Pool driver 抽象的关系

本规范以 [ARD-0043](../ard/0043-admit-gitlab-as-the-second-pool-backend.md)（已修订）的共享 Pool 规则为前提。spec 0026 §9 的六条不变量对 GitLab 同样成立。共享与分立的边界：

- **共享**：容量算术、idle deadline、Retirement / 硬超时的选择、状态 phase（`pool_driver`）；准入、固定 pin、Generation 行、工作区准备与 apply 结果记录（`pool_create`）；清退状态机与证明要求（Runner Operation，经 `RunnerRegistrations` 接缝）。
- **按 backend 分立**：需求证明、库存身份、注册与不确定性分类、readiness / 领取。

GitLab 对这些接缝的额外要求：

1. 需求只有在 `GitlabAnchorDemand::proven_for` 给出值时才是证据，缺失时等同轮询失败。
2. 清退前置动作（暂停）在 `RunnerRegistrations` 内完成：暂停那一轮总是返回 Busy，复核至少推迟一轮；Forgejo 无此动作。
3. 关联等级由 backend 声明（Forgejo `Unverified`，GitLab `Verified`），driver 不得在 backend 未声明时提升等级。

## 10. Acceptance

验收记录“本地运行 / 真实实例运行”分离证据，不能以本规范条款代替：

- **G1 闭环**：真实 GitLab + group 令牌：Anchor 创建 → 需求 1 → 预注册 → `run-single` 以文件 token 启动 → readiness → 领取并记录 `Verified` → 暂停 → job 结束 → 删除注册 → 模板 Destroy。
- **G2 匹配**：两组不同 tags 的 job（含 untagged）只按 Anchor 属性计入需求；不匹配的 job 不被领取。
- **G3 需求快照**：权限撤销、Anchor 被删、Anchor 被取消暂停、轮询失败四种情况都保留旧快照并标 stale，不授权普通缩容。
- **G4 Busy-safe**：未到硬超时时，对执行中 Generation 发起普通 Destroy → 暂停后复核为 Busy，资源保留；job 结束后正常收敛。
- **G5 token 重用**：重启策略被模板校验拒绝；领取后暂停使同一 token 的第二个进程无法领取。
- **G6 注册 Uncertain**：注入响应丢失，`ExactlyOne` / `None` / `Multiple` 分别按 §6 收敛。
- **G7 关联**：Jobs 视图显示 `Verified` 关联，注册删除后关联保留。
- **G8 泄漏扫描**：argv、环境、资源 metadata、Setup Info、Operation Log 与普通日志中不含 token 或 short token；容器 Terraform 输入不含 token。
- **G9 无回归**：GitHub 与 Forgejo Fleet 的本地测试与既有真实路径证据在 Pool 抽象后复跑通过。
