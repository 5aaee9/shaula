---
status: accepted
date: 2026-09-04
---

# Expose v1 HTTP through loopback and an authenticating proxy

Shaula v1 的 Axum listener 只支持 loopback address。若 bootstrap 配置 `0.0.0.0`、`::` 或任何 non-loopback address，daemon 必须在 startup fail closed，而不是静默降级、自动生成证书或启动不受保护的 listener。

需要远程管理时，由部署方提供 trusted reverse proxy。Proxy 终止 TLS、认证 caller，并在只有 proxy 与 Shaula 可访问的 backend trust boundary 内传递 authenticated actor；Shaula 验证该 context 并继续拥有 resource authorization、mutation audit、request limits、redaction 和 desired-state semantics。v1 不在 daemon 内实现 inbound HTTP TLS serving、mTLS client verification 或 OIDC token verification。

## Consequences

- 每个 management request，无论经 proxy 还是 direct loopback，都必须携带并通过 selected trusted actor assertion/backend authentication。Loopback origin 不会自动产生 actor；缺少或无效 context 的 direct call 必须拒绝。Proxy 必须丢弃 caller-supplied identity headers，再注入来自其认证结果的 actor context；request body 或普通外部 header 永远不能声明 actor。
- Exact actor assertion、anti-replay properties 与 proxy-to-Shaula backend authentication format 在实现前仍需固定；在此 contract 完成前，remote exposure 不得宣称可安全部署。
- Loopback 不是 authentication 或 tenant-isolation boundary。Host administrator 与有权重配 proxy/backend authentication 的主体位于 v1 administrative trust domain；普通 local process 仅凭连接 loopback port 不能成为 authenticated actor。
- Proxy 不能把 management request/response bodies、Authorization、actor assertion、PAT/App private key、Template sensitive bindings、JIT、Terraform input/state 或 OTel headers 写入 access/error logs 或 telemetry。
- Shaula 的 authorization 与 audit 测试必须覆盖 trusted proxy actor context、携带同一可信 backend context 的 direct loopback administrative access，以及对 caller identity-header forgery 和缺失/非法 actor context 的拒绝。
- `/livez` 与 `/readyz` 的 proxy exposure 可以单独限制，但不能绕过 management route 的 actor/authorization contract。
- 如果未来要求 daemon 原生 TLS、mTLS、OIDC、多租户 object authorization 或 public-network listener，需要新的 ADR 和 threat model；它们不是 v1 的隐含 extension point。

直接暴露 plaintext non-loopback listener 被拒绝，因为 write-only GitHub credential 与 Template binding 会跨越不受保护网络。v1 同时拒绝立即内建 mTLS/OIDC：它会把 certificate lifecycle、identity-provider semantics 和额外 network attack surface 纳入 daemon，而当前部署可由专用 reverse proxy 提供这些能力。

详细契约见 [Fleet HTTP Control-Plane Specification](../specs/0002-fleet-http-control-plane.md) 与 [Profile HTTP Control-Plane Specification](../specs/0005-profile-http-control-plane.md)。
