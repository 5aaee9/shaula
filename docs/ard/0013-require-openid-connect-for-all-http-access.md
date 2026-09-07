---
status: accepted
date: 2026-09-06
supersedes: 0011
amends: [0005, 0009, 0012]
amended-by: 0014
---

# Require OpenID Connect for all HTTP access

> Scope amended by [ADR-0014](0014-run-lifecycle-workers-with-a-database-http-state-backend.md): 本决定继续覆盖所有管理 UI/API/health；新增的独立内部 worker/state listener 使用 spec 0010 的分权 capabilities，不是管理认证回退。

## Context

原设计由 reverse proxy 认证 caller，Shaula 验证 backend token / actor headers，且 UI shell 公开。这无法满足启动时必须显式选择 OpenID Connect Provider、所有 UI/API 在认证后才能访问的新要求。OIDC 是 Shaula 的强制运行依赖，不再是代理的可选部署能力。

## Decision

Shaula MUST 在 daemon 内实现 OIDC relying party 和 API token verification。`serve` 必须通过 clap flag `--oidc-provider` 或 `SHAULA_OIDC_PROVIDER` 指定单一 issuer，同时提供所需 client ID、server-only client secret、固定 HTTPS public origin 和 API audience。完整配置表、协议、route matrix 和验收条件由 [spec 0009](../specs/0009-mandatory-openid-connect.md) 统一定义。缺失、非法或初始化失败时，daemon 在 listener 和资源 workers 启动前非零退出；没有默认 Provider、anonymous mode 或 proxy-header fallback。

浏览器使用 Authorization Code + PKCE S256，由 Rust server 验证 ID Token 并建立有期限、可撤销的 opaque cookie session。React 不持有 OIDC tokens。API/health 也可使用同一 Provider 为 API audience 签发的 JWT access token；ID Token 不能作为 API token。所有身份仍经过 Shaula 的 scope authorization，actor/audit/idempotency 以 `(iss, sub)` 为稳定 principal。

管理 listener 的 UI HTML、embedded assets、全部 API、health 和未来 routes 默认受 OIDC 保护。只有 exact GET login/callback routes 可匿名进入，以完成认证；它们不能提供应用内容。UI document 无 session 时 redirect，API/static/health 无凭据时 `401`，已认证但权限不足时 `403`。所有响应使用 private/no-store，cookie mutation 同时校验 Origin 与 CSRF。认证在 static cache/SPA fallback 和业务 handler 之前执行。

继续保留 loopback-only Axum listener 和远程 HTTPS reverse proxy：non-loopback bind 仍 fail closed，原生 inbound TLS/mTLS 不在本次决定内。Proxy 只承担 TLS/传输，不产生受信 actor；direct loopback、debug 和 development 均要求 OIDC。Native OIDC 的新增网络依赖只允许来自已配置 issuer/discovery 的受验证 endpoints。

## Consequences

- 本 ADR supersedes ADR-0011 的认证设计，并在此重申其 loopback/TLS exposure 限制；修订 ADR-0005 / ADR-0009 的认证责任和 ADR-0012 的 public-shell 决定。
- 移除 legacy backend token / actor/scopes headers 和 Vite actor injection；部署需注册 Provider client、配置 callback/API audience、显式授予 principal 权限，并让 HTTP probes 携带凭据。
- 未登录用户不能取得 UI bundle；session 到期后不能从服务器继续取得 UI/API/asset 内容。已交付给浏览器的字节无法撤回，因此禁止持久/共享缓存与离线 service worker。
- Identity-provider availability、discovery/JWKS rotation、session/CSRF lifecycle 成为 HTTP Adapter 的责任；使用成熟 Rust 库，限制网络重试和内存状态。暂时 outage 不授权 anonymous access，也不遗忘已有 Runner cleanup intent。
- 不引入多租户隔离、本地密码账户、自动首次登录 admin 或 GitHub credential 复用。现有资源 scope、conditional writes、audit、redaction 和 asynchronous convergence 语义继续适用。
- 实现进度与本地/真实 Provider 验收证据仅在 [implementation status](../IMPLEMENTATION_STATUS.md) 维护；本 ADR 的 accepted 状态不等于发布验收通过。
