# Fleet 跟随最新 Active Template Revision

- Status: Accepted (2026-09-10); implementation and verification tracked separately.
- Decision: [ARD-0028](../ard/0028-follow-latest-active-template-revision.md),
  [ARD-0029](../ard/0029-drop-pinned-template-revisions.md)（follow-only 化）.
- Amends: [spec 0002](0002-fleet-http-control-plane.md) Fleet spec 与 replacement 门禁、
  [spec 0021](0021-default-template-updates.md) §4 的 Fleet pin 不变条款。
- Extends: [spec 0017](0017-automatic-template-activation.md) 自动激活后的传播范围。
- Revision 2 (2026-09-10, ARD-0029): 移除 pinned 模式，follow latest 成为唯一语义。
- Revision 3 (2026-10-15): 跟随不再等待零占用；新 Revision 立即用于新 job，既有 Generation 保持原 pin（§3）。

## 1. Outcome

Fleet 的模板引用只有 **follow latest** 一种模式：`template_profile_ref` 是 bare key
（模板 Profile key 字符串）。Fleet 在其 Template Profile 激活新 Revision 后由 daemon
自动铸造携带新 pin 的 Fleet Revision，无需用户重新编辑 Fleet。有资源占用的 Fleet
同样在下一个 scan tick 升级：新 job 立即使用新 Revision，运行中的 Generation 保持原 pin。Revision、attestation、handoff
CAS 等不可变与防护机制全部保留为内部实现，用户侧不存在 revision 选择。

## 2. Reference semantics

- `template_profile_ref` 只接受 bare key（JSON 字符串）。准入时解析当前 Active
  Revision，无 Active 按原 `TemplateNotActive` 拒绝。
- ARD-0029 之前落库的 `{key, revision}` 对象形式在读取时归一化为其 key——
  spec_json 的历史字节不改写（不可变账本），解析层归一化后即 follower。resolved
  pin 永远只存在于 Fleet Revision 行的 template_revision/artifact/attestation 列。
- spec_json 原样保存 bare key；每次升级只更新 Fleet Revision 行的 resolved pin。
  Generation 仍从其准入时的 Fleet Revision 冻结完整上下文，per-generation
  不可变性不变。
- 单模板 Fleet PUT 在模板 key **及 `template_inputs` 均未变化**时保留原 resolved
  pin；原样重提按既有规则 NoOp，仅修改容量等非路由字段也不会隐式升级模板。
- **修改 `template_inputs`** 是显式 replacement：即使模板 key 相同，也解析该
  Profile 当前 Active revision，并按新 revision 的 input policy/schema 校验。
  这让新版 input contract 的输入可被接纳，但不绕过原零占用、ownership 与 mutation
  fence 门禁，也不改写既有 Generation。更换模板 key 同样解析当前 Active 并走
  replacement 门禁；未修改 key/inputs 时的自动追赶仍由 §3 cascade 完成。
- 共享池 Fleet 的 `template_pool_ref` 另由 [spec 0037 §4](0037-shared-template-pool-resource.md#4-fleet-reference-and-admission)
  定义：同 pool key 的 PUT 保留已冻结 pool revision，成员 inputs 属于独立池资源；
  不把上述单模板 inputs replacement 规则套用为共享池的隐式升级通道。

## 3. Level-triggered cascade

升级由 daemon 的周期 scan 驱动，不订阅 publish 事件：

1. 每个 scan tick 枚举非 tombstone、无 deletion marker 的 Fleet；其最新 spec 的
   resolved pin 与 Profile 当前 Active 不一致即为 lag。Profile 无 Active、Fleet
   正在删除/退役时跳过。
2. lag 的 Fleet 在下一个 scan tick 升级，**不等待资源占用归零**：升级前按新 pin 的 parameter schema 与
   fleet input policy 重新校验 Fleet 的现有 inputs（与手工 replacement 同一层门禁）；
   不兼容时本 tick 跳过并记 WARN，不修正或扩大授权。升级复用与 Fleet PUT 相同的
   `commit_fleet_mutation` 提交路径（mutation fence CAS、独占 effect gate、audit、
   outbox `fleet.change`），actor 记为 `shaula-daemon`，Change kind 为 `Replace`。
   同一 Profile 升到更新 Revision 不是 replacement，不走零占用门禁：每个 Generation
   已在准入时冻结自己的模板 revision、artifact、bindings 与 inputs，Destroy 也只用
   这些 pin，所以移动 Fleet head 不改变任何运行中的 runner。升级后新 Create 使用新
   Revision；旧 Revision 的 Idle Generation 即使仍有需求也被退役并按新 Revision 补齐，
   Busy Generation 先跑完当前 job（ephemeral runner 不会被复用）。更换模板 key、
   inputs、auth 或 pool 仍是 replacement，零占用门禁不变。
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

Fleet 表单与 API 均不存在 revision 输入：选择模板即提交 bare key 引用，inputs 始终
从当前 Active revision 的 input contract 加载。Fleet 详情与列表展示当前 resolved
pin（resolved revision、artifact digest），滞后只持续到下一个 scan tick。

## 6. Acceptance

- Fleet 在新模板 Revision 激活后一个 scan 周期内自动铸造新 Fleet Revision；审计与
  Changes 可见 actor=`shaula-daemon` 的 Replace。
- 占用非零的 Fleet 同样在下一个 scan tick 升级；既有 Generation 的 pin 不变，旧
  Revision 的 Idle Generation 被退役替换，Busy Generation 跑完当前 job。
- 新 pin 的 inputs policy/schema 与 Fleet 现有 inputs 不兼容时跳过并 WARN，不升级。
- 模板 key/inputs 未变时，原样 PUT 和仅容量变更保留旧 pin，即使 Profile 已有新 Active。
- 同 key 修改 inputs 时解析新 Active 并重新校验；非法输入拒绝，资源仍占用时不得
  提交 replacement，成功提交后旧 Generation 的原 pin 仍不可变。
- 携带历史 `{key, revision}` spec 的 Fleet 读取归一化为 bare key 并正常跟随。
- 服务重启不丢失升级义务（level-triggered 不依赖事件）；并发 publish 与 cascade
  由 mutation fence CAS 拒绝落后一方，不产生分叉 head。
