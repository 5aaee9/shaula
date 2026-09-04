---
status: accepted
date: 2026-09-04
---

# Keep platform capabilities in Terraform Template Profiles

Shaula 原生只实现 pure-Rust daemon/`clap` CLI、GitHub Scale Set adapter、HTTP/SQLite control plane、artifact/workspace 管理、provider-neutral `shaula-template` runtime、本地 IaC subprocess 和 OpenTelemetry。Kubernetes 与 Docker 都由 Terraform Template Profile 负责调用 provider；Shaula 不链接 Kubernetes 或 Docker client，不构造平台请求，也不把平台 object model 暴露给 Fleet Reconciler 或 Runner Lifecycle Interface。

## Consequences

- artifact manifest 是 `platform`、`bindings_contract` 与 `schemas/bindings.schema.json` 的唯一来源；HTTP/SQLite 只引用 admitted Profile Revision 与 exact opaque `bindings_digest` commitment，不得自行声明或推断 platform。该 commitment 不得是 sensitive plaintext 的 unkeyed digest/offline verifier。
- 通用 Template Runtime Interface 只接受不可变 Profile Revision、标准 protected input 和 per-generation workspace/state，并返回 schema-validated opaque output 与 Create/Destroy 结果。
- Runner state-mutating primitive 只有 Create 和 Destroy。Profile admission validation、state inspection 或只读 drift diagnosis 不是 Runner Operation，且不得修复或重建既有 Generation。
- saved plan 必须 fail closed：只接受 supported JSON major、`applyable=true`、`complete=true`、`errored=false`；Create 要求 empty prior managed state 和 exact creates，Destroy 要求 empty-state no-apply 或 exact deletes，只有 data resource 可 read/no-op；import、deposed、move、replacement、unknown 与 deferred 一律拒绝。
- plan digest 绑定 exact engine executable/kind/version/binary digest、artifact/input digest、state lineage/serial 或 empty sentinel、Generation 与 attempt；apply 前重新 hash plan、inputs 和 engine binary。Create 与 Destroy 分别先持久化 `ApplyStarting` 和 `DestroyApplyStarting`，exact original protected input 保留到 empty-state Destroy。
- Kubernetes namespace/Pod/Secret 与 Docker socket/container 的约束由 Profile HCL、manifest contract、declared evidence 和外部验收测试保证。
- v1 同时交付 `templates/kubernetes` 与 `templates/docker`；未来新增 Template Platform 应增加 Template Artifact 与验收，不应扩大 core lifecycle Interface。
- 每个可激活 Profile 必须有绑定 exact engine binary/provider lock and checksum/artifact/protected bindings/runtime/trust policy/image-or-executable/manifest contracts/suite tuple 的 passing attestation；无法证明安全 Create/Destroy、规定的 JIT handoff/no-active-leak 边界和已接受 trust limitation 的 Revision 不得 `Active` 或 advertise capability。v1 不把同 Runner Execution Domain 内的 JIT process-inspection isolation 作为门禁。
- Docker Template 的 Terraform child/provider 可以通过受保护的 `docker.sock` 创建 container；bundled default Runner container 明确不挂载该 socket。未来是否另交付显式 high-trust Runner-socket Profile 需要独立决定，不能改变 default Profile。
- 与 daemon 使用同一 OS identity 的所有 IaC child 都共享访问该 socket 的 ambient host-admin trust domain；minimal env 不是隔离边界。是否采用独立 identity/sandbox 仍是显式 open decision。
- Template Platform 管理凭据只进入对应 IaC subprocess，不进入 Runner 或 workflow；GitHub Control-Plane Credential 连 IaC subprocess 也不得进入。
- template failure 仅暴露 provider-neutral `TemplatePlanFailed`/`TemplateExecutionFailed` 与 bounded phase；从 Day 0 起，OTel 的 platform attribute 也只能由 artifact manifest 派生。

详细契约见 [Template Profile Runtime Specification](../specs/0004-template-profile-runtime.md)。
