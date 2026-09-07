---
status: accepted
date: 2026-09-04
---

# Manage Fleet desired state through HTTP and SQLite revisions

Shaula v1 通过进程内 HTTP control Interface 管理 Fleet resources。已接受的 Fleet Spec 作为不可变 Fleet Revision 保存在 SQLite，当前 desired head 是运行时唯一真相源；`shaula serve --config` 只保留 storage、HTTP binding、execution limits、root trust 和 observability 等 daemon bootstrap concerns，不再持续定义 Fleet 或 Profile catalog。

HTTP Adapter 调用 Fleet Registry Module。一次有效 mutation 在一个短 SQLite transaction 中提交 desired revision、Fleet Change、append-only audit fact 和 durable reconcile wake marker，然后在不调用 GitHub 或 IaC engine 的情况下返回；Fleet supervisor 异步处理已提交的 Revision，周期性 SQLite scan 修复丢失的进程内通知和进程崩溃。

外部 Interface 保持 resource-oriented 且足够小：

- 使用 conditional `PUT` 完整替换 Fleet，而不是 field-level `PATCH`；
- desired Fleet reads 与 runtime status reads 分离；
- 每次 mutation 使用 optimistic concurrency 和 idempotency key；
- mutation 返回可查询的异步 Fleet Change；
- `DELETE` 请求安全 Fleet Decommission，不是 database-row purge 或 force destroy。

Fleet-level replacement 不是 Runner Update primitive。每个既有 Runner Generation 仍然不可变，只能经过 Create 或 Destroy；Fleet Revision 的变化只能影响后续 Create 或通过 Destroy/Create 替换 Generation。

## Consequences

- Fleet 创建、容量调整和 Decommission 不需要 daemon restart。
- SQLite 在同一个 single-writer ownership boundary 内保存 Fleet revisions、Changes、status、idempotency records 和 Runner lifecycle ledger。
- HTTP Adapter、未来 remote CLI 和测试必须使用相同 Fleet Registry Interface，不得直接编辑 SQLite。
- Fleet Spec 使用 typed `github.com` organization/repository Target、稳定 Auth Profile key，以及 current Active 且已 attested 的精确 Template Profile Revision；它不接受 `config_url`、platform-specific raw target、attestation 或 credential。Fleet Revision 记录 admission-time Auth tuple，独立 Auth Handoff state 保存并推进当前完整 desired/observed Auth Revision Refs，因此 same-Profile promotion 不改写 Fleet Spec、Revision 或 ETag。
- GitHub access、Profile readiness 和 Template Platform prerequisites 是异步 status Conditions，不得让 HTTP transaction 调用远端系统。
- YAML 或 Git 可以产生 HTTP requests，但 daemon 不监视它们作为第二个 desired-state source。
- HTTP request cancellation 不会取消已提交的 Fleet Change 或其产生的 Runner Operation。
- Fleet Decommission 永久停止新 acquisition/Create，但允许持久化的 cleanup-only Auth Handoff；它等待安全 GitHub removal，Destroy 全部 owned Runner Resource，关闭 Fleet supervisor，并保留空 GitHub Scale Set 与 auditable tombstone。
- 不提供 force-delete path；Busy jobs、unknown remote Runners、Quarantine 或 missing state 可以让 Decommission 明确阻塞。
- Template Profile、Template Artifact 和 GitHub Auth Profile 由独立的 Profile HTTP resources 管理；`template.publish`、`template.attest` 与 `fleet.write` 分权，Fleet mutation 不能发布模板代码、提交 attestation 或提交原始 credential。
- Management authentication、authorization、mutation audit 和 HTTP telemetry 是 Day 0 requirements。经 [ADR-0013](0013-require-openid-connect-for-all-http-access.md) 修订：v1 listener 仍只允许 loopback，non-loopback configuration 在 startup fail closed；reverse proxy 提供 HTTPS，Shaula 自行验证 mandatory OIDC session/API token 并执行 authorization/audit。Legacy backend token 与 actor headers 不再建立身份；native inbound TLS/mTLS 不在范围内。
- Metrics 不使用动态 Fleet Key 作为无界 dimension；per-Fleet 状态通过 HTTP、traces 和 logs 观测。
- SQLite 只支持一个本地 active daemon；本决定不增加 multi-host HA、distributed leases 或 concurrent writers。

详细契约见 [Fleet HTTP control-plane specification](../specs/0002-fleet-http-control-plane.md)。
