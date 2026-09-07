---
status: accepted
date: 2026-09-04
---

# Manage Template and GitHub Auth Profiles through HTTP and SQLite

Shaula 通过同一个 HTTP control plane 管理 Fleet、Template Profile 和 GitHub Auth Profile。Profile 使用不可变 Revision，SQLite 保存 desired state、validation status 与 active heads；Template Artifact 通过 HTTP 发布到 content-addressed artifact store。`shaula serve --config` 不再包含 `template_profiles` 或 `github_auth_catalog` 真相源。

PAT、GitHub App private key 与 schema 标记为 sensitive 的 Kubernetes/Docker Template bindings 都作为 write-only HTTP fields 提交，并以未应用字段级加密的原始明文保存在各自 immutable Revision 的 SQLite rows 中。External secret reference 不是 v1 storage mode。HTTP transport 的保护与此 at-rest 决定相互独立；读取响应、audit、errors、logs 和 OpenTelemetry 永不回显 secret value 或其 prefix、suffix、hash、length 等可推导表示。Wire field `bindings_digest` 只能是不可用于离线猜测验证的 opaque Revision commitment，不能是 sensitive plaintext 的 unkeyed digest。

## Consequences

- Profile Registry Module 为 Template Profile、Template Artifact 和 GitHub Auth Profile 提供 typed resources；Fleet Registry 只引用它们，不承担模板发布或 credential 写入。
- Template Artifact 是不可变且 content-addressed 的；Template Profile Revision 固定 artifact、manifest-derived `platform`/`bindings_contract`、bindings digest、input contract 与 dependency lock。Static validation 只产生 `Ready`，exact conformance attestation 才能产生 `Active`。
- Fleet 只固定引用 current Active Template Profile Revision 及其 artifact/attestation identity；发布或 attesting 新 Revision 不会修改既有 Fleet 或 Runner Generation。
- Fleet durable state 保存完整 desired/observed GitHub Auth Revision Refs。same-Profile promotion 与零 Occupancy且无 active acquisition/GitHub/Runner Operation 的 cross-Profile replacement 共用一个 Auth Handoff；handoff 只做 quiesce/classification/acknowledgement，ordinary reconcile 独占 create/adopt、ID binding 与 session。
- 改变 auth kind、App/installation identity 或 Target allowlist 需要新 GitHub Auth Profile，不能伪装成 credential rotation。
- HTTP Adapter 只在短 SQLite transaction 中提交 revision、Profile Change、audit fact 和 durable work marker；artifact validation、credential validation 与 dependent Fleet handoff 都异步执行且可从 SQLite 恢复。
- Template Artifact 发布与 conformance attestation acceptance 是独立受信能力，必须分别与 Fleet/Auth mutation 分权；Fleet 只能选择 current Active Revision 与其允许的 inputs。
- Profile deletion 受 exact Profile heads、Fleet refs、in-flight effects/sessions、cleanup/recovery references 与 retention policy 约束；`Blocked` 不释放引用，也不提供 force delete。
- SQLite 主库、WAL/SHM、online copy、备份、迁移副本和 crash dump 都可能含明文 credential/binding，必须采用 credential-grade host permissions、retention、backup handling 和 disposal policy；application-level field encryption 不是 v1 requirement。
- GitHub Control-Plane Credentials 永不进入 Template/Terraform/Runner；sensitive Template bindings 只从 exact Revision 解析到获准 IaC child，并且永不进入 Runner/workflow。两类 secret 都不会因 `write-only` 而丧失 restart/Destroy/recovery 可用性。
- Management endpoint 的 authentication、authorization、audit、request-size limits 和 redaction 是 Day 0 requirements。经 [ADR-0013](0013-require-openid-connect-for-all-http-access.md) 修订：v1 listener 仍只绑定 loopback，remote TLS 由 reverse proxy 承担；Shaula 必须验证启动时显式配置的 OIDC Provider 身份，禁止 proxy actor/backend-token fallback。Native inbound TLS/mTLS 仍不在范围内。

详细契约见 [Profile HTTP Control-Plane Specification](../specs/0005-profile-http-control-plane.md)。
