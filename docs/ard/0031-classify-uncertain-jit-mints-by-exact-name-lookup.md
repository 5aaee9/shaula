---
status: accepted
date: 2026-09-10
---

# Classify uncertain JIT mints by exact-name lookup

`generatejitconfig` 的 Uncertain 响应改为按精确名字查找分类恢复，替代直接
quarantine。协议和验收统一维护在
[spec 0025](../specs/0025-jit-mint-uncertainty-recovery.md)。

生产事故（2026-09-10 21:37）：一次 Uncertain 的 JIT mint 实际已在 GitHub 创建
agent，旧实现丢弃 runner 引用并 quarantine Generation，孤儿 agent 使下一次
inventory 校验进入 UnknownRemoteRunner——fleet 永久 Degraded，tick 在容量决策
之前被阻断，后续 job 反复 assignment_withdrawn，且无任何日志指出原因（该缺陷
同日已补 WARN）。

spec 0003 §7 对 IaC 资源的 Uncertain 早已规定 lookup/removal/absence 分类，
本决定把同一分类应用到 JIT mint 效果上：命中即"效果已落地"，按幂等移除实体后
走 CleanupRequired——编码后的 JIT 配置无法从 lookup 恢复，Generation 本身不可
复用，但孤儿死锁被消除；缺失即"效果未落地"，同样进 CleanupRequired 并由下一
tick 的 fresh Generation 重试；歧义维持隔离。

未选择"恢复后复用同一 Generation 重发 mint"：fresh name 是 spec 0003 的明确
语义，且复用需要清理 JIT 意图行的状态机回退，复杂度不成比例。CleanupRequired
无 state identity 时最终 Quarantined 并占用一个容量槽，是有界且可见的代价。
