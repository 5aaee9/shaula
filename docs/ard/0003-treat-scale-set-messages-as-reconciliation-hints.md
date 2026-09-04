---
status: accepted
date: 2026-09-04
---

# Treat Scale Set messages as reconciliation hints

Shaula v1 由纯 Rust `shaula-scaleset` Adapter 为每个 Fleet 实现 message session 与长轮询。Adapter 在 ACK 前持久化 statistics snapshot、幂等 Job Observation、message checkpoint 和 acquisition intent；事务失败时不 ACK，重复投递按消息身份幂等处理。固定版本的 `actions/scaleset` Go SDK 与 `internal/testserver` 只作为 wire/outcome compatibility oracle，不进入生产运行时。

即使采用 persist-before-ACK，服务端截断、重分配、session 丢失、消息过期或协议本身不提供完整历史时，JobStarted、JobCompleted 仍可能缺失、重复或乱序。因此消息只用于加速 reconcile，不能累加成 desired count 或当作生命周期真相源。Shaula 通过最新 Assigned Demand 覆盖快照、GitHub Runner inventory、启动恢复、周期性 retirement reaper 和持久化 Runner Operation 等独立 level-triggered sources 收敛资源生命周期。

## Consequences

- v1 不承诺完整 job audit、exactly-once 处理或完整 job-to-runner 映射。
- 每个 Fleet 的持久化 session epoch 单调递增。Session install/replace 持 exclusive session-effect permit，并在旧 epoch outbound effects 完成或 durable classification 后推进 epoch；ACK/Acquire 持 shared epoch permit，从 final CAS authorization/`AcquireStarting` 直至结果分类。Demand/observation 也 compare-and-set 当前 epoch，使已取消或延迟的旧 poll task 只能 no-op，不能 ACK、Acquire 或覆盖新 session initial statistics。
- Assigned Demand 是覆盖式当前状态；容量计算不得对消息做加减计数。
- JobCompleted 不能单独授权 Destroy；已验证 Scale Set ownership 下的 Runner removal、`JobStillRunning` 响应和通用 liveness proof 构成安全门。Scale Set 已消失且资源可能启动时，remote absence 本身不是安全证明。
- “最终收敛”以完整 consistency set、可证明 ownership 和可执行的 GitHub/Template control path 为前提；state/ownership 无法证明时进入可见、可审计的 Quarantine，而不是冒险 Destroy。
- 如果未来要求无损事件记录，需要单独引入 durable event-history contract，并用新的 ADR 取代本决定；当前 oracle compatibility 不构成这一保证。
