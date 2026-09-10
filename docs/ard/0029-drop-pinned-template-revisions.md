---
status: accepted
date: 2026-09-10
amends: [0028]
---

# Drop pinned template revisions entirely

`template_profile_ref` 移除 `{key, revision}` 形式，bare key（follow latest）成为
唯一 Fleet 引用语义。行为契约在 [spec 0023](../specs/0023-fleet-template-follow-latest.md)。

ARD-0028 引入双模式（pinned + follow）后不久即发现 pinned 模式没有真实用途：
操作者从不真正想钉住旧模板——pinned 只是"发布后忘了 re-pin"这一事故的受体。
双模式要求 UI 永远提供 pin 入口、spec 永远区分两种引用，却把全部价值留在内部
（resolved pin 本来就冻结在 Fleet Revision 行上，与 spec 里的 revision 字段无关）。

改为 follow-only 后：FleetSpec 的引用字段就是模板 key 字符串；UI 移除 revision
输入与 Load 流程；`resolve_template_ref` 的"requested != active"拒绝分支消失。
历史 spec_json 里的对象形式不改写（不可变账本），反序列化时归一化为 key——
旧 Fleet 自动成为 follower，无需迁移或重新 PUT。

Revision 机制本身保留为内部 fencing/审计骨架（spec 0023 §1）；删除的是用户可见的
pin 语义，不是一致性模型。Auth Profile 侧自始即为 follow（staged activation 的
同事务 retarget），本决定只影响 Template 引用。
