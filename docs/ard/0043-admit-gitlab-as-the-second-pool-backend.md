---
status: proposed
date: 2026-10-13
relates_to: ["0030", "0031", "0033"]
---

# Admit GitLab as the second pool backend

决定以 [ARD-0033](0033-admit-forgejo-through-a-pool-backend-first.md) 的 Pool 模型接入 GitLab（自托管
GitLab CE/EE 与 gitlab-runner），且**不复制第三份 Pool 规则**：Forgejo driver 中与 provider 无关的
Pool 规则抽为共享模块，Forgejo 与 GitLab 各自只保留结构上不同的部分；GitHub Scale Set 路径不变
（Decision 1 于实现时修订，见下）。
协议与验收统一维护在 [spec 0043](../specs/0043-gitlab-runner-backend.md)；本文记录理由与代价。

## Context

2026-10-13 在一次性 GitLab CE 19.4.2 + gitlab-runner 19.4.1 上做了协议实验
（[脚本](../../scripts/gitlab-pool-spike/README.md)、[证据](../evidence/gitlab-pool-spike-2026-10-13/README.md)）。
与 Forgejo 相比，GitLab 有四处**更好**、三处**更差**、两处**只是不同**：

更好：

- **服务端需求匹配**：GraphQL `jobs(statuses: [PENDING], compatibleRunnerId:)` 由 GitLab 自己按 tags、
  `run_untagged`、group/project 归属计算某个 runner 可领取的等待 job；paused、从未连接的 runner 也有效，
  group Owner 访问令牌即可调用。Shaula 不必在客户端复刻 GitLab 的匹配规则，也不必逐项目轮询。
- **可验证关联**：job 被领取后 `job.runner.id` 与 `/runners/:id/jobs?status=running` 直接给出
  job ↔ 注册 id，Generation 可得到 `Verified` 关联（Forgejo 只能 `Unverified`）。
- **注册移除不中断任务**：删除正在执行 job 的注册返回 204，job 用自身 job token 继续并成功结束；
  `paused=true` 阻止后续领取，也不中断进行中的 job，且保留关联可查。暂停是可用的 drain 原语。
- **注册带可回读的所有权标记**：`maintenance_note` 可写入 Generation 身份并原样回读。

更差：

- **授权失败会伪装成零需求**：对无权管理的 runner 调用 `compatibleRunnerId` 不报错而返回 `count: 0`；
  instance runner 对非管理员仍可见。必须在同一查询中证明 `userPermissions.updateRunner`，否则零需求不可信。
- **token 不是一次性的**：同一 `glrt-` token 可被多个进程同时使用，一个注册实测并发执行了 2 个 job；
  服务端不强制 ephemeral，进程退出后注册仍在。一 Generation ↔ 一 job 必须由 Shaula 保证。
- **退出码不可区分**：`run-single --max-builds 1` 跑完 job 与 `--wait-timeout` 到期都退出 0。
- **删除抹掉关联**：注册删除后 `job.runner` 变为 `null`，关联必须在删除前落库。

只是不同：

- 引导：`gitlab-runner run-single --config <file> --runner <name> --max-builds 1 --wait-timeout <s>`，
  token 只在受保护的 config 文件中，不进 argv/env；runner 日志会打印 token 的 9 字符子串（short token）。
- 执行器：容器内只能用 `shell` executor；`kubernetes` executor 会在 Generation 之外自建 Pod，
  `docker` executor 需要宿主 socket，均与既有边界（ARD-0006、Docker 不挂 socket）冲突。

## Decision

1. **GitLab 走 Pool 模型，与 Forgejo 共享 Pool 规则，tick 编排按 backend 分立。**
   共享且只维护一份：容量算术、idle deadline、Retirement / 硬超时选择与状态 phase（`pool_driver`）；
   准入、固定 pin、Generation 行、工作区准备与 apply 结果记录（`pool_create`）；清退状态机、证明要求与
   Quarantine（Runner Operation，backend 差异经 `RunnerRegistrations` 接缝）。按 backend 分立：需求证明、
   库存身份、注册与不确定性分类、readiness / 领取。Forgejo 行为保持不变，以现有测试证明。
   `GitHubAccessPort`、`shaula-scaleset` 与 GitHub driver 不参与本次抽象。

   *修订（2026-10-13，实现时）*：原决定是“一条 provider-neutral 的 Pool driver”。实现中发现，分立的这
   四部分在 GitLab 上有结构性差异，不是参数差异：需求来自一个必须自行收敛的 Anchor，而不是 job 列表；
   身份是 ID + description + note，而不是 ID + UUID + name；领取后要先写关联再暂停；完成以后注册仍在。
   把它们塞进一个 driver 端口，端口会变成两套实现的并集。因此只共享语义相同的规则，并把共享部分的代码
   真正抽出（Forgejo 也已改用），避免“复制第三条 driver”中被拒绝的那种三份维护。
2. **需求来自 Fleet 自有的 paused Anchor 注册。** Fleet 接受时创建一个与 Fleet tags / `run_untagged` /
   scope 相同、`paused=true` 的注册，token 立即丢弃（从不持久化、从不交给任何进程），只作为
   `compatibleRunnerId` 的匹配模板。每次轮询同时证明 Anchor 存在、可管理且属性未漂移；任一失败即 stale。
3. **一 Generation ↔ 一注册 ↔ 一 job 由本地纪律保证**：每个注册只交付给一个 `run-single --max-builds 1`
   进程，禁止重启策略；观测到 job 领取后立即记录 `Verified` 关联并暂停注册。
4. **普通清退顺序为“暂停 → 复核 → 删除注册 → 销毁资源”。** 注册删除本身不中断任务，Busy-safe 只约束资源 Destroy；
   复核以暂停后的 `/runners/:id/jobs?status=running` 与 `job_execution_status` 为证据。
5. **凭据是独立 profile kind `gitlab_token`**：group / project 访问令牌（`create_runner`、`manage_runner`、
   `read_api`），instance scope 需管理员令牌；与 GitHub App、Forgejo token 不共用 schema。
6. **首个切片只交付 group / project scope、容器模板的 `shell` executor 与 VM 模板。** instance scope、
   VM 内 `docker` executor 与 `image:` 语义另行验收。

## Alternatives considered

- **复制 Forgejo driver 为第三条 driver**：最快，但会把容量算术、idle deadline、uncertain 分类维护三份；
  ARD-0033 已把“两条 driver”列为需要归一的代价，第三个 provider 正是归一的时机。拒绝。
- **单一 provider-neutral Pool driver + 宽端口**（原 Decision 1）：端口需同时表达 Anchor 收敛、两种身份
  模型、关联后暂停与“完成后注册仍在”，等于两套实现的并集，Forgejo 也要为不存在的步骤提供空实现。
  实现时改为“共享规则 + 分立编排”。
- **逐项目 REST `jobs?scope[]=pending` + 客户端 tag 匹配**：不需要 Anchor，但 group scope 需遍历全部子项目
  （GraphQL 限制每次只能查 1 个项目的 jobs），且要复刻 tags、`run_untagged`、protected ref、locked 等规则，
  规则漂移即误判。仅作为 Anchor 不可用时的诊断手段，不作为需求来源。
- **管理员 GraphQL 全局 `jobs(statuses: PENDING)`**：需要实例管理员，并且仍要客户端匹配。拒绝。
- **GitLab 官方 autoscaling（fleeting / docker-autoscaler）或 Runner Operator**：由常驻 runner manager 自行扩缩，
  与“每个 Generation 一份不可变基础设施”的模型重叠；Shaula 只会变成它的配置器。拒绝。
- **Runner controllers（18.9 Experiment，Ultimate）**：只做 admission control，不提供需求信号。不采用。
- **Job webhook**：需要公开入站端点，违反 ARD-0011 的回环 + 认证代理边界。拒绝。

## Consequences

- 共享模块先由 Forgejo 使用，并以既有测试证明零行为变化；GitLab 编排只建立在这些模块之上。
- 仍有两份 tick 编排（Forgejo、GitLab）。改动容量、清退或 phase 规则时只需改共享模块；改动编排顺序时
  两边都要审查。
- Anchor 是一个长期存在、从不连接的注册：会出现在 GitLab runner 列表中，可能被实例的
  stale-runner 清理策略删除；删除后 Fleet 需求进入 stale 并由 Shaula 重建，不得把缺失当零需求。
- 容器模板的 `shell` executor 忽略 `.gitlab-ci.yml` 的 `image:`；这是 GitLab 用户可见的语义差异，
  UI 与文档必须明示。
- runner 进程日志含 short token，属于 credential-adjacent 输出，不得进入 Operation Log 或 Setup Info。
- 与同 scope 内其它带相同 tags 的 runner 共享需求时，需求与容量不再一一对应，代价与 Forgejo 相同。
- `compatibleRunnerId` 的最低 GitLab 版本、分页与大实例查询成本尚未核验，列为 R5 / D7。

## Rollout

1. spec 0043 接受，本 ARD 转 `accepted`；冻结 R5 / D7 的最低版本与轮询预算。
2. 抽出共享的 `pool_create` / `pool_driver` 模块，Forgejo 改用它们，现有 Forgejo 测试与 A1–A8 证据复跑无回归。
3. 实现 `shaula-gitlab` adapter、`gitlab_token` profile、Fleet `gitlab` 段、模板 `runner_backends: gitlab`。
4. spec 0043 §10 的 G1–G9 在真实 GitLab 实例上验收，证据写入实现状态。
