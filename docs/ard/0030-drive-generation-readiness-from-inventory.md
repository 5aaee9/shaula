---
status: accepted
date: 2026-09-10
---

# Drive generation readiness from inventory

supervisor tick 增加 readiness 对账，实现 spec 0001 已设计但未驱动的两条
WaitingOnline 出口转换。协议和验收统一维护在
[spec 0024](../specs/0024-generation-readiness-reconciliation.md)。

生产事故（2026-09-10）：JIT ephemeral runner 完成唯一 job 后自注销，shaula 无
任何机制观察该事件，Generation 永久停留 WaitingOnline——既不进入只认
Idle/Retiring 的 retirement 通道（VM/ISO 泄漏），又持续计入 effective capacity
（后续 job 反复 assignment_withdrawn，因为系统认为已有可用 runner）。

选择 tick 驱动的库存对账而非事件驱动：自注销没有可订阅的 web 事件面（scale
set message 流只携带 job 请求，不含 runner 生命周期），而 ownership 校验本来就
每 tick 拉取库存。对 WaitingOnline 才发起查询保证常态零额外调用；失败退化为
下一 tick 重试，与整套 reconcile 循环的 level-triggered 哲学一致。

readiness 超时（默认 15 分钟）统一处理"从未注册"与"注册后自注销"：库存视角
二者不可区分，且都应收敛到 terraform destroy。观察到在线瞬间则先转 Idle 走
完整 retirement（remove_runner 对 AlreadyAbsent 幂等），两条路径收敛到同一
销毁效果。宽限期防止误杀 boot 中的 VM。
