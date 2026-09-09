---
status: accepted
date: 2026-09-04
---

# Manage Template and GitHub Auth Profiles through HTTP and SQLite

Artifact 存储与默认来源导入已由 [ARD-0019](0019-store-template-sources-and-discover-terraform-variables.md)
修订：原始 archive 持久化到 SQLite，文件系统材料是可重建缓存；Profile lifecycle 决定保持有效。

[ADR-0015](0015-route-one-github-app-profile-to-multiple-accounts.md) / [spec 0011](../specs/0011-multi-account-github-authentication.md) 已允许同 App 的 policy/installation bindings 通过新 Revision 演进。[ARD-0022](0022-retire-legacy-github-authentication.md) / [spec 0018](../specs/0018-github-app-only-authentication.md) supersede 本文原 PAT、单 installation/固定 allowlist 和旧格式兼容支持；当前仅接受 schema 2 GitHub App，历史 rows 保留但不转换或授权执行。持久化、secret、retirement 与其他 lifecycle 决定继续有效。

Shaula 通过同一个 HTTP control plane 管理 Fleet、Template Profile 和 GitHub Auth Profile。Profile 使用不可变 Revision，SQLite 保存 desired state、validation status 与 active heads；Template Artifact 通过 HTTP 发布到 content-addressed artifact store。`shaula serve --config` 不再包含 `template_profiles` 或 `github_auth_catalog` 真相源。

GitHub App private key 与 schema 标记为 sensitive 的 Kubernetes/Docker Template bindings 作为 write-only HTTP fields 提交，并以未应用字段级加密的原始明文保存在各自 immutable Revision 的 SQLite rows 中。历史 PAT bytes 继续受该 at-rest 保护，但不再接受 PAT publication。External secret reference 不是 v1 storage mode。HTTP transport 的保护与此 at-rest 决定相互独立；读取响应、audit、errors、logs 和 OpenTelemetry 永不回显 secret value 或其 prefix、suffix、hash、length 等可推导表示。Wire field `bindings_digest` 只能是不可用于离线猜测验证的 opaque Revision commitment，不能是 sensitive plaintext 的 unkeyed digest。

## Consequences

- Profile Registry Module 为 Template Profile、Template Artifact 和 GitHub Auth Profile 提供 typed resources；Fleet Registry 只引用它们，不承担模板发布或 credential 写入。
- Template Artifact 是不可变且 content-addressed 的；Template Profile Revision 固定 artifact、manifest-derived `platform`/`bindings_contract`、bindings digest、input contract 与 dependency lock。Static validation 通过后自动产生 `Active`，已有 Ready 由相同路径重新校验和激活，详见 [spec 0017](../specs/0017-automatic-template-activation.md) / [ARD-0021](0021-activate-templates-after-static-validation.md)。兼容的 `*_attestation_id` 字段承载 opaque activation provenance；自动激活不伪造 conformance record。
- 新/改变的 Fleet Template 引用只接受 current Active subject；既有 pin 的 capacity/inputs/no-op mutation 不重新准入或升级，详见 spec 0002。发布/attest 新 Revision 不修改既有 Fleet/Generation。
- Fleet durable state 保存完整 desired/observed GitHub Auth Revision Refs。same-Profile promotion 与零 Occupancy且无 active acquisition/GitHub/Runner Operation 的 cross-Profile replacement 共用一个 Auth Handoff；handoff 只做 quiesce/classification/acknowledgement，ordinary reconcile 独占 create/adopt、ID binding 与 session。
- Auth publication 只支持 schema 2 GitHub App。不同已验证 numeric App identity 需要新 Profile；同 App 的 key、TargetPolicy 和 installation replacement 经 spec 0011 的 Candidate 验证，不再属于必须新 key 的变化。PAT、旧版本、未知版本、固定 installation 和 allowlist 输入在 persistence/replay 前拒绝，且没有旧 Profile 升级入口。
- HTTP Adapter 只在短 SQLite transaction 中提交 revision、Profile Change、audit fact 和 durable work marker；artifact validation、credential validation 与 dependent Fleet handoff 都异步执行且可从 SQLite 恢复。
- Template publication 与 attestation 分权。`template.publish` 授权静态校验后的自动激活；v1 选择已验证 OIDC actor + 独立 `template.attest` 权限 + immutable record/audit 作为 conformance 证据完整性边界，不另建签名 PKI，不声称 daemon 重跑外部 harness。Attestation PUT 不改变 Profile head 或 activation ID；exact subject 与 evidence linkage 见 spec 0005 §5.1。
- Profile retirement 先等 live Fleet/session/effect/worker/cleanup/recovery refs，随后按 spec 0005 §7.1 原子释放自身 heads 的执行引用并终结；历史 Revision/terminal Change 仅因 retention 存在，不可永久阻塞 retirement。`Blocked` 本身不是 release proof，也无 force delete。
- SQLite 主库、WAL/SHM、online copy、备份、迁移副本和 crash dump 都可能含明文 credential/binding，必须采用 credential-grade host permissions、retention、backup handling 和 disposal policy；application-level field encryption 不是 v1 requirement。
- GitHub Control-Plane Credentials 永不进入 Template/Terraform/Runner；sensitive Template bindings 只从 exact Revision 解析到获准 IaC child，并且永不进入 Runner/workflow。受支持的 v2 App 和 Template binding secrets 不会因 `write-only` 而丧失 restart/Destroy/recovery 可用性；旧认证执行必须在部署前按 spec 0018 用上一版本恢复，保留旧 bytes 不授权当前执行。
- Management endpoint 的 authentication、authorization、audit、request-size limits 和 redaction 是 Day 0 requirements。经 [ADR-0013](0013-require-openid-connect-for-all-http-access.md) 修订：v1 listener 仍只绑定 loopback，remote TLS 由 reverse proxy 承担；Shaula 必须验证启动时显式配置的 OIDC Provider 身份，禁止 proxy actor/backend-token fallback。Native inbound TLS/mTLS 仍不在范围内。

详细契约见 [Profile HTTP Control-Plane Specification](../specs/0005-profile-http-control-plane.md)。
