---
status: accepted
date: 2026-09-04
---

# One daemon supervises multiple isolated homogeneous Fleets

一个 `shaula serve` 实例可以管理多个 Fleet。每个 Fleet 固定绑定一个 GitHub Actions Scale Set、一个 GitHub Auth Profile 和一份同质 Template Profile Revision，并拥有独立的 listener/session、需求快照和生命周期状态；所有 Fleet 共享 SQLite、artifact store、有界且公平的本地 IaC subprocess 调度器和 OpenTelemetry pipeline。

这个形状允许不同 Fleet 使用不同 Template Platform 和 Profile，同时避免在单个 Scale Set 的聚合需求中进行无法可靠完成的逐 job 模板路由。

## Consequences

- 不同 OS、架构、权限域、Template Platform 或 Template Profile Revision 必须使用不同 Fleet/Scale Set。
- Fleet Key 和远端 Scale Set identity 在 daemon 管理的 active Fleet set 中必须唯一；一个 Scale Set 同时只能由一个 daemon 所有。
- Fleet-local listener、认证、模板或 IaC 故障只降级对应 Fleet；共享存储、全局调度器或 telemetry pipeline 的故障可能影响整个 daemon，但不得造成跨 Fleet 状态污染。
- 全局 worker budget 必须在 Fleet 之间公平调度，不能让一个 Fleet 长期饿死其他 Fleet。
- 每个 Fleet 独立执行 create-or-adopt 并验证兼容性；普通 daemon 退出不删除任何 Scale Set。
- v1 仍是单活本地 daemon，不提供跨主机 lease 或 HA。
