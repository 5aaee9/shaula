---
status: accepted
date: 2026-09-09
---

# Reconcile labels on owned Scale Sets

Fleet labels 是 GitHub job 路由配置。将其归为 immutable identity 会使已有 Fleet 的普通编辑返回 `immutable identity change rejected`，迫使用户为一次路由修改创建新的 Fleet。Scale Set 的稳定身份仍由 Target、runner group、name 和已绑定 ID 决定；labels 不进入其 fingerprint。

## Decision

允许通过既有 Fleet conditional PUT 更新 labels，并沿现有 Revision、fence、Change 和 supervisor 流程异步收敛。只修改 labels 不要求零 Occupancy，不更换 Scale Set ID，不改变现有 Generation 或终止 Busy jobs。字段组合修改继续遵循各自 admission barrier。Web UI 在已有 Fleet 的编辑器中允许填写 labels，并保留原 snapshot version 和冲突草稿。

GitHub Adapter 增加窄接口 `update_scale_set_labels`，通过 `PUT _apis/runtime/runnerscalesets/{id}?api-version=6.0-preview` 更新指定 Scale Set 的 labels。请求 body 仅包含 `labels`，不携带或复制 ID、name、runner group、runner settings，不暴露任意 Scale Set update body。显式值使用 `System` type；空配置沿用 Scale Set name 的 `System` fallback。Wire boundary 接受大小写规范化后的 `System` / `User`，拒绝非 GitHub 枚举值 `Customer`。采用完整集合比较，以支持删除 labels。

2026-09-09 的 GitHub.com 验证推翻了以 [actions/scaleset 通用 PATCH 方法](https://github.com/actions/scaleset/blob/cb0405b2d874500e75ae34eff8d582ab75956b45/client.go) 作为 labels 更新协议的原假设：`Customer` 请求返回 `400`；`System` PATCH 返回 `200` 却未改变 labels；PUT 成功更新 labels。确认已分配、已领取、运行中和已注册计数均为零且该 Scale Set inventory 为空后，仅含 `labels` 的 PUT 将 labels 改回原集合，再恢复目标集合；每步 authenticated GET 均确认 labels 实际变化，且 ID/name/runnerGroupId/runnerGroupName/runnerSetting 逐项未变，因此选用该窄 body。通用客户端的更新方法存在，不代表服务端对每个字段执行更新；该路径必须保留真实 readback 验证，不能仅依赖 mock 的 `200`。

持久化 `owned_scale_set_id`，将成功 create/adopt 与仅 lookup 到候选 ID 区分开。Store 集中维护该标记：明确 Adopted 的正数 ID 建立证明；同 ID、fingerprint、name/group 的 pending/access 状态保留；重新绑定、Unbound、MissingWithResources 或新 Create intent 清除。迁移只回填旧 Adopted 记录，不能把旧 AccessBlocked/UnknownRemoteRunner 的候选 ID 当作所有权。

Supervisor 先证明同一 identity、owned ID 与已知 Runner inventory，再在既有 exclusive effect gate 内重验当前 Fleet/Auth authority，持久化 pending state 并提交 Scale Set labels PUT。Pending state 使 listener 的 durable guard 拒绝 acquisition；gate 对请求提交与 Fleet PUT/DELETE 排序。网络调用不持有 SQLite transaction。更新后必须 readback；恢复时先读同一对象，已收敛则完成，否则在当前 Revision 下重试。通过现有 Fleet status/Change 暴露错误，不新增中央命令队列或 Runner Operation。

## Consequences

- 只放宽已证明归属对象的 labels 配置；首次 adoption、未知 Runner 和所有其他身份冲突仍 fail closed。
- Inventory 先验证 Target 范围清单的结构，再筛选明确属于当前 Scale Set 的 Runner；普通 Runner（归属缺失/为 0）和其他 Scale Set 成员不构成本 Fleet 的 ownership conflict。精确名称查询缺少归属不能证明 ownership 或 absence，详见 spec 0001 §7。
- 复用现有 session 恢复协议，不重置 durable job/acquisition facts，不为 labels 变化运行 Terraform。
- 未确认更新不发布 Ready，不启动新的 acquisition/Create；已经执行的 job 继续。
- GitHub 没有由此获得本地 fence 或幂等保证。超时后读回、周期性重试实现最终收敛，不保证既有排队/分配 job 在更新瞬间重新路由。
- 需要验证 HTTP admission/replay/CAS、labels 删除/清空、首次 adoption 拒绝、迁移与重启恢复、失败/不确定结果、stale revision/deletion/auth guard，以及生产 wiring 的 listener/Ready 路径。

权威行为契约见 [spec 0002 §6.1](../specs/0002-fleet-http-control-plane.md#61-mutable-scale-set-labels)。本决定修订 [ARD-0005](0005-manage-fleet-desired-state-through-http-and-sqlite.md) 的 Fleet replacement 范围，Runner Generation 仍只有 Create/Destroy。
