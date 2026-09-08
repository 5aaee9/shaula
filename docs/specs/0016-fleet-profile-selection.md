# Fleet Profile Selection

- Status: Accepted (2026-09-08); implementation and verification are tracked separately.
- Decision: [ARD-0020](../ard/0020-load-fleet-profile-choices-from-registry.md).
- Extends [operator UI](0008-embedded-web-ui.md) and
  [visual Template inputs](0014-visual-template-inputs.md).

## 1. Outcome

创建或编辑 Fleet 时，GitHub authentication profile 和 Template profile 均提供明确的
下拉选择，选项来自服务端当前列表。用户不需要知道或手写 Profile key，也不需要先输入
模板名称再点击 Load template。Template Source 与已发布 Profile 的区别保持不变：
默认 Docker/Kubernetes 来源不是可直接引用的 Profile。

## 2. Read contract and eligibility

- 复用 `GET /api/v1/github-auth-profiles` 和 `GET /api/v1/template-profiles`，响应均为
  `{profiles: [...]}`，当前不分页。分别要求 `auth.read` 和 `template.read`。
  不增加 API、数据库实体、GitHub 探测或平台请求。
- Template 列表不得吞掉单项存储/读取错误后返回不完整的成功列表；只跳过读取时已不存在
  的项，其余基础设施错误传播为失败，成功结果按 key 排序，与 authentication 列表一致。
- 打开表单时读取/刷新列表，提供各自的显式刷新入口，并沿用页面内 query cache。
  使用既有 same-origin session、private/no-store、401 清理及权限规则。
- 选项以 key 排序，显示 key、可用 Active revision 和状态；authentication 可附带类型。
  只有有 `activeRevision` 且不在 `Retiring`/`Retired` 状态的 Profile 可新选。
  desired candidate 尚未 Active 不影响旧 Active 的选择资格；不能仅允许 `status=Active`。
  auth binding 的 `Unknown` health 也不能作为隐藏或禁用 Profile 的依据。
- 没有 Active 或正在 retirement 的项仍可见，但禁用并说明原因。初始 placeholder 为空，
  不自动选择第一项或唯一项，不保留手写 key 或 datalist 作为默认输入方式。
- 选择器不在浏览器重做 Target policy 或 GitHub 权限判断；Profile 可选不代表它一定
  授权当前 owner/repository。最终完整 Fleet PUT admission 保持权威。
- authentication 提交已有的字符串 `auth_profile_ref`，由服务端事务解析当前 Active；
  UI 显示的 revision 是观测信息，不承诺 authentication 的 exact revision 固定。

## 3. Loading, failures and original references

- 两个列表分别显示加载中、无 Profiles、有 Profiles 但无可用项、缺少读取权限和读取错误。
  `403` 不伪装为空列表；临时失败提供重试，已有缓存不能掩盖最近一次读取失败。
- 缺少对应读取权限时不发出该列表请求。新建或更换该引用需要成功的列表结果，且当前
  选择仍在可用列表中；错误、移除、retirement 或无 Active 时阻止新引用提交。
  刷新进行中可保留上一次成功结果；刷新失败或资格变化后明确提示并保留选中的 key。
- 编辑原 Fleet 时，引用即使不在列表或无读取权限也继续显示为当前值，不清空、不替换。
  未改变的原始引用不因列表失败而阻止容量等其他字段的提交；服务端仍可按 admission 拒绝。
  没有权限时该选择器只读。有权限且列表恢复后可以显式选择一个可用替代项。
- 保留原 authentication key、Template wire reference、resolved pin、原 inputs、Fleet ETag
  和 MutationAttempt 语义。只刷新列表不改变上述快照，不自动切换 Profile 或 revision。
  编辑时可通过取消切换回到原先保留的引用，不能因原选项后来不可用而困在新草稿中。

## 4. Template selection and draft continuity

- 选择 Template key 后立即读取该 Profile 详情和 current Active exact input-contract。
  详情仍需检查 retirement 与 Active，避免使用选择列表与详情之间变化的不可用状态。
  显示实际加载的 key/revision；新建和更换引用以展示的 `{key, revision}` 提交。
- 新选择清除之前输入的 revision 候选，捕获所选模板当时的 Active。Advanced 中显式
  revision 的既有校验继续生效；用户可以用 Load latest Active 明确重读当前选项。
- 网络请求失败、快速连续选择、取消或过时响应不得覆盖最后一次选择和原 inputs。
  切换会丢弃已填写 inputs 时继续要求明确确认；取消后恢复此前选中项与完整草稿。
- 列表刷新或 Active promotion 不触发 input-contract 自动重载。同 key 新 revision 必须
  显式加载/复核；提交期间的 promotion/retirement 由服务端校验，并保留失败表单。
- 没有成功加载 contract 时不得把输入视为 `{}`。编辑旧 Fleet 且原 contract 不可读时，
  保持 spec 0014 的引用和 inputs 锁定保留行为，仍可修改其他 Fleet 字段。
- Template inputs 继续在主表单展示，不放入 Advanced settings。

## 5. Acceptance

1. 打开 Fleet create/edit 时调用有权限的两个真实列表接口，控件使用服务端 key；初始不预选，
   鼠标及键盘均可选择，标签可访问，390px 窄屏不横向溢出。
2. 覆盖空列表、全部不可选、加载、403/503、刷新恢复；有旧 Active 的非 Active desired
   status 仍可选，Retiring 即使有 activeRevision 也不可新选。
3. 新建选择 authentication key 和 Template 后无需 Load template，即显示 inputs；最终
   PUT 保留 authentication 字符串、Template exact revision 和无损 inputs。
4. 列表移除选中项、背景刷新、输入加载乱序、promotion、失败/取消切换不丢草稿或偷偷换版本。
   新引用不可用时禁止提交，未改引用的旧 Fleet 可保留原值提交其他变更。
5. 缺少 auth.read/template.read 不发送对应列表请求或提供手写绕过；保留既有 session
   renewal、terminal 401、ETag/412 与幂等回归。
6. 实际 HTTPS/OIDC 浏览器回归确认列表、选择和 contract 接线；mock 浏览器测试覆盖失败
   与竞争场景。前端 build/lint/format/browser 和受影响 Rust 门禁通过后记录证据。

实现顺序：复用列表 queries → 选择器及资格/错误状态 → Template 自动加载接线 →
迁移现有表单 fixtures 并增加场景回归 → 独立实现 review 与最终验证。
实现、部署和验收记录由 [implementation status](../IMPLEMENTATION_STATUS.md) 维护。
