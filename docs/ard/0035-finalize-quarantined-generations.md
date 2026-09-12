---
status: accepted
date: 2026-09-12
---

# Finalize quarantined generations through an explicit operator endpoint

[spec 0028](../specs/0028-quarantined-generation-finalize.md) 定义
`POST /api/v1/generations/{id}/finalize`：操作者在外部确认资源已不存在后，将
`Quarantined` Generation 终结为 `Destroyed` 并释放 occupancy。

2026-09-12 生产观察：`pve-builder-tyo` 的五个 Generation 因 destroy 时缺少持久化的
state identity（create 期间 operation-log archive 不可用，F07 证明链断裂）进入
`Quarantined`。无 runner、无 job，但 occupancy 保持为 5，template input replacement
（cpu_cores）被 `retirement blocked` 拒绝，且无合法收敛路径——Quarantined 是状态机
死端，`transition_allowed` 没有任何出边。

替代方案及其拒绝理由：

- **daemon 自动重试 destroy**：缺 state identity 正是 F07 的 fail-closed 边界；
  无证明链的 destroy 可能删除不属于本 Generation 的资源，不能放宽。
- **运维直接 UPDATE SQLite**：绕过 OIDC actor、scope、`transition_allowed` CAS、
  audit 与 idempotency，违反 spec 0001/0002 的 mutation 通道纪律。
- **fleet decommission 后重建**：可行但过重——要求 Fleet 级零占用已无法满足
  （quarantine 本身就在占位），且销毁 Fleet spec 与 revision 账本只为清理几条
  ledger 行。

设计要点：finalize 只接受 `Quarantined` 行（非通用 kill switch）；不发起任何远程
效果（runner/VM/IaC 的外部核实责任在操作者，证据文本持久化在 audit）；幂等与
audit 复用 spec 0002 的既有机制；所需 scope 为 `fleet.retire`，与处置 Fleet 同级。
