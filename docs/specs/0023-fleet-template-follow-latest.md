# Fleet 跟随最新 Active Template Revision

- Status: Accepted (2026-09-10); implementation and verification tracked separately.
- Decision: [ARD-0028](../ard/0028-follow-latest-active-template-revision.md).
- Amends: [spec 0002](0002-fleet-http-control-plane.md) Fleet spec 与 replacement 门禁、
  [spec 0021](0021-default-template-updates.md) §4 的 Fleet pin 不变条款。
- Extends: [spec 0017](0017-automatic-template-activation.md) 自动激活后的传播范围。

## 1. Outcome

Fleet 的模板引用支持两种模式：**pinned**（`{key, revision}`，现有行为不变）与
**follow latest**（bare key，即不含 revision 的引用）。follower Fleet 在其 Template
Profile 激活新 Revision 后由 daemon 自动铸造携带新 pin 的 Fleet Revision，无需用户
重新编辑 Fleet。有资源占用的 Fleet 延后升级，在占用归零后的下一个 scan tick 自动
补齐。Revision、attestation、handoff CAS 等不可变与防护机制全部保留为内部实现；
用户只选择"跟随"或"钉住"。

## 2. Reference semantics

- `template_profile_ref` 为 bare key（JSON 字符串）时 Fleet 是 follower；为
  `{key, revision}` 对象时是 pinned。两种形式的准入校验不变：bare key 在准入时
  解析当前 Active Revision，无 Active 按原 `TemplateNotActive` 拒绝。
- follower 的 spec_json 原样保存 bare key 形式；每次升级只更新 Fleet Revision 行的
  resolved pin（template_revision、artifact digest、attestation id）。Generation
  仍从其准入时的 Fleet Revision 冻结完整上下文，per-generation 不可变性不变。
- 对 follower 的 PUT 语义不变：spec 未变化按既有规则 NoOp；reference 未变化时保留
  既有 pin（升级不经过 PUT，避免 PUT 成为隐式升级通道）。follower 改为 pinned 或
  更换 key 是普通 reference 变化，走原零占用门禁。
- follower 关系由最新 Fleet Revision 的 spec 判定：bare key 即跟随。从 Exact 改为
  bare（或反向）本身构成 spec 变化，按原规则铸造新 Revision。

## 3. Level-triggered cascade

升级由 daemon 的周期 scan 驱动，不订阅 publish 事件：

1. 每个 scan tick 枚举非 tombstone、无 deletion marker 的 Fleet，解析其最新 spec；
   follower 的 pinned template revision 与 Profile 当前 Active 不一致即为 lag。
   Profile 无 Active、 Fleet 正在删除/退役时跳过。
2. lag 的 follower 在 **资源占用为零** 时升级：升级前按新 pin 的 parameter schema 与
   fleet input policy 重新校验 Fleet 的现有 inputs（与手工 replacement 同一层门禁）；
   不兼容时本 tick 跳过并记 WARN，不修正或扩大授权。升级复用与 Fleet PUT 相同的准入材料
   解析与 `commit_fleet_mutation` 提交路径（mutation fence CAS、独占 effect gate、
   audit、outbox `fleet.change`），actor 记为 `shaula-daemon`，Change kind 为
   `Replace`。占用非零时本 tick 跳过，下一 tick 重试——繁忙 Fleet 在 runner
   排空后自动升级，不需要操作者回来重试。
3. 解析或提交失败（fence 竞争、Profile 被撤、artifact 不可用）记 WARN 并在下一
   tick 重试；单个 Fleet 的失败不影响其他 Fleet 或 scan 循环本身。
4. cascade 铸造的 Revision 与手工 PUT 的 Revision 在 reconcile、supervisor 重建
   （R10-01 缓存键）与审计中不可区分。级联不会在 Active 未变化时重复铸造：
   pin 与 Active 一致即无 lag。

## 4. Auth profile 的既有先例

GitHub Auth Profile 的 Revision 提升（promotion）早已在同一事务内 retarget 所有
存活 Fleet 的 handoff desired（spec 0005 §6 的 staged activation）——Auth 侧天然
是 follow latest。本 spec 把 Template 侧对齐到同一策略，Auth 行为不变。

## 5. UI 约定

Fleet 表单默认 follow latest：选择模板即提交 bare key 引用。Pinned revision 输入与
"Load template revision" 移入高级设置，仅用于显式钉住。Fleet 详情与列表展示当前
resolved pin，并对 follower 标注跟随状态（含"等待资源排空后升级"的滞后提示）。

## 6. Acceptance

- follower 在新模板 Revision 激活后一个 scan 周期内自动铸造新 Fleet Revision；
  审计与 Changes 可见 actor=`shaula-daemon` 的 Replace。
- 占用非零的 follower 不升级；最后一个 Generation Destroyed 后自动补升级。
- pinned Fleet 在任何 publish/activation 下保持原 pin。
- 服务重启不丢失升级义务（level-triggered 不依赖事件）；并发 publish 与 cascade
  由 mutation fence CAS 拒绝落后一方，不产生分叉 head。
