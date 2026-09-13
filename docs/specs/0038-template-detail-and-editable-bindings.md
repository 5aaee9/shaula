# Template Detail Page and Editable Non-Secret Bindings

- Status: Proposed.
- Decision: [ARD-0038](../ard/0038-template-detail-page-and-editable-non-secret-bindings.md).
- Amends: [spec 0005 §5.2](0005-profile-http-control-plane.md) read projection and
  [spec 0021 §4–§5](0021-default-template-updates.md) update; extends
  [spec 0008](0008-embedded-web-ui.md) Templates UI.

## 1. Outcome

Template 详情成为独立路由 `/templates/{key}`，展示 Profile 元数据和
**非敏感** binding 值；敏感 binding 仍以只写方式保存，任何读取仅暴露其存在性。
**Update** 在沿用默认来源的基础上允许编辑非敏感 binding 与 Fleet input policy；
敏感字段留空或省略即保留原值，提交新值才替换。Templates 列表的行内展开与重复的
Update 入口收敛为每个 Profile 一个 Update 动作。

非敏感 binding 可编辑是 spec 0021 的扩展：原先 Update 一字不差地复用整个不可变
binding 集，改一个非敏感字段也要重输全部 secret。本规范让服务器按字段解析提交值，
敏感字段保留或替换，非敏感字段保留或更新，合并结果仍先经 schema 校验再入库。

## 2. Schema-driven read projection

Spec 0005 §5.2 原先对所有读取只暴露 `bindings_present: boolean`，连非敏感值也一并
隐藏。本规范改为按 artifact bindings schema 的 `sensitive` 标注拆分：

- Revision 读取返回 `bindings` 对象，逐项包含：
  - `sensitive: false` 字段返回其原始 JSON 值；
  - `sensitive: true` 字段返回一个存在性标记对象 `{"sensitive": true, "set": <bool>}`，
    绝不返回值、前缀、后缀、hash 或长度。
- `bindings_present` 保留用于兼容；无 binding 的 Revision `bindings` 为 `null` 或
  缺省，schema 未知/无标注字段一律视为敏感、不返回值。
- 该投影对 Profile GET / list / status / revision / attestation 相关读取一致生效，
  与调用方写权限无关。secret 字节仍只存在于不可变 Revision 与发往 IaC 子进程的
  精确 Revision 交接中，绝不进入响应、日志、span、metric 或 audit。
- `sensitive: true` 标注保护整个顶层成员/子树；嵌套混合敏感对象整体保护，不按
  JSON-path 拆分读取。发布者不得把含凭据的字段标注为非敏感（spec 0005 §5.2 不变）。

## 3. Editable bindings on Update

`POST /api/v1/template-profiles/{profileKey}/updates` 的请求体增加可选
`bindings` 对象。省略 `bindings` 时行为不变（沿用 base Revision 的全部绑定）。
携带时它表示期望的**完整** binding 集，由服务器对 base Revision 的已存绑定逐字段
解析：

- `sensitive: false` 字段：提交的值替换；省略则保留 base 中的已存值。
- `sensitive: true` 字段：省略、或提交 keep 标记 `null`，即保留 base 中的已存值；
  提交非 `null` 的合法新值则替换。`null` 是“保留”语义哨兵，绝不作为值写入 Revision。
- 合并结果必须先通过目标 artifact bindings schema 校验才进入 admission；非敏感
  编辑不能静默丢弃 required 字段，合并后仍缺 required 或不合 schema 按 422 拒绝。
- `bindings` 的非对象、`null` 顶层、未知字段或试图把 `sensitive` 字段当普通值
  回读，均按 strict JSON / validation 规则拒绝。Update 仍不接受 `platform`、
  `bindings_contract` 等第二权威字段。

幂等、预检与 source 关联规则沿用 spec 0021 §5，新增两点：

- Update request identity 包含 `bindings` 是否提供及其规范值；secret 相等性仍在受
  保护内存中与已提交 Revision 比较，幂等记录不含 secret 字节或可由 secret 推导的
  verifier。同一 key 重放返回原结果，变更后冲突。
- NoOp 判等在 artifact、engine、source key、policy 之外纳入**合并后**的 binding 集：
  仅非敏感值变化也会创建新 Revision，与 secret 变化同等处理。

## 4. Templates UI

- Templates 列表的模板名称链接到 `/templates/{key}` 详情路由，不再用 `?key=` 行内
  展开。详情页展示 platform、bindings contract、status、active/desired revision，以及
  非敏感 binding 值；敏感 binding 只显示 “Configured” 标记。
- 详情页提供两个意图明确的动作：**Update**（进入 `/templates/{key}/update`，沿用默认
  来源并可编辑非敏感 binding 与 policy）与 **New revision**（发布新 artifact）。列表行
  保留单个 Update 动作；详情不再重复三个按钮。
- Update 表单在非敏感字段预填当前值、敏感字段留空并提示 “leave blank to keep”；提交时
  空敏感字段省略，非空则携带新值。非敏感字段始终回显可编辑。
- 详情路由直接可打开、刷新和登录后返回；非法 key、缺失 Profile、权限不足、
  Retiring/Retired 均按现有错误与权限门槛展示。

## 5. Acceptance

- Revision 读取返回非敏感 binding 值与敏感字段的存在标记，任何读取、日志、span、
  metric 或 audit 均不泄露 secret 值/前缀/后缀/hash/长度；schema 未知字段一律敏感。
- Update 携带 `bindings` 时逐字段合并：非敏感值可改，敏感值留空/省略保留、提交新值
  替换；合并后经 schema 校验，缺 required 或非法值按 422 拒绝且不产生部分写入。
- Update identity/NoOp 覆盖合并后 binding 集；secret 相等性只在受保护内存比较，
  幂等记录不含 secret 或 verifier；同 key 重放返回原结果，变更冲突。
- 详情路由 `/templates/{key}` 展示非敏感 binding 与元数据，敏感项仅 “Configured”；
  列表行名跳转详情，详情页只有 Update 与 New revision 两个动作。
- 浏览器测试覆盖非敏感回显、敏感字段 “leave blank to keep”、提交新 secret、
  仅非敏感变更产生新 Revision、以及权限/retirement/冲突草稿门槛。
