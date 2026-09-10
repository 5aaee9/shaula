---
status: accepted
date: 2026-09-10
amends: [0025]
---

# Follow latest Active template revision

Fleet 的模板引用增加 follow-latest 模式：bare key 引用的 Fleet 由 daemon 在 Profile
激活新 Revision 后自动升级 pin，pinned 引用行为不变。协议和验收统一维护在
[spec 0023](../specs/0023-fleet-template-follow-latest.md)。

ARD-0025 让已发布模板的 Update 保留所有 Fleet pin（spec 0021 §4），升级完全依赖用户
对每个 Fleet 手工 replacement。实践中发布修复型模板（如 guest bootstrap 修复）后，
每个 Fleet 都要重复一次无信息量的 re-pin 操作；而 Revision 机制真正的价值——per-
Generation 冻结、attestation 链、handoff CAS——全部发生在内部，与用户是否手工选择
revision 无关。

考虑过完全移除 revision pin（可变 current spec）。但凭证轮换 fencing（handoff
desired/observed）和 replacement 的零占用门禁仍需要单调的权威标记来区分新旧 effect；
删除表结构只会把同一概念以内部 epoch 形式重新实现，却失去现成的审计与恢复语义。
因此保留 Revision 作为内部实现，只把"跟随最新"提升为引用语义。

级联采用 level-triggered scan 而非 publish 事件钩子：占用非零的 Fleet 本来就必须延后
升级，延后队列若用事件实现需要额外的持久化与去重；周期 scan 天然在占用归零后重试，
重启与事件丢失都不丢升级义务。升级提交复用 `commit_fleet_mutation` 与 effect gate，
不与并发 PUT/Create 产生新的竞争窗口。

Auth Profile 侧自 staged activation 起即为 follow latest（promotion 同事务 retarget
全部存活 Fleet 的 handoff desired），本决定把 Template 侧对齐到同一策略，不改动 Auth。
