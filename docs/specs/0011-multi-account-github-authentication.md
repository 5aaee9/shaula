# Multi-account GitHub Authentication Specification

- Status: Accepted — local implementation and remaining integration boundaries are recorded in [the implementation status](../IMPLEMENTATION_STATUS.md); real-GitHub multi-account acceptance remains an external release gate; legacy retirement is governed by spec 0018
- Date: 2026-09-07
- Amendment: 2026-09-09 — GitHub Re-auth、显式 Target policy update 与独立 Authentication 表单页面。
- Decision: [ADR-0015](../ard/0015-route-one-github-app-profile-to-multiple-accounts.md)
- Scope: 一个 GitHub App Auth Profile 覆盖多个组织及个人账户的仓库，按具体 Fleet Target 选择 installation

本规范已被接受；本地实现与现有运行时集成边界见 implementation status，真实 GitHub 路由验收待执行，替代 [spec 0001 §6.3](0001-shaula-runner-scale-set.md#63-github-auth-profile)、[spec 0005 §6](0005-profile-http-control-plane.md#6-github-auth-profile-resource) 中 GitHub App 的单 installation / 固定 exact allowlist 模型，并扩展 [spec 0002 §6–7](0002-fleet-http-control-plane.md#7-auth-revision-handoff) 的 mutation / Auth Handoff。其余 Fleet identity、conditional mutation、credential redaction、Busy-safe removal 与 retirement 契约继续适用；旧版发布、执行兼容与升级入口已由 [spec 0018](0018-github-app-only-authentication.md) 移除；§7 仅保留历史数据与部署边界。

## 1. Outcome and boundaries

Operator 可以创建一个 `shared-github` authentication，保存一份 App credential，并声明：

- 允许 `Indexyz` 的 organization-scope Fleet；
- 允许 `5aaee9` 当前及未来拥有、且该 App installation 获准访问的 repository-scope Fleet；
- 继续添加其他明确选定的组织或账户，无需复制同一个 private key 到多个 Auth Profiles。

例如 `5aaee9/new-project` 创建后，Operator 可直接创建引用 `shared-github` 的 repository Fleet。Shaula 在异步 reconciliation 中验证仓库归属、installation access 与 runner permission；不需要先修改 authentication 的仓库列表。

本次范围不包含自动创建 Fleet、自动安装或扩大 GitHub App 权限、个人账户级 Scale Set、GitHub Enterprise、多个 App/PAT credential 的聚合或 fallback。一个 Fleet 仍只有一个具体 `GitHubTarget`（organization 或 repository）；账户选择器不是新的 Fleet Target。组织 Fleet 的仓库使用范围仍由 GitHub runner group policy 管理。

当前只支持 v2 GitHub App。PAT、固定 installation/exact allowlist 发布与旧版升级入口已按 [spec 0018](0018-github-app-only-authentication.md) 退出支持。

## 2. Domain and identity

术语由 [CONTEXT.md](../CONTEXT.md) 定义。本规范区分以下身份：

| 层次 | 内容 | 演进规则 |
| --- | --- | --- |
| Auth Profile | 稳定 key、`github_app` kind、GitHub App identity | 改 App 或 auth kind 必须新 key |
| Auth Revision | credential、Target policy、验证后冻结的 account/installation bindings | 每次成功 publication 产生不可变版本 |
| Target selector | 对一个 organization、exact repository 或一个账户所拥有仓库的准入规则 | 显式 policy publication 才能增加或改变规则 |
| Account binding | 已证明属于该 App 的 GitHub account ID/type 与 installation ID | 在一个已验证 Revision 中冻结；重装须新 Candidate |
| Resolved Auth Context | exact Auth Revision Ref、具体 Target 的远程身份及其唯一 binding | 为一个 Fleet 持久化；不可由请求临时改选 |

GitHub 的 account/repository numeric ID 是归属判定依据；login/name 是地址与展示信息。`App ID` 在新格式中使用正十进制字符串，`installation_id`、`account_id`、`repository_id` 使用 `1..=2^53-1` 的 JSON integer，禁止浮点或截断。现有 Go/Rust protocol 的 ID 编码不因此改写。

输入沿用严格 GitHub identifier validation，拒绝 URL、路径片段、正则和 `*`。新 selector 在语法校验后按 ASCII case-insensitive 名称做去重/匹配，GitHub 返回的 canonical spelling 用于展示。旧 Fleet 的 Target 字节、fingerprint、ETag 和历史幂等记录不因大小写规范化而重写。

## 3. Target policy

新 GitHub App publication 使用 `schema_version: 2` 和结构化 `target_policy`：

```json
{
  "schema_version": 2,
  "kind": "github_app",
  "app_id": "4863460",
  "private_key": "<write-only PEM supplied by operator>",
  "target_policy": [
    {"kind": "organization", "owner": "Indexyz"},
    {"kind": "account_repositories", "account_kind": "user", "owner": "5aaee9"}
  ]
}
```

该示例只含公开 App/account metadata，不包含可用 credential。增加多个组织就是增加多个 `organization` selector；不要求它们共享 installation ID。GitHub App 必须已经分别安装在每个账户。

| Selector | 可以准入的 Fleet Target | 不隐含的权限 |
| --- | --- | --- |
| `organization { owner }` | 该组织本身的 organization Target | 该组织下的 repository Target |
| `repository { owner, repository }` | 一个具体 repository Target | 同名重建仓库或其他仓库 |
| `account_repositories { account_kind, owner }` | 指定 `user` 或 `organization` 当前拥有且 installation 获准访问的仓库 | 账户作为协作者参与的其他 owner 仓库、organization Target |

Policy 是非空集合，最多 100 个 selector、50 个不同账户；重复 selector、未知字段/variant、错配 account kind 或超限输入返回 `422 SpecInvalid`。集合排序按 canonical kind/account-kind/owner/repository 编码决定，语义相同的顺序变化不产生不同 mutation。编码使用版本化 canonical codec；不支持的旧请求在 replay lookup 前被拒绝，历史 replay 记录不改写。

`account_repositories` 的有效范围始终是 **固定 account ID 当前拥有的仓库 ∩ installation 当前可访问仓库 ∩ 所需 runner 权限**。GitHub 安装选择 `All repositories` 时可覆盖未来仓库；选择 `Only select repositories` 时只动态覆盖 GitHub 选中的集合。UI 必须显示两种覆盖范围，不能把 selected 安装标成账户全部仓库。

动态匹配不通过定期列出全部仓库并写回 policy 实现，不设“启动时快照”。仓库枚举仅用于有界、分页的 UI 预览，结果不是授权真相源。没有任何 selector 可以自动授权该 App 未来安装到的另一个账户。

## 4. Validation and target resolution

### 4.1 Candidate validation

HTTP handler 只做本地 schema/precondition/identity 检查，并在短事务中提交 Candidate、Change、audit、outbox；继续返回 `202`，不在 HTTP mutation transaction 内访问 GitHub。

异步 validator MUST：

1. 验证 private key 能认证为声明的 App；所有 API host 由固定 `github.com` 配置派生。
2. 对 organization/user selector 分别使用 App JWT 调用 `GET /orgs/{org}/installation` 或 `GET /users/{username}/installation`；对 exact repository 使用 `GET /repos/{owner}/{repo}/installation`。
3. 校验响应中的 App ID、非零 installation ID、account ID/type、未 suspension 与所需 permission；同一账户的 selector 必须收敛到同一个 installation。
4. 冻结每个 selector 的 account identity 和 installation binding；exact repository 还须验证并冻结 repository ID/owner ID。复用旧 selector 时，已绑定 numeric identity 不得仅因同名 lookup 结果变化而被覆盖。
5. organization / exact repository selector 执行实际 runner access probe。动态 selector 先验证账户 installation、权限与可访问 repository metadata API；账户没有仓库也可通过账户级验证，不得要求存在某个任意测试仓库。
6. 为全部 live dependent Fleet 的具体 Target 执行 §4.2 的身份/access 检查，但只保存绑定 Candidate ref、dependent-set version、已检查 Fleet identity/fence 的 validation snapshot；不得改写 Fleet desired/observed ref/context 或把 Candidate credential 交给执行副作用的 Adapter。
7. 全部验证成功且 Candidate 仍为 desired 时，以事务 CAS 整体 promotion，并 enqueue dependent Fleet Handoff。失败保留旧 active Revision；不能部分发布 Revision。

这里的 validation 可以签发短期 installation/registration/admin credential，以完成现有 access probe；不得 create/adopt Scale Set、建立 message session、Acquire、签发 JIT 或注册/删除 Runner。不能把 client construction 或本地字段检查当成 access 成功。

Installation/account 不存在、suspended、identity 不匹配或真实 permission denial 为终止性 Candidate rejection。网络、`429`、明确的 rate-limit `403` 与 GitHub `5xx` 留在 Pending 并有界退避重试，显示下一次尝试；不能无界忙等。限流 deadline/budget 沿用 [D3](../README.md#仍需决定或冻结)。

### 4.2 Resolve one concrete Target

Fleet admission 对 active Revision 执行本地 selector 结构匹配并记录 exact Auth Revision Ref。新 repository 即使尚无 resolution cache 也可接受为 Pending；结构匹配不等于 GitHub access 已验证。第一次任何远程资源 mutation 前，异步 reconcile 必须得到有效 Resolved Auth Context。

Resolver MUST 按以下顺序产生且只产生一个 context：

1. 找到所有匹配具体 Target 的 selector。没有匹配则 `TargetNotAllowed`；多个等价匹配只有在 account/App/installation identity 相同时才能合并，不能靠数组先后顺序选 credential。
2. 用 exact Revision 的 App credential 查询对应 Target 的 installation，核对已冻结 account ID/type、App ID 和 installation ID。结果中的新 installation ID 不是可以自动接受的同一 binding。
3. 对 repository 获取受认证的 repository metadata，核对 owner ID、repository ID（已有 pin 时）及 canonical owner/name。Candidate 仅把首次查得的 IDs 写入 validation snapshot；Fleet 的首次 identity pin 只能在第 5 步 CAS 中建立。同名重建不能覆盖旧 pin。
4. 对该 Target 做所需 permission/access probe。Metadata 查找可使用当前 installation 的 metadata-only token；repository runner token 必须进一步限缩到该一个 repository ID，避免 500 repositories token 参数上限和旧 token 对新增仓库的覆盖假设。
5. 对已准入且按 retention 保留的 exact Auth Revision，普通 reconcile/Handoff/cleanup 按其已记录的执行引用，将 context 与 accepted Fleet auth ref、Target、当前 fence 一起 CAS 持久化，再交给 Scale Set Adapter；不要求 cleanup/recovery 使用当前 active head。Candidate validation 只返回 proof，不执行这一步；promotion 仅提交新的 desired ref/context resolution intent，observed context 仍由 Handoff 推进。GitHub 网络检查不持有数据库 transaction；结果过期或 CAS 失败即丢弃，不得产生后续副作用。

Context 至少包含 `auth_ref=(profile_key, revision)`、GitHub host、App ID、account ID/type、installation ID、具体 Target kind、organization ID 或 repository ID/owner ID。可存储不可变 context ID 作为引用，但不能只存 profile key / installation ID。secret/token 不属于 context metadata。

名称变更与 redirect 必须先按 IDs/ownership 分类，禁止向未经允许的 origin 跟随 credential-bearing redirect。本次不修改既有 Fleet Target 不可变的规则：大小写之外的 owner/repository rename、transfer 或同名重建将旧 Fleet 标记 `TargetIdentityChanged`，不静默重写其 Target/Scale Set fingerprint。新地址需显式创建符合现有 replacement/decommission 契约的 Fleet；旧资源证据保留。

## 5. Publication, handoff and isolation

### 5.1 Profile changes

同 key v2 PUT 可以轮换同 App 的 private key、显式增加/替换 Target policy，或重新验证已重装的 installation；这些操作都产生新 Auth Revision，不能修改旧 Revision。每次完整 PUT 都重新提交 private key；读取面不返回 secret，也不接受 redacted placeholder 或隐式“复制 latest credential”。

独立的 Target policy update 允许 Operator 明确指定 current Active 的 `base_revision`，
只提交新策略并复用该精确版本的凭据。这是单独的 publication 操作，不改变完整 PUT 的
必填 private key 契约。凭据复用、Active base 检查与新 Candidate 写入必须处于同一事务，
不能先在 handler 读取“最新私钥”再交给普通 PUT。Active 在验证期间可能改变而 desired
ETag 不变，因此仅检查 If-Match 不足以固定凭据来源。

删除/缩小 selector 只有在新 policy 仍覆盖所有 live dependent Fleet Target 时才能激活；否则 Candidate `Rejected(TargetPolicyInUse)`。live 包括 active、Blocked、Decommissioning Fleet，以及 session、in-flight effect、worker cleanup 和 recovery references。历史 terminal Change 本身不阻塞删除。

依赖检查必须在 promotion 事务中重新校验 dependent-set version / 等价 CAS；与新 Fleet admission 并发时只能有一方先以有效 policy 提交。验证期间新增的未验证 Target 使 activation 回到 validation，不能带着旧检查结果 promotion。Profile retirement 仍阻止新 admission/publication。

App identity 与 kind 仍不可改；不支持的历史 Profile 必须用显式发布的新 v2 Profile 替换。同一个账号/App 的 installation ID 因重装变化可通过显式新 Candidate 重新绑定；默认不在后台自动接受新 installation。新增账户必须显式提交 selector，即使 App JWT 已能列出其 installation。

### 5.2 Exact context handoff

`AuthRevisionRef` 保留原来的完整 tuple。desired/observed Resolved Auth Context 补充该 tuple，不能替代它。每个 GitHub effect、message session 与必要的 cleanup/recovery reference 绑定所用的 exact context；跨 context 的 token/session cache 不可复用。

同 Profile credential/policy/binding promotion 及已准入的 cross-Profile replacement 复用现有 durable Auth Handoff：

1. 停止新 acquisition，处理跨 fence 的在途 acquisition/effect 并持久化结果。
2. 用 desired ref 解析具体 Target 的新 context；验证其 Target numeric identity 与已保存的 Fleet identity 相同。
3. 已绑定 Fleet 对保存的 Scale Set ID/immutable identity 做只读 ownership proof；未绑定 Fleet 只做 access classification。
4. 原子推进 `observed_auth_ref` 和 `observed_auth_context`，CAS 同时比较 desired ref/context 与 fence；仅 auth ref 相同不足以证明 context 已切换。
5. ordinary reconcile 才能 create/adopt、绑定 ID 或为 observed context 建立新 session；acquisition 只有在该 session ready 后恢复。Decommission 仍只允许 cleanup-only handoff。

quiesce 必须与真实 listener 使用同一 per-Fleet effect gate：停止新 acquisition、等待已开始的 ACK/Acquire 完成或持久化 uncertainty 后再推进 observed context；普通 reconcile 在新 session 安装前关闭持久化的旧 session。单纯取消 poll future 或更新 observed ref 不构成 quiesce。session 安装、message ingest、ACK 与 Acquire-start 同时校验 exact context、epoch 和 Fleet head/fence；迟到的旧 effect result 只更新原 intent 的历史结果。旧 uncertainty 的恢复与引用释放遵循 spec 0001 §8 的新 authoritative snapshot 边界。

同 key promotion 不修改 Fleet Spec/Revision/ETag，不替换 Runner Resource。cross-key replacement 仍要求 spec 0002 的零 Occupancy / effect barrier。旧 credential/context 在所有真实执行、session、cleanup 与 recovery 引用解除之前不得 GC；`Blocked` 不释放引用。

### 5.3 Runtime failures and freshness

运行中一个 installation 的 suspension、权限失效或路由故障，只阻塞使用该 installation/Target 的 Fleet；不让同 Profile 下独立健康的组织停止工作。App credential 本身失效可能影响该 App 的全部 bindings。新 Candidate 的整体发布门禁与已 active Revision 的逐 binding 运行健康度分开显示。

所有 route proof 有明确 `checked_at`/`valid_until`；正向授权证据最长复用 60 秒，负向 cache 最长 15 秒，不持久化为无限期许可。运行中的 acquisition/JIT/管理 effect gate 定期刷新，过期而无法刷新即停止新的相关 effect。新 Target、daemon restart、auth/context handoff 和已观测的 access failure 强制重新验证。已经在途的请求仍需 durable outcome classification。

刷新 proof 必须重新检查当前 installation identity、suspension、权限，以及具体 repository 的 owner/identity/access；仅签发一个新 token 不得延长 `valid_until`。每个 proof 的生命周期独立于 Auth Revision 的不可变 identity bindings。

这些时限是 Shaula 发现权限变化后的阻断边界，不承诺 GitHub 撤权瞬间中止已经发出的请求、Actions session 或 Busy job。恢复不得用旧 proof、旧 Revision、其他 installation、PAT 或另一个 Profile 作为 fallback；外部副作用前后的竞态按既有 ownership/removal 契约保守处理。

Token cache key 至少区分 GitHub host、exact Auth Revision Ref、account ID、installation ID、具体 Target ID 与 repository/permission scope；installation token 还需按 GitHub `expires_at` 提前刷新。相同 key 的 refresh 合并并发请求，限流按 App/installation 的实际预算退避，禁止在全局锁中等待 GitHub。

`401`、普通 `403`、access-filtered `404` 或 stale route 不代表 Scale Set/Runner 已不存在，不能触发 Create、AlreadyAbsent 或 Destroy。权限收缩阻止无法重新证明安全的 cleanup，保留 Occupancy 和恢复证据。

本次不要求 webhook 服务。若已有事件源提供 installation/permission/repository-selection 变化，必须使该 installation 的相关 proof 全部失效；`all -> selected` 即使 `repositories_removed` 为空也不能只按列表失效。周期验证与按需解析必须独立成立。

## 6. HTTP and UI contract

沿用 `/api/v1/github-auth-profiles/{key}` 的 conditional PUT/GET/DELETE、OIDC scopes、CSRF、ETag、idempotency、Profile Change 与 retirement。v2 仅扩展 GitHub App resource，不增加另一套账户凭据资源或 OAuth 登录方式。

`kind: github_app` 与 `schema_version: 2` 必须显式提交。旧 `installation_id`、`target_allowlist`、PAT 字段，以及省略、旧版或未知 schema version 的请求均在持久化和 replay 前拒绝；不猜测或转换版本。重复 JSON key、未知字段与数值溢出仍按严格 schema 拒绝。完整退出支持契约见 [spec 0018](0018-github-app-only-authentication.md)。

GET 对 v2 返回 `schema_version`、kind、App ID、`credential_present`、active/desired Revision、结构化 `target_policy` 和非 secret `bindings`。每个 binding 包含 account ID/type/canonical login、installation ID、`repository_selection`、验证时间及有界 health/reason。desired Candidate 与 active bindings 必须标明归属 Revision，不可混合展示为一个已生效配置。禁止拼造单个 `identity=app/.../installation/...` 代表整个 Profile。

Fleet status 在现有 desired/observed Auth Revision Ref 旁显示 desired/observed context 的非 secret route metadata 和 resolution/handoff reason。`Active` 表示 Revision 已通过发布验证；它不证明未来某个仓库可用，也不等于所有 Fleet/session ready。

UI MUST：

- 用可增加/删除的 typed selector 行区分“组织 runners”“单个仓库”“个人账户的仓库”“组织拥有的仓库”，不复用一个含义不明的裸文本框。
- App ID/private key 输入一次；默认按账户发现 installation，无需 Operator 猜测哪个单一 Installation ID 能覆盖所有目标。
- 明示 account repositories 包含未来仓库、受 GitHub 安装的 all/selected 范围限制；组织 runners 的仓库使用范围由 runner group 决定。
- 提交前展示 policy 增减、目标账户及 live Fleet 影响；policy publication 与仅轮换 credential 分别命名，保持相同 CAS/validation 流程。
- 显示逐账户的解析/权限故障及 Candidate 总体状态。个人 installation 失败时，仍能查看并使用健康的组织 binding。
- Fleet 页面明确显示具体仓库的异步 resolution/access 结果；authentication 的 Active 状态不能代替该结果，不永久展开 policy。
- 历史不支持的 Profile 显示 `Unsupported` 和非 secret 标识，不解析旧 credential metadata，不展示升级/旧凭据轮换入口，不能选作新的 Fleet 引用。纯读取不得转换数据或扩大授权。

### 6.1 GitHub Re-auth

连接详情提供 **Re-auth**，按用户点击才调用
`GET /api/v1/github-auth-profiles/{key}/installation-link`。要求 `auth.read` 与
`auth.write`；响应 private/no-store 的 `{url, appId, revision, incarnation}`，无凭据。
仅使用仍可管理 Profile 的 current Active v2 GitHub App；没有 Active、历史不支持格式、
Retiring/Retired 均不可用，失败 Candidate 不能替代 Active。

独立 GitHub access adapter 读取该精确版本的 App ID 与 protected credential，以 App JWT
调用固定 `github.com` 的 `GET /app`，核对 numeric App ID。只接受安全的 slug 路径片段，
构造 `https://github.com/apps/{slug}/installations/new`；不信任远端 `html_url`，不跟随
credential-bearing redirect。返回前再次确认 Profile incarnation、Active ref 与可管理状态；
浏览器同样核对响应身份与当前连接，并只跳转到上述固定安装路径。

不存在返回 404；无可用 Active 或读取期间身份改变返回 409；GitHub 身份/响应不合法返回
502；缺少可用凭据、网络、限流或远端临时故障返回脱敏的 503，可显式重试。失败不会修改
Profile 健康状态。读取不创建 Revision、installation token、policy、binding 或 Change。
此操作通过独立 adapter 执行，不把 GitHub 请求放入 Profile publication admission。

GitHub 页面负责账户选择、组织安装权限和审批。安装后由 Operator 回到 Shaula，使用
**Edit target policy** 将组织加入明确的 selector 集合；不依赖安装回调、不接受 URL 中
`installation_id` 作为授权证明，也不承诺 GitHub 自动返回。Re-auth 不创建 OAuth 登录流程，
不自动扩大 policy，不轮换 private key。

### 6.2 Target policy publication

`POST /api/v1/github-auth-profiles/{key}/policy-updates` 接受且只接受：

```json
{
  "base_revision": 2,
  "target_policy": [{"kind": "organization", "owner": "example-org"}]
}
```

`base_revision` 为正整数、表示用户复核的精确 Active Revision。沿用结构化 policy grammar、
重复/未知字段拒绝、OIDC `auth.read` + `auth.write`、CSRF、If-Match 和 Idempotency-Key；
Idempotency-Key 沿用既有 mutation 的可选 HTTP 契约，Web UI 每次 publication 必须发送并在同一请求重试时复用。
不接受 App ID、private key、installation ID 或可变的“latest”来源。

- 接受后返回现有 202 / Profile Change 及新 Candidate Revision，复用同一个 App ID 和
  指定 base 的完整凭据。后台仍对全部 selector、账号身份、权限、live dependents 做既有
  v2 验证；通过后原子 promotion/Handoff。即使策略相同，显式 publication 也可重新验证
  installation；不能把页面打开、GitHub 返回或后台刷新视为 publication。
- 事务内先按既有边界核对持久幂等记录。相同已接受请求在 activation、head 变化或旧凭据
  回收后仍可 replay；请求身份包含独立操作种类、base_revision、规范化 policy 和 If-Match，
  不依赖执行时最新凭据。相同幂等键不同请求拒绝，不与完整 PUT 互相 replay。
- 新请求须同时满足 head incarnation/desired If-Match、current Active == base_revision、
  base 格式及凭据可用、Profile 未退休。检查与 base 凭据复制在同一个 SQLite writer
  transaction 内完成。Active 变化、删除重建、并发 publication 或 credential cleanup
  不能使请求悄悄使用另一个版本。失败保留原 Profile、凭据与客户端草稿。
- 沿用原有 400/403/404/409/412/422/428 错误分类；base 不再是 current Active 返回明确冲突，
  客户端要求重新打开页面复核，不自动换 base、刷新 ETag 或重发。缺少存储凭据不能生成
  空凭据 Candidate。policy shrink 仍由既有 live-Fleet coverage gate 拒绝。

### 6.3 Dedicated Authentication pages

Authentication 列表和所选连接详情保留在 `/auth?key={key}`。复杂表单使用独立页面：

| 操作 | 页面 | 内容 |
| --- | --- | --- |
| Create profile | `/auth/new` | 新 Profile key、App ID、private key 与初始 target policy |
| Edit target policy | `/auth/{key}/targets/edit` | Active policy、精确 base、typed selectors 与 live-Fleet 影响；不显示或要求 private key |
| Rotate credential | `/auth/{key}/rotate` | 同一个 App 的新 private key，保留所复核的 Active target policy；不混入策略编辑 |

三个页面都有明确标题、返回/取消和固定可见的提交操作，支持 390px 窄屏。详情页提供
Re-auth、Edit target policy、Rotate credential，按 scopes、受支持状态和 Active 可用性
显示可用入口；历史不支持格式没有旧凭据编辑/升级入口。退休确认可以保留现有短对话框。

直接访问、刷新和 OIDC 登录后的 return target 都支持这些精确合法路径。编辑/轮换页面每次
进入读取并固定新的 resource/Active/ETag snapshot，不继承上次页面或后台 refetch 的旧 base。
显示 Candidate 状态不能让待验证策略覆盖 Active 编辑起点。读取失败保留明确错误与重试。
编辑/轮换页面缺少 `auth.read` 或 `auth.write` 时不能借深链接读取材料或提交；创建页面沿用
现有完整 PUT 的 `auth.write` 权限，不读取已有连接材料。

编辑目标策略提交前始终展示增减与当前 live-Fleet 影响；影响读取失败阻止 publication。
轮换凭据保留当前明确策略并重新走完整 PUT。422/409/412、网络失败或幂等重试不丢草稿；
后台刷新不得替换输入、base 或 ETag。跳出页面/取消/成功提交结束该草稿；迟到响应不能导航
覆盖后来打开的表单。秘密与草稿只留在页面内存，terminal session failure 清除它们。
成功后返回 `/auth?key={key}` 并显示对应 Change；取消返回列表并保留原连接选中状态。

新增有限 reason codes：`TargetNotAllowed`、`InstallationNotFound`、`InstallationSuspended`、`InstallationChanged`、`TargetIdentityChanged`、`TargetPolicyInUse`、`AmbiguousInstallation`。已有 `Unauthenticated`、`PermissionDenied`、`TargetHiddenOrNotFound`、`RateLimited` 保留。异步失败记录在 Change/status；不能用新 reason 把网络故障伪装为同步 validation 成功。

## 7. Storage, historical records and rollout

版本化 Auth Revision policy/bindings 与 Fleet/effect/session context refs 继续沿用 SQLite 原子提交和重启恢复。Profile revision、validation snapshot、promotion、handoff context/fence、audit/outbox 仍绑定 exact Revision；原始 PEM 与完整 protected backups 仍处于现有 credential-grade 边界。

本节原先的 legacy runtime、旧请求 replay、同 key App 升级和 client-ID 转换承诺由 [spec 0018](0018-github-app-only-authentication.md) / [ADR-0022](../ard/0022-retire-legacy-github-authentication.md) 替代：

1. 保留旧 Profile/Revision 行、credential bytes、审计、幂等记录、revision numbers 与执行证据；不删除列，也不将旧 installation、allowlist 或身份转换为 policy/binding。
2. 旧版 active/desired 行可识别为不支持。纯读取不解析其 credential metadata、不调用 GitHub、不自动激活；发布、准入和执行不使用旧格式。
3. 停机部署前检查 active/desired profiles，以及 live/retained Fleet、session、operation 引用。仍依赖旧执行授权的工作须先在上一版本显式恢复；新版本拒绝旧授权并保留 ownership/cleanup 证据。惰性的旧历史本身不阻止启动。
4. 不支持的 Profile 用新的明确 v2 Profile 替换；不再提供同 key 自动或显式 legacy upgrade。pending v2 Candidate 不使旧 active Revision 可用。
5. 本次退出支持不改写或删除旧数据，代码回退不需要因本次改动恢复数据库；外部资源与 worker 一致性仍须按现有运行规程核对。

Indexyz 与 5aaee9 的多账户用例继续按本规范的 v2 policy 执行。文档不以旧版迁移示例代替部署前的真实引用检查。

## 8. Ownership and implementation sequence

| Owner | 新职责 / 保留边界 |
| --- | --- |
| `shaula-core` | Target selector、account identity、binding/context/ref 类型与纯匹配/规范化；不引入第三种 Fleet Target |
| `shaula-store` / migration | 版本化 policy/bindings、dependent-set CAS、context refs、历史行保留、拒绝旧请求与授权、retention |
| `shaula-daemon` / Profile Registry | HTTP admission、完整 publication、policy/live-reference gates；不在 handler 中调用 GitHub |
| `shaula-scaleset` | App identity proof、target-aware installation resolution、token scope/cache、权限失败分类 |
| `shaula` wiring / auth worker / Fleet supervisor | Candidate 验证与原子 promotion、context Handoff、健康隔离、恢复/effect fence |
| `shaula-http` / `web` | 版本化 DTO/status、typed selector 表单、逐账户结果与 policy 变更预览 |

实施顺序：先冻结 schema/identity 与历史记录拒绝边界；再实现 resolver 与权限分类；接入 publication/admission/Handoff/context persistence；最后更新 UI 并执行不支持格式拒绝与 multi-account 端到端验收。不得先上线只接受 wildcard 的 UI 而让 runtime 继续复用单 installation credential。Rust 文件拆分、fmt/clippy/nextest 要求继续适用。

## 9. Acceptance criteria

| 场景 | 必须观察到的结果 |
| --- | --- |
| 同一 App 的 `Indexyz` + `5aaee9` | 一个 Profile、一次 credential 输入；org/repo Fleet 使用各自正确 installation |
| 两个或更多组织 | 每个组织解析独立 binding；不复用其他组织的 token/admin session |
| 新个人仓库 | 在 Profile activation 后创建仓库；无 policy/revision 修改即可创建并验证新 repository Fleet |
| 空个人账户 | 动态 selector 可 Active；以后首个仓库正常按需解析 |
| selected installation | 选中仓库成功，未授权仓库明确失败；新增 GitHub selection 后重新解析成功；UI 不显示全账户覆盖 |
| 私有仓库、fork、协作者仓库 | 只接受指定 owner ID 拥有且 installation 授权的仓库；与账号协作关系无关 |
| organization selector 与 repo selector | organization 本身不匹配 repo；两个等价 repo match 只合并相同 route，冲突不得 first-match |
| rename、transfer、同名重建/账号名复用 | 旧 Fleet/policy numeric identity 不被替换；即使另一账户也在 policy 中也不静默接管 |
| uninstall/reinstall | 旧 binding 停止新 effect；显式新 Candidate + Handoff 验证后才使用新 installation |
| suspension / permissions / all-to-selected | 在 proof freshness 界限内阻止相关新 effect；empty removed list 也使整 installation cache 失效 |
| 一个 installation 故障 | 受影响 Target Blocked/Degraded，健康组织继续；App credential 失效准确显示更大影响范围 |
| policy expansion/shrink 与 admission 竞态 | expansion staged activation；丢失 live Target 的 shrink 拒绝；CAS 不发布未经验证的新依赖 |
| rotation/Handoff/restart | refs 和 context 同时精确匹配；旧 session/effect 被 fence，Busy Runner 与 recovery 引用保留 |
| rate limit / timeout / 401 / 403 / 404 | 分类准确、退避有界、不过期兜底、不误 Create/AlreadyAbsent/Destroy |
| cache scope 与大量仓库 | refresh singleflight；无跨账户/Revision/Target 污染；不把全账户展开进 500-item token 请求 |
| 旧格式、旧 client 与历史引用 | 旧发布/replay/授权被拒绝；历史记录和执行证据保留，不转换权限或身份；部署前检查引用，见 spec 0018 |
| UI / sensitive data | 一份凭据、多类型 selector、per-binding 状态与 policy 变更预览；任何读取/错误/日志/审计均无 PEM/token |
| Re-auth | 点击才读取真实 App 身份并跳转安装页面；401/403/404/409/502/503、恶意 slug/URL、active/candidate 与删除重建竞态均不误跳转或扩大 policy |
| policy-only publication | 不上传 PEM，精确 base 凭据在事务内复用；base activation 竞态、If-Match、幂等 replay/冲突与旧凭据回收均不改变来源或重复创建 Revision |
| Authentication 独立页面 | 创建、策略编辑、轮换的直达/刷新/OIDC/返回/取消/迟到响应及 390px 布局通过；策略编辑不出现私钥框，轮换不修改选择器 |

验证必须包含纯匹配/codec tests、真实 SQLite transaction/crash tests、scripted GitHub HTTP（含分页/redirect/限流）、API/UI integration，以及真实 `github.com` 的 App × 多 organization/repository 路由验收。真实服务需验证 Scale Set、session、JIT、inventory、安全 removal，而不只验证 Profile Active。并沿用 exact-pinned Go oracle 的协议差异测试；发布证据记录在 implementation status，不写成未运行的通过声明。

## 10. External references

- [GitHub App REST API — app identity and installation lookup](https://docs.github.com/en/rest/apps/apps)
- [Installation access tokens — repository/permission narrowing and expiry](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app)
- [Installation repository listing](https://docs.github.com/en/rest/apps/installations#list-repositories-accessible-to-the-app-installation)
- [Durable IDs and GitHub App best practices](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/best-practices-for-creating-a-github-app#use-the-durable-unique-id-to-store-the-user)
- [Runner permissions](https://docs.github.com/en/rest/actions/self-hosted-runners)
- [Installation repository-selection events](https://docs.github.com/en/webhooks/webhook-events-and-payloads#installation_repositories)

上述来源说明 GitHub 能力；60/15 秒 freshness、Candidate 整体 promotion、显式 reinstall publication 和 live Target shrink gate 是 Shaula 在本提案中选择的契约。
