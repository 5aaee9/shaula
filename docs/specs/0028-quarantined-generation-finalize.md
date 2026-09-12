# Quarantined Generation Finalization

- Status: Accepted (2026-09-12); implementation and verification tracked separately.
- Decision: [ARD-0035](../ard/0035-finalize-quarantined-generations.md).
- Amends: [spec 0001](0001-shaula-runner-scale-set.md) 的 Resource Occupancy 与
  Quarantined 终局条款、[spec 0002](0002-fleet-http-control-plane.md) 的 Fleet
  mutation 边界、[spec 0025](0025-jit-mint-uncertainty-recovery.md) §2 的
  "真正的 quarantine GC 属于独立的 operator procedure" 占位。

## 1. Outcome

`Quarantined` Generation 代表 daemon 无法自证所有权或无法证明销毁安全的 ledger 记录：
ownership proof 缺失、state identity 缺失、runner 移除被 `JobStillRunning` 反复阻塞等。
它计入 Resource Occupancy 并阻塞 Fleet 的 template/input replacement，直到操作者介入。
此前没有收敛路径：Daemon 不会自动销毁一个无证据的 Generation，直接改库绕过认证、
并发控制与 audit——这不是 operator API。

本 spec 增加显式 operator finalize：`POST /api/v1/generations/{id}/finalize` 在操作者
以带外手段（平台 inventory、GitHub runner 列表、IaC state 检查）确认该 Generation 的
外部资源已不存在或不再运行后，将 ledger 行终结为 `Destroyed` 并释放 occupancy。
Finalize **从不执行任何远程效果**：不调用 GitHub、不调用 IaC、不删除 VM 或 runner
实体。它是"操作者已为外部一致性负责"的 ledger 证据提交，不是 destroy 的替代实现。

## 2. Endpoint contract

- 路由：`POST /api/v1/generations/{generationId}/finalize`；所需 scope 为
  `fleet.retire`——与 Fleet decommission 同级的高权限处置。
- Body 为 strict JSON：`{"reason": "<string>"}`，必填、非空白、上限 4096 字节；
  记录操作者声明的外部核实依据（例如 "VM 9001 absent on pve; runner absent in
  scale set inventory"）。缺省/空白/超长按 422 拒绝。
- `Idempotency-Key` 按 spec 0002 §5.2 的既有规则：同 key 同 body 的精确重放返回原
  结果，不同 body 冲突为 409。丢失响应后安全重试，不产生第二条事实。
- 目标 Generation 不处于 `Quarantined`：`Destroyed` 返回原结果的 200 replay 语义
  （见下）；其他状态按 409 `Conflict` 拒绝——finalize 不是通用 kill switch，
  活动中的 Generation 只能走正常 Retire/Destroy 通道。
- 未知 id 返回 404。Fleet tombstone 或 deletion marker 不阻塞 finalize：
  Decommission 必须能排空 quarantine 残留（spec 0002 §8 cleanup 不阻塞原则同样适用）。

## 3. Atomicity and evidence

Finalize 在单个 SQLite transaction 中提交：

1. 校验目标行为 `Quarantined`（CAS 于 state 列），推进到 `Destroyed`，写
   `updated_at`。同一 transaction 内由 `transition_allowed` 保证只接受
   `Quarantined → Destroyed` 这一条新增的合法边。
2. `runner_operations` 追加一条 `kind="Finalize"` 记录，携带 actor、reason 与
   完成时间，使操作日志/UI 可见处置依据。
3. `audit` 追加 `resource_kind="runner_generation"`、`action="finalize"`、actor、
   reason（detail）与 outcome=`accepted`。审计不携带凭据、JIT 或 state 内容。
4. 可选 idempotency 行与结果一并持久化；丢响应重试读回原 200/202。

并发 finalize 与 supervisor 推进（quarantine→…）由 state CAS 裁决：落后一方按
当前行状态返回 409 或 replay，不产生分叉终局。Finalize 不触碰 Fleet revision、
fence、session 或 desired；它只释放一条 Generation 行的 occupancy 占位。

## 4. Security boundary

- Finalize 是声明式证据入口：daemon 不验证、也无法验证操作者的外部核实真伪。
  这与其他 operator 处置（decommission、auth retirement）同级——其安全性来自
  OIDC actor 认证、`fleet.retire` scope 与持久化 audit，而非效果证明。
- 滥用 finalize 于外部资源仍存活的 Generation 是操作者责任：审计行保留 actor 与
  reason，泄漏的 VM/runner 由平台侧处置，不属于 daemon 收敛范围。
- Finalize 不缩短 runner 移除的 `JobStillRunning` 安全语义：活动中的 Busy/Idle
  Generation 仍必须先经过正常 retire gate；finalize 只接受 Quarantined 行。

## 5. Acceptance

- Quarantined Generation finalize 后状态为 `Destroyed`，occupancy 减少一，
  Fleet template/input replacement 门禁恢复可用。
- 非 Quarantined（Idle/Busy/Retiring/Destroying 等）目标的 finalize 返回 409，
  状态不变。
- 重复 finalize 同 key 同 body 返回原结果；不同 body 返回 409。
- 无 `fleet.retire` scope 返回 403；匿名返回 401。
- reason 缺失/空白/超长返回 422；audit 与 operation 记录含 actor 与 reason。
