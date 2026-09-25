---
status: accepted
date: 2026-09-25
---

# Destroy generations that never started a Create apply

决定：没有 Create `ApplyStarting` 记录、且其 Runner Registration 已证明移除或不存在的
Runner Generation，直接推进到 `Destroyed` 并释放 occupancy，不执行 Terraform destroy，
也不进入 `Quarantined`。Registration 无法证明（查找歧义、查找失败、移除被安全门阻塞）时
仍然 Quarantine。该规则对 GitHub 与 Forgejo 两个 Runner Backend 相同，修订
[spec 0025 §2](../specs/0025-jit-mint-uncertainty-recovery.md#2-状态与占用)。

理由：`ApplyStarting` 是 at-most-once apply 的持久化前置记录（spec 0004 §6），在任何
mutating apply 启动之前提交；它也是后续 Destroy 复核的 original provenance。因此“没有该记录”
可以证明 Runner Resource 从未被创建。Quarantine 的定义是“无法证明”资源身份或安全销毁条件，
这里可以证明，隔离就不必要。原规则（spec 0025 §2）会让每一次 PlanFailed、JIT mint 未落地、
准入后 fence 移动都永久消耗一个容量槽，直到操作者 finalize（ADR-0035）。Forgejo Pool 切片
（spec 0026）已经按本规则实现；本决定也是统一 Runner Operation 的前提，使 Runner Backend
不改变 Runner Operation 的规则（CONTEXT.md）。

被拒绝的方案：

- **两个 backend 都按 spec 0025 §2 隔离**：最保守，但 plan 失败或模板错误会让 Fleet 快速
  占满 `max_runners` 并停止创建。代价由 Quarantine 承担，而这里本来就有证明可用。
- **按失败原因区分**（只有 PlanFailed、JIT 未落地走 `Destroyed`）：规则更细，还需要在账本中
  持久化失败原因，但证明力并没有比“没有 ApplyStarting”更强。

后果：

- 本规则的安全性完全依赖“ApplyStarting 先于 apply spawn 持久化”这一不变量。如果该不变量
  出现缺陷，泄漏的资源不会再以 `Quarantined` 的形式显示出来。因此统一后的 Runner Operation
  测试矩阵必须覆盖该不变量本身。
- 已经处于 `Quarantined` 的历史 Generation 不会被自动改判，仍然走 ADR-0035 的 finalize。
