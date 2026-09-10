# JIT Mint 不确定性恢复

- Status: Accepted (2026-09-10); implementation and verification tracked separately.
- Decision: [ARD-0031](../ard/0031-classify-uncertain-jit-mints-by-exact-name-lookup.md).
- Implements: [spec 0003](0003-kubernetes-runner-resource.md) §7 的 uncertainty
  分类条款在 JIT mint 效果上的应用（该条款原文即为 runner-scale-set 协议路径）。

## 1. Outcome

`generatejitconfig` 返回 `Uncertain`（请求可能已达 GitHub，响应丢失）时，supervisor
不再直接丢弃 runner 实体引用并将 Generation quarantine——那会在 GitHub 留下无主
agent，使下一次 inventory 校验进入 `UnknownRemoteRunner`、fleet 永久 Degraded、
tick 不再到达容量决策（2026-09-10 生产事故）。替代流程按精确名字查找分类：

1. **ExactlyOne**（mint 落地）：编码后的 JIT 配置已不可恢复，Generation 无法启动
   runner。记录 runner id（inventory 从此有主），随后**移除该 runner 实体**
   （`Removed` / `AlreadyAbsent` 均为收敛），Generation 推进 `CleanupRequired`，
   走现有清理/终结通道。无孤儿、无库存阻塞。
2. **None**（mint 未落地）：Generation 被证明零资源。推进 `CleanupRequired`；
   下一个 tick 以全新 Generation/name 重试（spec 0003 §7 的 fresh-name 语义），
   无需操作者介入。
3. **Multiple**、查找失败、或移除被 `JobStillRunning` 阻塞：保守推进
   `Quarantined` 并记 WARN（含失败摘要）。实体可能残留，inventory 保持阻塞，
   由操作者处置——与"歧义即隔离"的既有规则一致。

`Err`（确定性失败，请求从未到达或被明确拒绝）行为不变：记 WARN 后 quarantine。边界：效果类
端点（如 generatejitconfig）**响应体解码失败**时效果可能已落地，理想分类是 Uncertain 而非
Err；传输层解码失败目前仍归入 Err，本 spec 的恢复路径不覆盖该窗口。2026-09-10 事故中该
窗口的触发图（generatejitconfig 返回数字 AgentStatus 导致解码失败）已在 wire 层修复，
但传输分类的完善留给后续 ARD。

## 2. 状态与占用

CleanupRequired 的 Generation（本次两类落地路径均无 post-apply state identity）
按既有 60 秒 stale 规则收敛到 `Quarantined` 并保留 occupancy，直到操作者按
quarantine 处置流程清理。每次未闭环的 Uncertain 事件消耗一个容量槽——这是有界、
可见、fail-safe 的代价，换取 fleet 不死锁；真正的 quarantine GC 属于独立的
operator procedure 范畴。

## 3. Ordering 与证明

- 恢复在 `acquire_generation_jit` 内同步执行：durable `JitStarting` 意图行先于
  远程效果存在（既有 R9-07 顺序不变），恢复结论只补充该行的后续状态。
- 移除使用与 retirement 相同的 `remove_runner` 幂等语义；`AlreadyAbsent` 说明
  实体已消失，同样视为收敛。
- 恢复不重试 mint 本身：fresh Generation 由下一 tick 的容量循环创建，天然携带
  新的 attempt id 与 JIT 上下文。

## 4. Acceptance

- Uncertain + 精确名命中：runner id 落库、实体被移除、Generation 进入
  `CleanupRequired`；inventory 校验立即可通过。
- Uncertain + 精确名缺失：Generation 进入 `CleanupRequired`，无任何远程调用副作用
  之外的残留；下一 tick 为同一 demand 创建全新 Generation。
- Uncertain + 歧义/查找失败：`Quarantined` 且 WARN 携带原因。
- 确定性 `Err`：行为与既往一致（quarantine + WARN）。
