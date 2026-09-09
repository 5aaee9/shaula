---
status: accepted
date: 2026-09-07
supersedes: 0002
amends: [0001, 0004, 0005, 0008, 0010, 0013]
---

# Run lifecycle workers with a database HTTP state backend

> [ARD-0024](0024-bootstrap-official-runner-images-outside-containers.md) adds a fixed host-side container bootstrap within the same Create after apply. It remains subject to execution admission/fencing and does not introduce Update, a second apply, or an implemented worker/HTTP-state composition claim.

Supersedes [ADR-0002](0002-run-immutable-runner-lifecycles-as-local-subprocesses.md); amends earlier orchestration/state ownership and ADR-0013's HTTP scope. The normative contract lives in [spec 0010](../specs/0010-lifecycle-worker-and-http-state-backend.md).

## Context

原设计让 daemon 持久化并调度每一条 Terraform operation，在 Runtime、store、recovery coordinator 之间重复表达一个本来顺序执行的生命周期。用户选择把每个 Generation 的完整生命周期交给独立 `shaula job`，保留 Workspace 恢复能力而非任意命令位置续跑；GitHub 控制面凭据仍由 daemon 保管。

用户进一步选择参考 `sdwan-terraform-backend`：Terraform 使用 HTTP backend 把 state 写入数据库，并以 LOCK/UNLOCK 加服务端事务保护并发。参考项目把 instance state、lock 和 executor 分开，但其实现允许无锁写、在事务外分离检查锁与写入，也会重建旧工作目录；这些行为不能直接作为 Shaula 的 CAS/恢复保证。

## Decision

- 一个 Generation 的 Create → 等待 GitHub 安全删除 → Destroy → 清理由一个 Lifecycle Worker 顺序执行。worker 在独占目录中 CoW/copy frozen Template；不可变 Generation 和 Create/Destroy-only 原则不变。
- daemon 只拥有 Fleet/session/GitHub、capacity/admission、worker supervision 和外部安全 gates，不镜像每个 Terraform 内部阶段。持久化必要的 Worker Claim、可能已开始的副作用、exact inputs/ownership、state/lock 与 terminal facts，不构建新的逐命令 durable operation ledger。
- 建立小的 Executor Interface：launch、observe、stop/fence worker 及 descendants。v1 只实现本地主机 `exec` Driver；Kubernetes Job 是未来扩展，不在本次交付中引入远程 driver 或 Kubernetes client。
- SQLite 经 daemon 的内部 Terraform HTTP backend 保存 authoritative state；LOCK/UNLOCK 和所有 state writes 在同一事务 discipline 中校验 current worker、锁持有者及版本。锁不自动过期接管，旧进程可能仍能执行 provider side effect 时保持隔离。
- 独立 loopback worker listener 使用 Generation/worker-scoped capabilities；控制权限与 state 权限分开。Terraform 通过专用 Basic password env 使用 state backend；管理 UI/API/health 继续 OIDC-only，不恢复 legacy 管理 backend token。
- GitHub credential 不下发 worker。worker 请求 daemon 获取 JIT、观察 Runner 和安全移除；Busy、unknown ownership、missing Scale Set 等仍 fail closed。worker exit 不等于 Destroyed。
- 保留 exact artifacts/inputs、database state 及尚未上传的 emergency local state。恢复证明旧进程停止后执行观察/cleanup-only 路径；不承诺 Create 的逐指令 resume，不在 uncertainty 下重新 apply Create。
- 空 state completion 由 daemon 绑定当前 worker/state version 原子确认并 seal，再释放 Occupancy、允许本地 cleanup。SQLite/retained materials/emergency state 构成新的备份一致性集合。

Wire methods、credential scopes、Create-start handover、事务/replay 规则、故障恢复与验收只在 spec 0010 维护。Template admission 与 saved-plan policy 继续由 spec 0004 定义；该拆分不是放宽 Terraform/provider 信任或把平台业务逻辑移入 core。

## Alternatives considered

- **继续补强 daemon-owned operation ledger**：拒绝作为目标架构。它可记录很多中间状态，但增加重复状态和跨层恢复组合；现有实现可作为迁移前基线，不再扩展成新的规范真相。
- **worker 完全无持久化、本地 state 任意丢弃**：拒绝。进程退出无法证明资源不存在；缺失 state 不能安全 Destroy 或释放容量。
- **只加 LOCK/UNLOCK routes、state 写入另行 UPDATE**：拒绝。锁检查与写入存在竞态，且无锁写无法限制 stale worker。
- **立刻实现 Kubernetes Job executor**：推迟。先验证 exec seam；远程 executor 的网络、制品、state 和 descendant fencing 是独立交付。
- **给 worker 原始 GitHub credential 或依赖外部 OIDC machine client**：不采用。前者扩大暴露，后者引入额外 Provider 注册和刷新依赖；内部 capability 可精确绑定已有 Generation/worker authority，不改变管理 OIDC。

## Consequences

- Lifecycle 顺序下沉到一个 worker，daemon/store 不需要了解每条 init/plan/inspect 命令；增加了小的内部协议和 exec supervision 边界。
- HTTP backend 依赖 daemon/SQLite 可用性。shutdown 必须保留 backend 到 worker 完成最后写入；backend outage 后的 emergency state 属于必须保留的故障证据。
- State lock 不是 infrastructure fence，也不是任意 HTTP client 的 If-Match CAS；host/process containment 验收仍不可省略。
- State/JIT/provider material 仍为 credential-grade，不能因从文件搬入数据库而出现在管理 reads、日志或一般备份中。
- 同 OS identity 的 ambient host-admin 风险不因子进程化自动降低；root/privileged Docker-capable IaC child 仍在同一受信宿主域内。
- 需要显式 local-state/ledger migration 与 crash tests。当前实现进度只见 [implementation status](../IMPLEMENTATION_STATUS.md)；接受本 ADR 不代表新架构已实现。
