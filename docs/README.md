# Shaula 文档索引与决策状态

本目录区分**目标契约、架构决定、实现进度、发布证据**。规范中的 MUST 是目标要求，不是实现或验收通过的声明。

## 权威来源

| 内容 | 唯一维护位置 |
| --- | --- |
| 领域术语 | [CONTEXT.md](../CONTEXT.md)；只定义术语 |
| 产品范围、全局安全不变量 | [spec 0001](specs/0001-shaula-runner-scale-set.md) |
| Fleet admission、replacement、Auth Handoff、Decommission | [spec 0002](specs/0002-fleet-http-control-plane.md) |
| Kubernetes / Docker Runner Resource 差异与外部验收 | [spec 0003](specs/0003-kubernetes-runner-resource.md) / [spec 0006](specs/0006-docker-runner-resource.md) |
| Template materialization、inputs/outputs、Terraform plan policy | [spec 0004](specs/0004-template-profile-runtime.md) |
| Profile publication、retirement、sensitive reads、attestation | [spec 0005](specs/0005-profile-http-control-plane.md) |
| Rust crate ownership 与依赖方向 | [spec 0007](specs/0007-rust-workspace-architecture.md) |
| UI 行为 | [spec 0008](specs/0008-embedded-web-ui.md) |
| Fleet Template inputs 可视化编辑 | [spec 0014](specs/0014-visual-template-inputs.md) / [ADR-0018](ard/0018-render-fleet-inputs-from-approved-template-options.md) |
| 数据库模板库、默认文件导入与 Terraform 变量发现 | [spec 0015](specs/0015-template-library-and-variable-discovery.md) / [ARD-0019](ard/0019-store-template-sources-and-discover-terraform-variables.md) |
| Fleet authentication/template Profile 服务端列表选择 | [spec 0016](specs/0016-fleet-profile-selection.md) / [ARD-0020](ard/0020-load-fleet-profile-choices-from-registry.md) |
| Template 静态校验后的自动激活与旧 Ready 升级 | [spec 0017](specs/0017-automatic-template-activation.md) / [ARD-0021](ard/0021-activate-templates-after-static-validation.md) |
| GitHub authentication 连接列表、搜索、详情发现 | [spec 0012](specs/0012-github-authentication-inventory.md) / [ADR-0016](ard/0016-list-authentication-connections-from-the-profile-registry.md) |
| 管理 HTTP 的 OIDC、session、CSRF | [spec 0009](specs/0009-mandatory-openid-connect.md) |
| Browser session 到期后的 Provider 续期、页面保留 | [spec 0013](specs/0013-provider-backed-browser-session-renewal.md) / [ADR-0017](ard/0017-renew-browser-sessions-in-the-authentication-guard.md) |
| Worker/Executor、内部 control/state HTTP、locks/CAS、恢复与备份 | [spec 0010](specs/0010-lifecycle-worker-and-http-state-backend.md)；理由见 [ADR-0014](ard/0014-run-lifecycle-workers-with-a-database-http-state-backend.md) |
| 多账户 GitHub App authentication、动态仓库 selector、installation routing | [spec 0011](specs/0011-multi-account-github-authentication.md) / [ADR-0015](ard/0015-route-one-github-app-profile-to-multiple-accounts.md)；已有本地实现，运行时集成边界见 implementation status，真实 GitHub 路由验收与生产迁移未执行 |
| 已实现、未接线、未验证与迁移缺口 | [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) |
| OIDC 部署操作 | [oidc-deployment.md](oidc-deployment.md) |
| Nix 开发、打包、NixOS 部署与 VM 测试 | [nix.md](nix.md) |

ARD 保存选择的理由、代价与历史；详细协议在其引用的 spec 中维护。已 superseded 的 ARD 不是当前实现选项。通用规则由所属 spec 定义，平台文档只增加差异和验收，不维护另一套通用状态机。

## 已确定的基线

- 一个 daemon 管理多个 Fleet；每个 Generation 由独立 `shaula job` Lifecycle Worker 执行完整生命周期。v1 只有 `exec` Executor Driver，未来 Kubernetes Job executor 不等于 Kubernetes Runner Resource。
- Terraform state 通过 daemon 内部 HTTP backend 写入 SQLite；LOCK/UNLOCK、锁持有者校验和 state 写入是数据库事务契约，不以本地 `terraform.tfstate` 为主状态。
- daemon 保管 GitHub 控制面凭据，worker 通过受授权控制通道请求 JIT、观察与安全删除；管理 HTTP 保持 OIDC，内部 worker/state HTTP 使用分权的 Generation/worker 专用凭据。
- Runner Generation 不可变；Create 与 Destroy 是唯一基础设施 mutation，Busy-safe removal、原始 inputs/artifact、worker fencing 和故障时保留证据不因进程拆分而取消。
- Kubernetes 与 Docker 都是 v1 bundled Template Platforms；GitHub App 与 PAT 都受支持，不做运行时 credential fallback。
- Fleet Decommission 保留空 Scale Set；Profile DELETE 是异步 retirement，不因正在使用而改成同步删除或 force delete。
- Template 当前候选静态校验通过后自动激活，已有 Ready 在扫描时重新校验并激活；独立 `template.attest` 的 exact conformance 记录作为运行验证证据保留，不再控制激活，见 spec 0017。
- 默认 Docker Runner 不挂载 host socket；JIT 同 Runner Execution Domain 的进程检查风险、Kubernetes name-based deletion 风险和同 OS identity IaC children 的 ambient host-admin 风险按相应 ARD 记录。

以上是设计基线；实现差距必须留在 implementation status，不能通过修改此表宣称已完成。

## 仍需决定或冻结

各 spec 的 Open decisions 只链接此表，不再重复维护问题。

| ID | 类型 | 尚缺事实或选择 | 所属契约 |
| --- | --- | --- | --- |
| D1 | 产品支持范围 | PAT classic、fine-grained 或两者的正式支持矩阵；App/PAT × organization/repository 的真实 GitHub 验收 | 0001 §6.3 |
| D2 | 运行策略 | Changes、幂等记录、audit、tombstones、retired credentials、artifacts、state snapshots、emergency state 与 Workspace 的 retention 时限；原始凭据的外部撤销时机 | 0005 §7 |
| D3 | 运行策略 | operation/recovery timeout、retry budget、reaper interval、worker/backend body/rate/backlog/concurrency 的最终默认值与硬上限；OIDC 已有具体值见部署说明，不重新标为待定 | 0001 §5 / 0009 |
| D4 | 持久格式，阻塞发布冻结 | `bindings_digest` 是否继续作为独立 commitment，以及 exact Revision/incarnation 绑定、编码和兼容迁移；本轮不新增 bd2/HMAC 格式，不重写旧记录 | 0004 §3 / 0005 §5 |
| R1 | 发布配置与验收 | Rust toolchain/features、Terraform binary、provider locks/checksums、Runner/init/shim images、runtime/trust policy 与 conformance suite 的 exact tuple | 0003 / 0004 / 0006 / 0007 |
| R2 | 协议验收 | Go oracle 的 commit/module/checksum、获取方式和完整 differential suite；真实已注册 OIDC Provider 的 browser/API 验收 | 0007 §4 / 0009 §7 |
| R3 | 平台验收 | Kubernetes CPU/memory/ephemeral-storage、安全上下文、seccomp/capabilities、namespace sharing/network policy 和所需 RBAC；host OS 的 worker/descendant fencing | 0003 §10 |

代码或 manifest 已选择某个库/字段，不自动等于其兼容性验收通过；已有实现也不应继续作为“完全未实现”记录。

## 多账户认证与后续扩展

多账户 GitHub authentication 已按 [spec 0011](specs/0011-multi-account-github-authentication.md) 与 [ADR-0015](ard/0015-route-one-github-app-profile-to-multiple-accounts.md) 接受并完成本地实现：一份 App credential、多个明确账户/Target selector、个人未来仓库的按需验证。它替代单 installation、同 key policy 不可变的旧基线条款；真实 GitHub 路由验收与生产迁移仍未执行，当前能力边界以 [implementation status](IMPLEMENTATION_STATUS.md) 为准。

以下需要新的决定和对应验收，不阻塞按现有基线实现：额外 high-trust Runner-socket Profile、Docker memory-only JIT、pre-JIT provider-backed namespace preflight、每 Profile 独立 OS identity/sandbox、OpenTofu advertisement、远程 Executor Driver、多主/HA、自动删除 Scale Set、Quarantine force-recovery。它们不能作为匿名认证、跳过 locking 或丢弃可能残留资源的理由。

## 维护与验收

- 修改规则时更新其唯一 owner、受影响 ARD 和实现状态；其他 spec 链接 owner，不能留下相反的 MUST。
- 一条“已实现”记录应给出源代码/测试入口；“本地测试存在”“本地运行通过”“真实服务验收通过”分别记录。
- 文档检查覆盖链接、旧契约残留和场景要求；它不替代 cargo、fault injection、真实 Terraform/GitHub/平台验收。
