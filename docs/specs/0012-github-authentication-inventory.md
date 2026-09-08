# GitHub Authentication Inventory Specification

Status: Accepted (2026-09-08); implementation and verification are tracked separately.

## 1. Problem and scope

Operator 打开 `/auth` 时必须能够发现 Shaula 中已经保存的 GitHub authentication，
不必先知道 Profile key。当前页面只有按 key 打开详情的表单；
`GET /github-auth-profiles` 已出现在 spec 0005，但没有接入 HTTP router。
因此 `indexyz-org` 可以通过详情 URL 读取，却不会出现在入口页。

本规范定义 Auth Profile 的 collection read 和列表 UI，理由见
[ADR-0016](../ard/0016-list-authentication-connections-from-the-profile-registry.md)。
页面中的 **connection** 是 GitHub Auth Profile 的显示名称，不是新的持久实体，
也不是 OIDC 登录会话、Account Binding 或 GitHub App 上的所有 installations。

本规范补充 [spec 0008](0008-embedded-web-ui.md)，并明确
[spec 0005 §4](0005-profile-http-control-plane.md#4-http-interface) 的 Auth collection 契约。
[spec 0011](0011-multi-account-github-authentication.md) 继续拥有 active/desired policy、
bindings 与 health 的含义；凭据发布、轮换、retirement 和 conditional writes 不变。

## 2. Collection contract

`GET /api/v1/github-auth-profiles` MUST：

- 使用现有 OIDC/session middleware；未认证返回 `401`，缺少 `auth.read` 返回 `403`。
  只有 `auth.read` 的 Operator 也能查看列表；不要求 write 或 retire 权限。
- 从 Shaula 的持久 Profile registry 枚举所有现存 Auth Profiles，包括 legacy GitHub App、
  PAT、v2 多账户 App，以及尚未激活或处于 retirement 的记录；不得仅从 Fleet 引用、
  浏览器历史或已访问的详情拼出列表。
- 返回 `200 application/json` 和 `{ "profiles": [...] }`。没有记录时返回空数组。
  每个 key 最多出现一次，按 key 升序排列。
- 每项复用单 Profile GET 的脱敏表示：key/incarnation、status、kind、
  desired/active Revision、credential presence，以及各自 Revision 的 policy、bindings
  和已有的非 secret metadata。读取列表与详情时发生并发更新可能使两次结果不同；
  本接口不承诺全部 Profiles 的跨记录事务快照。
- 遵循现有 private/no-store 响应策略。不得返回 private key、PAT、installation token，
  或凭据值的前后缀、长度、摘要。GET 不调用 GitHub，不创建或修改 Profile、installation、
  binding、validation job、Fleet 或授权范围。
- 基础设施读取失败时返回错误，不能省略失败项后返回成功、也不能把失败表现为无连接。

本增量采用**完整 collection response**，不提供 cursor、limit 或静默截断。
它枚举的是本地配置的认证 Profiles，不是每个账户的仓库集合。
这明确替代 spec 0005 接口表原先未展开的 Auth collection “Paginated”描述；
Template/Fleet collection 的要求不在本次改动范围内。
将来若 Profile 数量需要分页，应先定义分页与搜索契约并同时更新 API/UI，
不得让旧客户端无提示地只显示第一页。完整读取的开销随 Profile 数量增长是本方案的已知代价。

Collection 响应不为行提供写入凭证。编辑或 retirement 必须读取单 Profile，
继续捕获详情响应的 `Shaula-Resource-Version` / strong ETag，不能用列表中的 Revision
合成 `If-Match`。

## 3. UI behavior

具有 `auth.read` 的 Operator 打开 `/auth` 后 MUST 自动请求 collection 并展示列表。
没有选中行是正常初始状态，不能用 “No authentication profile selected” 代替库存。

每行 MUST 显示：

| 内容 | 语义 |
| --- | --- |
| Connection | 稳定 Profile key；可点击打开详情 |
| Credential type | GitHub App / Personal access token；缺失时显示占位符 |
| Status | 持久 Profile 状态；不能推导为所有目标的实时可用性 |
| Active targets | active Revision 的 typed target policy 或 legacy exact allowlist |
| Active revision | 生效版本；没有 active head 时显示占位符 |
| Desired revision | 当前 desired head，允许与 active 不同 |

Active targets MUST NOT 展示 desired Candidate 的新增范围为已生效；
首个 Candidate 尚未激活时显示占位符。动态个人/组织仓库 selector 保留规则语义，
不能枚举仓库快照替代规则。逐账户 health 和 Candidate 原因继续放在现有详情中，
`Active` 不能取代 health 或 Fleet access 结果。

列表 MUST 支持：

- 对 key 进行忽略大小写、去除首尾空白的本地搜索。
- 手动刷新和每 10 秒的后台刷新；通过现有 query invalidation 在创建、更新、retirement
  被接受后刷新列表。仅重读 registry，不触发 GitHub 验证或其他 mutation。
- 点击行打开 `/auth?key=<encoded-key>`，保留列表并显示选中状态。
  直接访问现有详情链接仍可用；collection 失败不应阻止有权限的独立详情读取。
- 分别展示 loading、真正空列表、无搜索匹配、读取失败；错误提供重试。
  刷新失败不能继续把旧列表呈现为本次成功结果。
- 现有键盘、表格语义、移动端横向滚动和 scope 控制。
  没有 `auth.read` 时不发送 collection 或 detail 请求，显示权限说明。

搜索、选择和返回的非 secret metadata 只使用现有页面/query 状态；不新增浏览器持久存储。
会话失效继续使用统一的卸载/缓存清理路径。

## 4. Acceptance

1. 在真实 Router → ControlPlane → SQLite 测试中保存 legacy/PAT 与 v2 Profiles，
   不给 URL 提供 key，collection 返回这些记录，且每项与单 Profile GET 的脱敏字段相同。
2. 证明 `401`、`403`、仅 `auth.read` 可读取、空数组、错误传播和 secret redaction。
3. 浏览器测试直接打开 `/auth`，看到已有连接；点击可进入详情，legacy 和多账户记录均可读。
4. 搜索、刷新后新增记录、空列表、API 失败、缺少 read scope 各有独立断言。
5. Candidate 与 active 不同时，只把 active policy 显示在列表；尚无 active 的记录不显示
   Candidate policy 为生效目标。
6. 完成现有前端 build/lint/format、相关浏览器回归、Rust fmt/Clippy 和完整 workspace tests。
7. 部署后在现有 `/auth` 入口直接看到 `indexyz-org`，确认 Active r2、Indexyz 组织与
   5aaee9 个人仓库 selector，点击进入原有详情；不重新创建或扩展 authentication。

文档记录的是目标契约。实现、测试与线上验收证据由
[IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md) 维护。
