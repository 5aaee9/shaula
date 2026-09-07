---
status: superseded
date: 2026-09-04
superseded-by: 0014
---

# Run immutable Runner lifecycles as local subprocesses

> Historical decision, superseded by [ADR-0014](0014-run-lifecycle-workers-with-a-database-http-state-backend.md). Immutable Generations and Create/Destroy-only remain; daemon-owned command orchestration, local-state authority and the permanent prohibition on future remote Executors do not. Current worker/state protocol: [spec 0010](../specs/0010-lifecycle-worker-and-http-state-backend.md).

Shaula 在 daemon 主机上直接以 subprocess 运行受支持的 IaC engine，并为每个 Runner Generation 冻结 Template Profile Revision、Template Artifact、输入、dependency lock、Runner Workspace 和 IaC state。Runner 生命周期只提供 Create 和 Destroy；配置或模板变化通过新的 Runner Generation 实现，不向既有 Generation 执行 Update。

同一 Create 在外部结果仍可证明未产生时可以恢复性重试；一旦 subprocess 结果可能未知，则先经过 Runner 安全注销和 Destroy，无法证明安全时进入 Quarantine。

## Consequences

- IaC engine 始终运行在 daemon 主机的本地 subprocess 中，不能通过 Kubernetes Job、Runner container 或其他远端执行器承载。
- Kubernetes 或 Docker 操作只由 Template Artifact 中的 IaC provider 发起；Shaula 原生不链接相应平台 client。
- Destroy 必须使用 Create 时冻结的原始 artifact、输入、dependency lock 和 state，不能重新复制当前模板。
- 每个 Runner Workspace 必须串行执行，其 subprocess 环境、平台凭据和 state 与其他 Runner 隔离。
- 模板升级会产生资源 churn，可能暂时降低容量，但显著收紧了恢复语义和状态空间。
- Destroy 可对同一 Generation 幂等重试；无法证明安全的未知状态必须 fail closed。
