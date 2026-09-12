# Generation Readiness 对账（WaitingOnline 与 Idle 驱动）

- Status: Accepted (2026-09-10; rev 3 2026-09-12); implementation and verification tracked separately.
- Decision: [ARD-0030](../ard/0030-drive-generation-readiness-from-inventory.md).
- Implements: [spec 0001](0001-shaula-runner-scale-set.md) 状态机的
  `WaitingOnline --> Idle: GitHub inventory online`、
  `WaitingOnline --> CleanupRequired: readiness timeout` 与
  `Idle --> Retiring: ephemeral runner 自注销或下线` 三条转换的驱动方。

## 1. Outcome

每个 Fleet 的 reconcile tick 在容量收敛之前执行一次 **readiness 对账**，用一次
scale set 库存查询（`list_runners`）判定处于 `WaitingOnline` 与 `Idle` 的
Generation。

对 `WaitingOnline`：

1. 库存中存在 id 与 runner name 都匹配、状态为 `online` 的 runner → 推进
   `Idle`。Idle 即进入现有 retirement 通道（`retire_excess` 的
   needs_removal 集合），后续按 excess 正常销毁。
2. 未匹配到 runner（从未注册、或注册后因一次性 JIT 完成自注销而消失）且
   `now - created_at` 超过 readiness 超时 → 推进 `CleanupRequired`，进入现有
   cleanup/destroy 通道（见 §2.1）。
3. 未匹配但在宽限期内（默认 15 分钟，daemon 常量）→ 保持 `WaitingOnline`，
   继续等待注册。匹配到但状态非 online（boot 阶段的 offline）同样等待。

对 `Idle`（rev 3）：

4. 库存中存在 id 与 runner name 都匹配且状态为 `online` 的 runner → 保持
   `Idle` 不动。
5. 库存中**不存在**该 runner，或存在但状态非 `online` → 推进 `Retiring`，走
   正常 destroy 链（`remove_runner` 幂等，`AlreadyAbsent` 视为成功，
   `JobStillRunning` 门禁在 runner 实际仍有任务时权威阻塞重试）。JIT runner
   是 ephemeral 一次性实体：自注销（消失）或 agent 死亡无重启（runner
   service `Restart=no`，表现为 offline）都意味着它不可能再接受任何 job。
   库存 status 只有 online/offline 两值，busy 中的 runner 仍报告 `online`，
   不会误判。

没有任何 `WaitingOnline` 或 `Idle` Generation 时本步骤零 API 调用；库存查询
失败（网络、权限）记 WARN 并整体跳过，下一 tick 重试——对账是
level-triggered 的，不依赖事件，重启不丢失义务。

## 2. 为什么超时路径覆盖一次性 runner

JIT 生成的 runner 是 ephemeral：执行完一个 job 后自行注销并从库存消失。在线
窗口可能短于一个 tick，因此"从未出现"与"出现后消失"在库存视角无法区分——
两者都表现为"当前不在库存中"。readiness 超时统一处理两类：

- 超时仍未注册 → 资源处于不可证明状态，进入 CleanupRequired 由 terraform
  destroy 收敛（与 spec 0001 §11.2 的 fail-closed 清理一致）。
- 注册、执行、自注销 → 同样由超时驱动进入 CleanupRequired，销毁 VM 与 ISO。

若 tick 恰好观察到在线瞬间，Generation 先进 `Idle`，由 retirement 通道完成
带 `remove_runner`（AlreadyAbsent 视为成功）的完整销毁——两条路径最终都
收敛到同一个 destroy 效果。

## 2.1 CleanupRequired 的 destroy 驱动（rev 2，2026-09-10 事故修订）

原实现把 CleanupRequired 交给不存在的"现有 cleanup 通道"：60 秒后 quarantine_stale
直接转隔离，VM/ISO 永不销毁（同日 23:24 矩阵事故：8 台 VM 在 Proxmox 泄漏）。
修订为：supervisor tick 对龄越过 grace 的 CleanupRequired Generation **先驱动
`remove_runner`（有 runner id 时；AlreadyAbsent 幂等）再进入 Retiring →
DestroyPending → Destroying → Destroyed 的正常 destroy 流**；仅当 destroy 无法
开始或无法证明终止（state identity 缺失、provenance 缺失、JobStillRunning 门禁
阻塞）时按原 60 秒规则 quarantine。即 quarantine_stale_cleanup 的语义从"60 秒
后隔离"改为"60 秒后先尝试 destroy，不可证明才隔离"。

## 2.2 Idle 的幽灵 runner（rev 3，2026-09-12 事故修订）

rev 1–2 只覆盖 `WaitingOnline`：runner 一旦被观察到 online 推进 `Idle` 后就
不再被库存对账复查。2026-09-12 事故：fleet `pve-builder-tyo` 的 runner
239600 被 tick 观察到 online 推进 `Idle`，随后完成其唯一 job 并自注销；此时
下一个 job 已分配到 scale set 使 `demand=1`，幽灵 `Idle` Generation 仍计入
effective capacity 使 `excess=0`——retirement 通道永不轮到它（只处理超额），
capacity 也不再创建新 runner，job 永远等不到 runner、VM 永久泄漏，形成
死锁。

修订为：`Idle` Generation 同样纳入每 tick 的库存对账（§1 规则 4–5）。runner
消失或下线即推进 `Retiring`——`Retiring` 是 resumable destroy 状态，即使
`excess=0` 也会在当 tick 走完 remove_runner → DestroyPending → Destroying →
Destroyed 全链；effective capacity 同时释放，同一 tick 即可为该 demand 创建
替代 runner。唯一代价是存在 Idle Generation 时每 tick 多一次
`list_runners` 查询（与 WaitingOnline 对账共用同一次调用）。

## 3. Ordering 与并发

- 对账位于 listener 就绪检查之后、容量收敛之前：同 tick 内推进的 Idle 可立即
  被 `retire_excess` 拾取；推进 CleanupRequired 立即从 effective capacity
  移除，避免泄漏的 WaitingOnline 长期占用容量、阻塞新 job 的 runner 创建。
- 推进使用现有 `generation_advance` 的状态机校验与 CAS；并发 tick 的重复推进
  被状态机拒绝（Idle→Idle、CleanupRequired→CleanupRequired 非法），失败
  记 debug 即可。
- 对账不读取或修改 JIT/绑定等秘密材料；它只消费库存的 id/name/status。

## 4. 库存端口

`RunnerRef` 增加 `status`（`"online"` / `"offline"`，未知值按非 online 对待）。
该字段来自 GitHub inventory 的既有响应体，不新增请求。

## 5. Acceptance

- WaitingOnline + 库存 online 匹配 → 一个 tick 内推进 Idle。
- WaitingOnline + 库存缺失 + 超过超时 → 推进 CleanupRequired，随后按现有
  cleanup 通道销毁；宽限期内不推进。
- Idle + 库存缺失或 runner 非 online → 一个 tick 内推进 Retiring，即使
  excess=0 也走完 remove_runner + terraform destroy 全链；runner 仍 online
  的 Idle Generation 不受影响。
- 无 WaitingOnline 或 Idle Generation 时无额外 GitHub 调用；库存调用失败仅
  WARN。
- 卡死的 WaitingOnline 与幽灵 Idle 都不再长期计入 effective capacity（新
  job 能获得新 runner），不再泄漏 VM/ISO。
