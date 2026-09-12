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

生产事故（2026-09-12，rev 3 修订）：同一缺陷的另一形态——runner 曾被在线
观察到、已推进 `Idle` 之后才自注销。此时对账不再复查它，而在 demand ≥ 1 时
`excess=0`，幽灵 Idle 永久占住 effective capacity：job 等不到 runner（GitHub
侧 runner 实体已消失），VM 也永不销毁。因此对账范围扩展为 `WaitingOnline`
与 `Idle` 两类：Idle 代 runner 消失或下线即推进 `Retiring`（resumable
destroy，excess=0 也走完 remove+destroy 全链并同 tick 释放容量补建 runner）。

选择 tick 驱动的库存对账而非事件驱动：自注销没有可订阅的 web 事件面（scale
set message 流只携带 job 请求，不含 runner 生命周期），而 ownership 校验本来就
每 tick 拉取库存。对 WaitingOnline 才发起查询保证常态零额外调用；失败退化为
下一 tick 重试，与整套 reconcile 循环的 level-triggered 哲学一致。

readiness 超时（默认 15 分钟）统一处理"从未注册"与"注册后自注销"：库存视角
二者不可区分，且都应收敛到 terraform destroy。宽限期防止误杀 boot 中的 VM。

观察到在线瞬间后，rev 1–2 曾假设 Idle 最终经 excess retirement 销毁；rev 3
补上漏洞：Idle 也每 tick 复查库存。runner 仍在库且 online 才保持 Idle；消失
或 offline 即推进 Retiring——ephemeral runner 的这两个状态都意味着它不可能
再接受 job（service `Restart=no`，busy 中的 runner 仍报告 online 不会误判）。
代价是存在 Idle Generation 时每 tick 一次 `list_runners`，与 WaitingOnline
对账共用同一调用，常态无 Idle/WaitingOnline 时仍零额外流量。
