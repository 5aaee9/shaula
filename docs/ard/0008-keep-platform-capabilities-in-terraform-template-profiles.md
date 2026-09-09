---
status: accepted
date: 2026-09-04
---

# Keep platform capabilities in Terraform Template Profiles

> **Bootstrap amendment:** [ARD-0024](0024-bootstrap-official-runner-images-outside-containers.md) supersedes the original container handoff/platform-tool clauses below for new official-image Templates. Docker uses native JIT env and stopped-container copy/start; Kubernetes uses a Secret env reference and conditional host publication before startup. No custom shim/init image is used. The fixed Runtime bootstrap may use host platform tools and exact Revision credentials; no general Update or core platform API is added. The original clauses below describe retained v1/v2 artifacts only. Credential-grade state, no Runner provider/control-plane credentials, and original-state Destroy remain current.

Shaula 原生实现 pure-Rust daemon/CLI、GitHub Adapter、HTTP/SQLite、artifact/workspace、Template Runtime 与 OTel。[ADR-0014](0014-run-lifecycle-workers-with-a-database-http-state-backend.md) 将完整 Generation 生命周期下沉到 exec 启动的 `shaula job`，state/locks 交给内部数据库 HTTP backend；平台能力边界不变。Kubernetes 与 Docker 都由 Terraform Template Profile 负责调用 provider；Shaula 不链接 Kubernetes 或 Docker client，不构造平台请求，也不把平台 object model 暴露给 Fleet Reconciler 或 Runner Lifecycle Interface。

## Consequences

- artifact manifest 是 `platform`、`bindings_contract` 与 `schemas/bindings.schema.json` 的唯一来源；HTTP/SQLite 只引用 admitted Profile Revision 与 exact opaque `bindings_digest` commitment，不得自行声明或推断 platform。该 commitment 不得是 sensitive plaintext 的 unkeyed digest/offline verifier。
- 通用 Template Runtime Interface 只接受不可变 Profile Revision、标准 protected input 和 per-generation workspace/state，并返回 schema-validated opaque output 与 Create/Destroy 结果。
- Runner state-mutating primitive 只有 Create 和 Destroy。Profile admission validation、state inspection 或只读 drift diagnosis 不是 Runner Operation，且不得修复或重建既有 Generation。
- Saved-plan action/shape/provenance policy 仍 fail closed，由 spec 0004 §5 唯一维护；worker 在 apply 前重验原始 engine/plan/artifact/inputs 与 state。
- 不再要求 daemon 持久化 `ApplyStarting`/`DestroyApplyStarting` 等逐命令 subphase；spec 0010 的 Worker Claim、Create-start facts、process fencing 与 state/lock/terminal transactions 提供恢复边界。原始 inputs 和 emergency state 保留到可信 empty-state completion。
- Kubernetes namespace/Pod/Secret 与 Docker socket/container 的约束由 Profile HCL、manifest contract、declared evidence 和外部验收测试保证。
- v1 同时交付 `templates/kubernetes` 与 `templates/docker`；未来新增 Template Platform 应增加 Template Artifact 与验收，不应扩大 core lifecycle Interface。
- Template 通过静态校验后自动激活，规则由 [spec 0017](../specs/0017-automatic-template-activation.md) / [ARD-0021](0021-activate-templates-after-static-validation.md) 维护。独立 passing conformance attestation 绑定 exact engine binary/provider lock and checksum/artifact/protected bindings/runtime/trust policy/image-or-executable/manifest contracts/suite tuple，作为安全 Create/Destroy、规定的 JIT handoff/no-active-leak 边界和已接受 trust limitation 的运行验证证据；没有该证据不得声称完整平台验收通过，但不阻止 Active 或 Fleet 引用。v1 不把同 Runner Execution Domain 内的 JIT process-inspection isolation 作为门禁。
- Docker Template 的 Terraform child/provider 可以通过受保护的 `docker.sock` 创建 container；bundled default Runner container 明确不挂载该 socket。未来是否另交付显式 high-trust Runner-socket Profile 需要独立决定，不能改变 default Profile。
- 与 daemon 使用同一 OS identity 的所有 IaC child 都共享访问该 socket 的 ambient host-admin trust domain；minimal env 不是隔离边界。v1 接受该共享宿主信任域；独立 identity/sandbox 是需另行决定与验收的可选 hardening，不是当前保证。
- Template Platform 管理凭据只进入对应 IaC subprocess，不进入 Runner 或 workflow；GitHub Control-Plane Credential 连 IaC subprocess 也不得进入。
- template failure 仅暴露 provider-neutral `TemplatePlanFailed`/`TemplateExecutionFailed` 与 bounded phase；从 Day 0 起，OTel 的 platform attribute 也只能由 artifact manifest 派生。

详细契约见 [Template Profile Runtime Specification](../specs/0004-template-profile-runtime.md)。
