# Shaula 文档

项目用途与功能介绍见 [项目首页](../README.md)。本目录集中维护使用、开发、设计和验证说明。

## 使用与开发

| 内容 | 指南 |
| --- | --- |
| Nix 构建环境、打包与 NixOS 服务部署 | [Nix / NixOS](nix.md) |
| 登录、HTTPS、API 身份与访问授权 | [OIDC 部署](oidc-deployment.md) |
| Jobs 状态、apply / destroy 日志与保留策略 | [Jobs 与执行日志](jobs-and-operation-logs.md) |
| 从源码构建、开发 Web UI 与运行检查 | [开发指南](development.md) |
| 官方 Runner 镜像、JIT 输入与版本升级 | [Runner 镜像](runner-image.md) |
| 启用 GitHub Set up job 中的 apply 输出 | [Setup Info 模板](setup-info-templates.md) |
| 真实 Docker Runner 的创建、运行与清理验证 | [Docker 冒烟验证](docker-conformance.md) |
| Proxmox 基础 VM、DHCP 与 cloud-init 配置 | [Proxmox Runner](proxmox-runners.md) |
| 已实现范围、集成缺口与验收证据 | [实现状态](IMPLEMENTATION_STATUS.md) |

## 设计与决策

文档区分**目标契约、架构决定、实现进度、发布证据**。规范中的 MUST 是目标要求，不是实现或验收通过的声明。

### 权威来源

| 内容 | 唯一维护位置 |
| --- | --- |
| 领域术语 | [CONTEXT.md](CONTEXT.md)；只定义术语 |
| 产品范围、全局安全不变量 | [spec 0001](specs/0001-shaula-runner-scale-set.md) |
| Fleet admission、replacement、Auth Handoff、Decommission | [spec 0002](specs/0002-fleet-http-control-plane.md) |
| 已有 Fleet labels 更新与 owned Scale Set 收敛 | [spec 0002 §6.1](specs/0002-fleet-http-control-plane.md#61-mutable-scale-set-labels) / [ARD-0027](ard/0027-reconcile-labels-on-owned-scale-sets.md) |
| Kubernetes / Docker Runner Resource 差异与外部验收 | [spec 0003](specs/0003-kubernetes-runner-resource.md) / [spec 0006](specs/0006-docker-runner-resource.md) |
| Template materialization、inputs/outputs、Terraform plan policy | [spec 0004](specs/0004-template-profile-runtime.md) |
| Profile publication、retirement、sensitive reads、attestation | [spec 0005](specs/0005-profile-http-control-plane.md) |
| Rust crate ownership 与依赖方向 | [spec 0007](specs/0007-rust-workspace-architecture.md) |
| UI 行为 | [spec 0008](specs/0008-embedded-web-ui.md) |
| Workflow Jobs、Apply/Destroy 日志保留与 Setup Info 内容 | [spec 0019](specs/0019-workflow-jobs-and-operation-logs.md) / [ARD-0023](ard/0023-retain-operation-logs-and-present-workflow-jobs.md)；已有本地实现，真实平台验收边界见 implementation status |
| 官方容器镜像、宿主 bootstrap 与启动门槛 | [spec 0020](specs/0020-official-container-runner-bootstrap.md) / [ARD-0024](ard/0024-bootstrap-official-runner-images-outside-containers.md) |
| Fleet Template inputs 可视化编辑 | [spec 0014](specs/0014-visual-template-inputs.md) / [ADR-0018](ard/0018-render-fleet-inputs-from-approved-template-options.md) |
| 数据库模板库、默认文件导入与 Terraform 变量发现 | [spec 0015](specs/0015-template-library-and-variable-discovery.md) / [ARD-0019](ard/0019-store-template-sources-and-discover-terraform-variables.md) |
| 内置模板同步与已发布模板 Update | [spec 0021](specs/0021-default-template-updates.md) / [ARD-0025](ard/0025-sync-default-templates-and-explicitly-update-published-revisions.md) |
| Proxmox VM clone、NoCloud 和基础镜像信任 | [spec 0022](specs/0022-proxmox-runner-template.md) / [ARD-0026](ard/0026-provision-proxmox-runners-with-nocloud.md) |
| Fleet 跟随最新 Active 模板 Revision（follow-only 引用与 level-triggered 级联升级） | [spec 0023](specs/0023-fleet-template-follow-latest.md) / [ARD-0028](ard/0028-follow-latest-active-template-revision.md) / [ARD-0029](ard/0029-drop-pinned-template-revisions.md) |
| Fleet authentication/template Profile 服务端列表选择 | [spec 0016](specs/0016-fleet-profile-selection.md) / [ARD-0020](ard/0020-load-fleet-profile-choices-from-registry.md) |
| Template 静态校验后的自动激活与旧 Ready 升级 | [spec 0017](specs/0017-automatic-template-activation.md) / [ARD-0021](ard/0021-activate-templates-after-static-validation.md) |
| 仅支持 v2 GitHub App authentication、历史格式停用与部署检查 | [spec 0018](specs/0018-github-app-only-authentication.md) / [ARD-0022](ard/0022-retire-legacy-github-authentication.md) |
| GitHub authentication 连接列表、搜索、详情发现 | [spec 0012](specs/0012-github-authentication-inventory.md) / [ADR-0016](ard/0016-list-authentication-connections-from-the-profile-registry.md) |
| 管理 HTTP 的 OIDC、session、CSRF | [spec 0009](specs/0009-mandatory-openid-connect.md) |
| Browser session 到期后的 Provider 续期、页面保留 | [spec 0013](specs/0013-provider-backed-browser-session-renewal.md) / [ADR-0017](ard/0017-renew-browser-sessions-in-the-authentication-guard.md) |
| Worker/Executor、内部 control/state HTTP、locks/CAS、恢复与备份 | [spec 0010](specs/0010-lifecycle-worker-and-http-state-backend.md)；理由见 [ADR-0014](ard/0014-run-lifecycle-workers-with-a-database-http-state-backend.md) |
| 多账户 GitHub App authentication、动态仓库 selector、installation routing | [spec 0011](specs/0011-multi-account-github-authentication.md) / [ADR-0015](ard/0015-route-one-github-app-profile-to-multiple-accounts.md)；已有本地实现，运行时集成边界见 implementation status，真实 GitHub 路由验收与生产迁移未执行 |

ARD 保存选择的理由、代价与历史；详细协议在其引用的 spec 中维护。已 superseded 的 ARD 不是当前实现选项。通用规则由所属 spec 定义，平台文档只增加差异和验收，不维护另一套通用状态机。

### 已确定的基线

- 一个 daemon 管理多个 Fleet；每个 Generation 由独立 `shaula job` Lifecycle Worker 执行完整生命周期。v1 只有 `exec` Executor Driver，未来 Kubernetes Job executor 不等于 Kubernetes Runner Resource。
- Terraform state 通过 daemon 内部 HTTP backend 写入 SQLite；LOCK/UNLOCK、锁持有者校验和 state 写入是数据库事务契约，不以本地 `terraform.tfstate` 为主状态。
- daemon 保管 GitHub 控制面凭据，worker 通过受授权控制通道请求 JIT、观察与安全删除；管理 HTTP 保持 OIDC，内部 worker/state HTTP 使用分权的 Generation/worker 专用凭据。
- Runner Generation 不可变；Create 与 Destroy 是唯一基础设施 mutation，Busy-safe removal、原始 inputs/artifact、worker fencing 和故障时保留证据不因进程拆分而取消。
- Kubernetes、Docker 与 Proxmox 是 bundled Template Platforms；GitHub authentication 只支持 schema 2 GitHub App、显式 TargetPolicy 和 Revision-scoped account bindings，不做运行时 credential fallback。PAT、旧 allowlist 和固定 installation publication 已按 spec 0018 停用。
- Fleet Decommission 保留空 Scale Set；Profile DELETE 是异步 retirement，不因正在使用而改成同步删除或 force delete。
- Template 当前候选静态校验通过后自动激活，已有 Ready 在扫描时重新校验并激活；独立 `template.attest` 的 exact conformance 记录作为运行验证证据保留，不再控制激活，见 spec 0017。
- 默认 Docker Runner 不挂载 host socket；JIT 同 Runner Execution Domain 的进程检查风险、Kubernetes name-based deletion 风险和同 OS identity IaC children 的 ambient host-admin 风险按相应 ARD 记录。
- Jobs 以已观测 workflow job 为主；Apply/Destroy 日志属于 Generation 的各次执行尝试，销毁后仍按独立策略保留。容器直接使用官方镜像；Apply 的安全投影由宿主生命周期执行端在 apply 完成后生成，并在开启 Listener 启动门槛前交付，见 specs 0019/0020。

以上是设计基线；实现差距必须留在 implementation status，不能通过修改此表宣称已完成。

### 仍需决定或冻结

各 spec 的 Open decisions 只链接此表，不再重复维护问题。

| ID | 类型 | 尚缺事实或选择 | 所属契约 |
| --- | --- | --- | --- |
| D1 | 协议验收 | GitHub App × organization/repository 的真实 GitHub 验收，包含多 org 和个人动态仓库路由；PAT 支持已由 spec 0018 明确取消，不再待决定 | 0001 §6.3 / 0011 / 0018 |
| D2 | 运行策略 | Changes、幂等记录、audit、tombstones、retired credentials、artifacts、state snapshots、emergency state 与 Workspace 的 retention 时限；原始凭据的外部撤销时机。Operation Log/Jobs 历史的独立默认值已由 spec 0019 冻结，不扩展为上述恢复材料的 GC 规则 | 0005 §7 / 0019 §5 |
| D3 | 运行策略 | operation/recovery timeout、retry budget、reaper interval、worker/backend body/rate/backlog/concurrency 的最终默认值与硬上限；OIDC 已有具体值见部署说明，Operation Log/Setup Info 的默认值见 spec 0019，不重新标为待定 | 0001 §5 / 0009 / 0019 |
| D4 | 持久格式，阻塞发布冻结 | `bindings_digest` 是否继续作为独立 commitment，以及 exact Revision/incarnation 绑定、编码和兼容迁移；本轮不新增 bd2/HMAC 格式，不重写旧记录 | 0004 §3 / 0005 §5 |
| R1 | 发布配置与验收 | Rust toolchain/features、Terraform binary、provider locks/checksums、官方 Runner image/宿主 bootstrap CLI、runtime/trust policy 与 conformance suite 的 exact tuple | 0003 / 0004 / 0006 / 0007 |
| R2 | 协议验收 | Go oracle 的 commit/module/checksum、获取方式和完整 differential suite；真实已注册 OIDC Provider 的 browser/API 验收 | 0007 §4 / 0009 §7 |
| R3 | 平台验收 | Kubernetes CPU/memory/ephemeral-storage、安全上下文、seccomp/capabilities、namespace sharing/network policy 和所需 RBAC；host OS 的 worker/descendant fencing | 0003 §10 |

代码或 manifest 已选择某个库/字段，不自动等于其兼容性验收通过；已有实现也不应继续作为“完全未实现”记录。

### 多账户认证与后续扩展

多账户 GitHub authentication 已按 [spec 0011](specs/0011-multi-account-github-authentication.md) 与 [ADR-0015](ard/0015-route-one-github-app-profile-to-multiple-accounts.md) 接受并完成本地实现：一份 App credential、多个明确账户/Target selector、个人未来仓库的按需验证。它替代单 installation、同 key policy 不可变的旧基线条款。[spec 0018](specs/0018-github-app-only-authentication.md) 进一步取消 PAT、v1 publication/replay、旧格式升级和 reference-only execution。历史 rows/credential bytes 保留且不自动转换；部署前必须确认没有仍依赖旧格式的 active/desired 或 retained execution 引用。真实验收与部署证据以 [implementation status](IMPLEMENTATION_STATUS.md) 为准。

以下需要新的决定和对应验收，不阻塞按现有基线实现：额外 high-trust Runner-socket Profile、Docker memory-only JIT、pre-JIT provider-backed namespace preflight、每 Profile 独立 OS identity/sandbox、OpenTofu advertisement、远程 Executor Driver、多主/HA、自动删除 Scale Set、Quarantine force-recovery。它们不能作为匿名认证、跳过 locking 或丢弃可能残留资源的理由。

## 维护与验收

- 修改规则时更新其唯一 owner、受影响 ARD 和实现状态；其他 spec 链接 owner，不能留下相反的 MUST。
- 一条“已实现”记录应给出源代码/测试入口；“本地测试存在”“本地运行通过”“真实服务验收通过”分别记录。
- 文档检查覆盖链接、旧契约残留和场景要求；它不替代 cargo、fault injection、真实 Terraform/GitHub/平台验收。
