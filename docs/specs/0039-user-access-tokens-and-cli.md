# User Access Tokens, Rust API Client and Web-equivalent CLI

- Status: **Draft / proposed**。本文定义目标契约，不表示实现、测试或发布验收已完成。
- Date: 2026-09-25
- Repository baseline: `5aaee9/shaula@118a732405069908d62fce7a9639c1079d9d348e`
- Decision: [ARD-0039](../ard/0039-user-access-tokens-and-api-client.md)
- Normative appendix: [API / Web / CLI 对等矩阵](0039-cli-parity-matrix.md)
- Implementation notes and source index: [代码依据、改动点与实施顺序](../design/0039-implementation-notes.md)

本文中的 MUST、MUST NOT、SHOULD 分别表示必须、禁止和建议。除明确标记为“基线事实”的内容外，配置、API、crate 和命令均为本次提案。

## 1. 目标与边界

### 1.1 产品目标

已获授权的用户能够为自己的 Shaula 身份签发有期限、可撤销、可收窄权限的 **Shaula Access Token**，使用 `Authorization: Bearer` 访问管理 API。浏览器继续使用 OIDC session；自动化和远程 CLI 不需要持有浏览器 cookie、CSRF token、daemon 的 OIDC client secret 或平台凭据。

新增独立可复用的 Rust `shaula-client` crate。`shaula` 二进制增加远程管理命令，且所有命令通过该 client 访问同一管理 API，不直接打开 SQLite、不调用 daemon 的内部 service、不执行 Terraform，也不模拟网页请求流程。

**功能对等**的定义是：在相同 principal、相同有效业务权限下，Web 已有的每一项数据读取和业务 mutation 都有明确的 typed client 方法、CLI 命令及验收测试；权限、条件写入、幂等、异步收敛、保密和错误语义相同。不是只实现 list/get，也不要求复刻网页布局、主题或浏览器会话机制。

### 1.2 纳入范围

包括 Fleet、Template Profile、Template Pool、GitHub/Forgejo authentication profile、Jobs、Generation、invocation/logs、Change 查询、健康状态，以及新增的个人 Access Token 管理。包括模板变量/输入契约、非敏感 bindings 编辑、认证目标策略更新和 quarantine finalize 等容易遗漏的操作。

安全敏感的凭据签发也必须有 CLI 路径：使用 **OIDC API access token** 签发个人 Token，见 §3.4、§9.3。日常业务操作使用 Shaula Access Token 即可。不得将“签发 Token 只能在 Web 完成”伪装成完全对等。

### 1.3 不纳入范围

本次不增加本地密码账户、service-account 身份体系、OAuth authorization server、第三方 delegated OAuth、任意 opaque IdP token introspection、多租户资源 ACL、细粒度 Fleet 级 Token 选择器，或通过 Token 绕过平台生命周期保护的 force 操作。

也不实现通用 Change 列表、GitHub/Forgejo 工作流触发或取消等当前 Web/API 不具备的业务功能。不以本次功能为由重构未接入的 lifecycle worker/HTTP state backend。

## 2. 已核对的代码基线

下列是固定 commit 的代码事实，详见代码索引 S01–S18，不是拟议 API：

| 位置 | 当前行为 | 对本提案的影响 |
| --- | --- | --- |
| `oidc/guard.rs` | Bearer 只进入 API/health；API JWT 由 OIDC verifier 验证；cookie 与 Bearer 混用被拒绝 | 新 Token 增加独立 verifier 分支，不改变外层边界 |
| `oidc/tokens.rs` | actor 是 JSON 编码的 `("oidc-v1", iss, sub)`；JWT scope 与配置 grants 取交集 | 新 Token 仍代表这个 principal，不以 token ID 替换 actor |
| `oidc/config.rs`、`registry.rs` | 11 个独立 scope；配置校验硬编码 scope 数量上限 11 | 添加 Token 权限时必须同步 scope catalog、配置和测试 |
| `router/mod.rs` | v1 路由、scope 检查、ETag 与 `shaula-resource-version`、200/202 回执 | SDK 以真实 handler 输出为准 |
| `web/src/lib/api.ts` | mutation 保留幂等 key，修改要求写版本；Fleet 编辑保留数字原始表示 | CLI 不得使用无条件覆盖或有损 JSON 编辑 |
| `service.rs` | 当前 Fleet 通用幂等 hash/lookup 没有 principal 参数 | 必须增加 principal 隔离，不能认为文档要求已经落实 |
| `router/jobs.rs`、`service_generation_finalize.rs` | finalize 返回嵌套回执、`Succeeded`；HTTP 指向未注册的 `/api/v1/changes/...` | 需独立结果模型与 header 修复，不能套用普通 Change 轮询 |
| `main.rs` | CLI 仅 `serve`、`version`；隐藏 SSH/fence dispatch 在 clap 前运行 | 新命令分支必须保留内部入口行为 |

`docs/specs/0009` 当前排除 opaque API token；ARD-0013 当前描述所有管理 API 身份来源于 OIDC。本规范仅修订其 **Shaula 自签发个人 API credential** 部分，保留 mandatory OIDC startup、浏览器认证、CSRF、HTTPS、loopback 和内部 capability 隔离。

## 3. 身份、权限与认证分流

### 3.1 三种管理认证方式

| 类型 | 身份来源 | 使用面 | CSRF |
| --- | --- | --- | --- |
| `oidc_session` | 已验证的浏览器 OIDC 登录 | UI/assets/API/health | unsafe methods 必须校验 Origin + CSRF |
| `oidc_access_token` | 当前配置 issuer 签发、现有规则验证的 API JWT | API/health | 不使用 cookie，免 CSRF |
| `personal_access_token` | Shaula Token registry 中的个人凭据 | API/health | 不使用 cookie，免 CSRF |

三者均经过同一业务授权。Access Token 不得用于 `/auth/oidc/*`、UI HTML/静态文件、worker/control/state listener 或 Runner Setup Info capability 路由；反之，内部 capability 也不得登录管理 API。

只有原来的 exact `GET /auth/oidc/login` 与 `GET /auth/oidc/callback` 匿名例外保留。不得新增无认证的 token-signup、token-exchange 或 bootstrap-admin 路由。

### 3.2 唯一 principal 和权限交集

Token owner 必须从已验证身份取得：`issuer`、`subject` 以及与现有 actor 相同的 versioned principal 编码。请求体不得包含可覆盖 owner 的字段。display name、email、GitHub login、CLI context 名称均不是 owner 键。

令 `G(p, t)` 为当前运行进程配置中 principal 的 grants，`A` 为签发请求的有效 scopes，`T` 为该 Token 固定下来的 scopes：

```text
签发：T 必须是 A 的子集，且 T 只能包含可委托的已知 scope。
使用：effective_scopes = T ∩ G(owner, now)
```

新增加的 grant 不得让旧 Token 获得 `T` 之外的权限。已删除的 grant 不得继续被 Token 使用。不使用 `*`，不让 `write` 自动包含 `read`，不从 `auth.write` 隐含获得 Token 签发权限。

基线 grants 由 bootstrap 加载，配置变更需要重启。因此“当前权限”指 **当前进程已经生效的 policy**，不是磁盘文件编辑瞬间，也不是 IdP 上的实时用户目录。完全删除 owner 的 grant 记录后，重启生效的 policy 必须使其 Token 认证失败；保留 grant 记录但 scopes 为空时，可保持有效身份，但无业务资源权限。删除 grant 是 policy 禁用，不会自动写 revoked_at；日后重新添加 grant 可能恢复仍未过期、未撤销的 Token。要求永久失效时必须执行撤销。

Token 对同一个 issuer 的不同 `sub` 没有身份合并规则。尤其不得假设不同 OIDC client 的 pairwise subject 相同，或通过 email 将 CLI 的另一 subject 映射到 Web 用户。

### 3.3 Scope catalog

现有 11 个业务权限不变，增加以下 **self-scoped** 权限：

| Scope | 权限 | 可写入个人 Token 的 scopes |
| --- | --- | --- |
| `access-token.read` | 列举、查看自己所有 Token 的元数据 | 是 |
| `access-token.write` | 为当前 principal 签发 Token | **否；还必须使用 primary OIDC 认证** |
| `access-token.revoke` | 撤销自己的其他 Token | 是 |

`GET/DELETE /api/v1/access-tokens/current` 是当前个人 Token 的自查/自撤销例外：只需要该凭据本身有效，不要求额外 scope，也不能选择其他 token ID。

用户不能查看或撤销其他 principal 的 Token。已有 Token 的管理权限不包含读取任何 secret。资源范围仍是现有全局业务 scopes；例如 `fleet.read` 不是“只能读取某个 Fleet”。Web 必须明确说明这一点。

scope catalog 必须统一维护可解析值、展示名称、delegable 属性，移除配置中的数字 `11` 上限假设。旧的 authorization 配置不会被自动追加 Token 权限；部署管理员显式授予新 scope。

### 3.4 签发权限不可由个人 Token 继续委托

允许签发的 primary authentication 是有效 `oidc_session` 或符合既有规则的 `oidc_access_token`；两者还必须拥有 `access-token.write`。普通个人 Token 即使因非法旧数据携带该 scope，也不得签发新 Token，返回 `403 PrimaryAuthenticationRequired`。

这避免泄露的长期 Token 通过自签发无限延长存续。CLI 用 `--credential-kind oidc --token-file <path>` 或 `--credential-kind oidc --token-stdin` 执行相同的签发 API；不接收 ID Token，不复用 daemon 的 confidential client secret，不假设 Provider 支持 Device Authorization Grant。

此限制是凭据签发的 authentication-strength 边界，不是 CLI 缺少 token-create 功能。安全的新旧 Token 切换见 §5.5。

### 3.5 严格分流与失败

Authorization header 只允许一份，Bearer scheme 大小写不敏感；Token 无空白、无控制字符。出现 `__Host-shaula-session` cookie 和 Authorization 的组合继续返回 401，不择优认证或合并 scope。

个人 Token 以 `shaula_pat_v1_` 开头。命中该前缀后只进入个人 verifier；格式错误、未找到、过期、撤销、realm 不匹配均为同一对外 `401 AuthenticationRequired`，不得回退 OIDC verifier。其他 Bearer 继续走现有 JWT 验证流程，不将任意 opaque IdP token 当成本地 Token。

认证错误不回显 Token、hash、owner 是否存在或详细撤销原因。API/health 的 401 继续含 Bearer challenge；数据库不可用是 `503 AuthenticationUnavailable` 并 fail closed，不是假 401 或 anonymous fallback。权限不足为 403。

## 4. Token 格式、持久化和有效期

### 4.1 格式与秘密材料

固定 v1 表示：

```text
shaula_pat_v1_<id>_<secret>

id     = 32 个小写 hex 字符的随机 UUID（去除连字符）
secret = 32 字节 OS CSPRNG 随机数的 base64url-no-padding（43 字符）
```

解析必须检查固定段长度和规范编码；不能按任意下划线 split 后猜测，因为 base64url secret 本身可包含下划线。API metadata 中 `id` 使用同一 32 字符形式。

秘密具有 256 bit 随机输入，不使用时间戳、用户名、Git commit、普通 PRNG 或单个 UUID 作为 secret。服务端使用 SHA-256 验证器：

```text
secret_digest = SHA256(length_prefixed(
  "shaula-personal-access-token-v1", realm, id, decoded_secret_bytes
))
realm = canonical_encoding(configured issuer, API audience, public HTTPS origin)
```

编码必须版本化、无歧义，使用已有成熟 crypto/RNG 库，digest 比较使用 constant-time primitive。不存明文、可解密的 Token 或可恢复 secret 的 seed。这个选择针对高熵随机 secret，不是用户口令的存储方案，也不保护已获得数据库写入或 daemon 内存控制的攻击者。

未知 ID 可进行同长度 dummy digest comparison，避免提供明显的比较差异；不得宣称这消除了数据库查询等所有 timing side channel。格式输入限制、全局认证预算和数据库索引仍须存在。

### 4.2 数据模型

新增独立 `personal_access_tokens` 持久化模型，不复用 GitHub/Forgejo auth profile、browser session 或 worker capability 表。以下是逻辑字段；migration 需适配仓库实际命名规范：

| 字段 | 约束 / 用途 |
| --- | --- |
| `id` | PK，随机公开标识，不重复使用 |
| `owner_issuer`, `owner_subject`, `owner_principal` | 只由 verified identity 生成；编码一致性校验 |
| `realm`, `format_version` | 绑定 issuer/audience/public origin；不从请求 Host 取得 |
| `name` | 1–128 UTF-8 bytes，非空、无控制字符；不是唯一标识 |
| `secret_digest` | 32 bytes；所有 HTTP 投影排除 |
| `scopes` | 去重排序的已知 delegable scope 集合 |
| `created_at_ms`, `expires_at_ms` | UTC unix milliseconds，非空；到期时间晚于创建 |
| `revoked_at_ms`, `revoked_by_principal` | 可空；撤销单向，不可恢复 |
| `last_used_at_ms` | 可空、近似观测，不参与认证决定 |
| `revision` | 从 1 开始，只在安全状态改变时递增 |

owner + created_at + id 必须有分页索引。认证按 id 精确查询。name 不作为索引查找凭据的依据。metadata 的 `state` 从 revoked/expiry/policy 派生，避免另一个需要定时更新才生效的 active 标志。

读取每个请求都必须检查有效期、撤销、realm 和当前 grants；v1 不缓存正向认证结果。`last_used_at_ms` 至多每 Token 每 60 秒合并更新一次，不改变 ETag；这是近似时间，不能拿来证明“从未使用”。更新失败可丢失该观测，但不得改写撤销状态或放宽认证。

创建/撤销记录与对应安全 audit 必须在同一个数据库事务提交。禁止在 SQLite 事务里调用 OIDC、云平台或 Terraform。

### 4.3 默认策略与配置

在 daemon bootstrap 新增 `http.access_tokens`，不向远程 API 开放策略修改：

```yaml
http:
  access_tokens:
    enabled: true
    default_ttl_secs: 7776000       # 90 天
    max_ttl_secs: 31536000          # 365 天；v1 硬上限也是 365 天
    max_active_per_principal: 20
    max_active_total: 10000
```

这些是**建议采纳的产品默认值**，不是仓库当前值。TTL 最小 60 秒，无 `0`、无 never-expire，无 sliding renewal；`expires_at <= now` 立即拒绝，不套用 JWT 的 60 秒 expiration leeway。部署可以缩短默认/最大 TTL。降低 max TTL 只约束新签发，已有到期值不自动修改；紧急场景应撤销或禁用。

`enabled: false` 必须同时禁止个人 Token 签发与 API 认证，但保留已登录 OIDC 用户列举、撤销旧 Token 的能力。缺少新段时按上述默认解析，不给任何用户隐式增加 `access-token.write`。

活跃配额只计算未撤销且未到期 Token。配额检查与 insert 原子化，竞态不能超发；相同请求的幂等恢复在配额检查前命中，不重复占位。签发额外限速默认每 principal 每分钟 5 次、全局每分钟 100 次，返回 429 与 Retry-After；状态必须有界，不能按未经验证的 token ID 创建无限 cache。

Token 元数据与其签发幂等记录至少保留至 `max(expires_at, revoked_at) + 30 天`。未提供重建证据的提早 GC 禁止。更长 audit/备份 retention 沿用部署政策，不因 secret 不入库而把 audit 当普通公开日志。

## 5. Access Token 管理 API

### 5.1 端点和权限

新端点返回 JSON；字段采用 snake_case，不修改旧端点既有 casing。错误统一为 `application/problem+json`，所有响应 `private, no-store`。

| Method / path | 成功结果 | 授权 |
| --- | --- | --- |
| `GET /api/v1/access-tokens` | 200 metadata page | `access-token.read`，仅自己 |
| `POST /api/v1/access-tokens` | 首次 201；幂等恢复 200 | primary OIDC + `access-token.write` |
| `GET /api/v1/access-tokens/current` | 200 当前 metadata | 仅个人 Token；无需另加 scope |
| `DELETE /api/v1/access-tokens/current` | 204 已撤销 | 仅个人 Token；无需另加 scope |
| `GET /api/v1/access-tokens/{id}` | 200 metadata + 写版本 | `access-token.read`，仅自己 |
| `DELETE /api/v1/access-tokens/{id}` | 204 已撤销 | `access-token.revoke`，仅自己；条件写入 |

`current` 必须作为 exact route 注册，不作为 ID 解析。OIDC session/JWT 调用 current 返回 `409 PersonalTokenRequired`。其他用户 ID 和未知 ID 对有 route 权限的调用者均返回 404。所有资源读取、mutation 和 replay 都重新认证、授权、检查 owner，不能仅凭持有幂等 key 绕过。

### 5.2 Metadata 与分页

```json
{
  "id": "914ce165246c43cdae7a33d414fd631f",
  "name": "ci-maintenance",
  "scopes": ["fleet.read", "fleet.write"],
  "effective_scopes": ["fleet.read", "fleet.write"],
  "created_at": "2026-09-25T15:00:00Z",
  "expires_at": "2026-12-24T15:00:00Z",
  "last_used_at": null,
  "revoked_at": null,
  "state": "active",
  "revision": 1
}
```

state 是 `active | expired | revoked | disabled`，判断优先级为 revoked、expired、disabled、active。disabled 包含功能关闭、realm/policy 不再允许该凭据。metadata 中禁止 `secret_digest`、secret、原始 Authorization 或所谓可用于登录的 preview。

列表支持 `state=all|active|expired|revoked|disabled`（默认 all）、`limit`（默认 50、1–100）和 opaque `cursor`。返回 `{items, next_cursor}`；按 `(created_at_ms DESC, id DESC)` keyset 分页。cursor 绑定 principal、筛选条件和格式版本，不接受另一个 owner 或筛选集合的 cursor；不提供总数快照保证。

detail/current 响应返回 `ETag` 和 `shaula-resource-version`，例如 `"914ce165246c43cdae7a33d414fd631f:1"`。`last_used_at` 和 derived effective scopes 变化不改变写版本；该 header 表示 mutation version，响应不允许缓存复用。

### 5.3 签发、一次性返回和丢响应恢复

请求必须包含有效 `Idempotency-Key`（复用既有 1–128 bytes 边界）；不要求 `If-None-Match`，因为 ID 由服务端分配。严格拒绝未知字段：

```json
{
  "name": "ci-maintenance",
  "scopes": ["fleet.read", "fleet.write"],
  "expires_in_seconds": 7776000
}
```

`scopes` 必须显式传入，允许 `[]` 表示仅身份/health/current 能力，不默认授予所有权限。请求包含未知、不可委托或超出签发者有效 scopes 的项时完整拒绝，不能静默裁剪。缺少 expires_in_seconds 使用 policy 默认；服务端时钟决定 created/expires。

首次成功 201：

```json
{
  "token": "shaula_pat_v1_<32-hex-id>_<43-character-secret>",
  "access_token": {
    "id": "914ce165246c43cdae7a33d414fd631f",
    "name": "ci-maintenance",
    "scopes": ["fleet.read", "fleet.write"],
    "effective_scopes": ["fleet.read", "fleet.write"],
    "created_at": "2026-09-25T15:00:00Z",
    "expires_at": "2026-12-24T15:00:00Z",
    "last_used_at": null,
    "revoked_at": null,
    "state": "active",
    "revision": 1
  },
  "secret_available": true
}
```

示例 token 是格式占位符，不是可用凭据。201 带 `Location: /api/v1/access-tokens/{id}`。必须先原子提交 metadata、digest、audit 和**不含 secret 的幂等记录**，之后才能将明文交给响应序列化。

对同 principal、同签发端点、同 key、同 canonical request 的重试：返回 200、原 Token 的当前 metadata、`secret_available:false`，**省略 token 字段**；不重新生成 Token，不恢复明文，不占新配额。TTL 默认值只在第一次处理时解析并记录，重试不能把“当前时刻”重新纳入 hash 或延后 expiry。不同 request 复用 key 返回 409 IdempotencyConflict。

这是对既有“回放完整 response_body”的明确例外。通用 `IdempotencyInsert.response_body`、SQLite WAL、audit、异常栈和 tracing 都不得含 secret。一次性返回指服务端最多一次构造明文成功响应，不承诺客户端一定收到，也不承诺客户端无法复制收到的 secret。

若提交后响应丢失，客户端使用原 key 恢复 metadata 并确认 ID；该 secret 不可恢复。CLI 必须返回 `SecretUnavailable` 和 ID，用户撤销它并重新签发。不能把 HTTP 200 恢复回执打印成“已取得可用 Token”。重试/撤销/重签发均需目前有效权限。

### 5.4 撤销与并发

按 ID 撤销要求 `If-Match`（缺少 428，stale 412）和建议的 Idempotency-Key。首次撤销在数据库事务中设置 revoked_at、递增 revision、写 audit，提交完成后才返回 204；没有异步 Change。经重新授权的相同 key replay 返回 204。已经撤销的 Token，在调用者取得其当前版本后再次 DELETE 是 204 no-op，不再增加 revision。

自撤销 current 不要求 If-Match，因为它是对当前已验证 credential 的单向操作，不提供可编辑字段。成功回执后，该 Token 本身再请求会得到 401；客户端不应期待可以用已经撤销的 Token 回放 204。用另一有效 primary/个人 credential 可查验或管理同 owner 的记录。

撤销提交之后开始的认证检查必须拒绝该 Token。已经通过认证、正在进行的请求可能完成；本文不承诺取消所有 in-flight requests。已经提交的 Fleet/Profile 操作和 cleanup 继续由 daemon 收敛，Token 撤销不是业务变更回滚。

### 5.5 轮换是受控组合操作

v1 不增加可用旧 Token 换新 Token 的 refresh/rotate 端点。Web 与 CLI 的 `tokens rotate` 均执行：primary OIDC 签发新 Token → 安全保存/展示 → 用新 Token 对同 origin 调用 session/current 做验证 → 用户确认或显式 `--revoke-old` 后撤销旧 Token。

新 Token 尚未成功取得、保存或验证时不得撤销旧 Token。撤销旧 Token 失败时输出“两者可能仍有效”，不得隐瞒部分完成。新 Token 权限/寿命由本次显式请求决定，不自动复制 read API 未公开的任何秘密字段。无 `access-token.revoke` 的 primary 身份可完成签发，但不能执行撤旧步骤，CLI 应在开始 rotate 前检查这一要求。

### 5.6 Session 能力发现

扩展现有 `GET /api/v1/session`，保留 `name`、`scopes`，增加非秘密字段：

```json
{
  "name": "display name",
  "scopes": ["fleet.read"],
  "principal": {"issuer": "https://id.example.test", "subject": "subject-123"},
  "authentication": {"kind": "personal_access_token", "token_id": "914ce165246c43cdae7a33d414fd631f"},
  "capabilities": {"personal_access_tokens": true, "token_management_api": 1}
}
```

非个人 Token 省略 token_id。capabilities 是功能发现，不是授权替代；Token 签发开关和相应权限仍由服务端检查。Bearer 响应绝不提供 CSRF token。旧服务端缺少新字段时，client 可使用既有业务 API，但 token-management 命令应明确报告不支持，而不是把 404 当空列表。

### 5.7 新 Token API 的错误细则

缺失或无效签发 Idempotency-Key 返回 400；非法 JSON 返回 400；未知字段、名称/TTL/未知 scope 等 schema 错误返回 422；请求 scope 超出当前有效授权返回 403 ScopeDenied；不可委托 scope 返回 422 NonDelegableScope；个人 Token 尝试签发先由 authentication-strength 检查返回 403 PrimaryAuthenticationRequired。重复 key 不同内容为 409，active 配额/限速为 429。已通过认证的调用者访问未知/其他 owner ID 均为 404，不泄露归属。

主体不存在、Token 无效和数据库不可用的认证错误遵守 §3.5。对所有 replay 先认证、检查当前权限/owner，之后再进行幂等查找；缺失某个旧 response 不得触发未授权的重签发。

## 6. 审计、幂等与保密不变量

### 6.1 请求 provenance

在 HTTP 认证边界生成可信 `AuthenticationContext`，包含 stable principal、effective scopes、credential kind，以及个人 token ID（若有）。actor.name 的既有 principal 编码保持不变。

业务 audit 必须同时保留“谁”与“使用哪类 credential”。将 `credential_kind`、`access_token_id` 作为受控 provenance 传入业务提交事务，不通过可伪造的请求 header，也不修改 actor.name 拼接 Token ID。内部系统操作使用明确的 system provenance；旧 audit 保留原样，缺少 credential 的旧条目标记 unknown，不伪造历史。

创建、撤销、失败的授权操作记录适量安全 audit；每次日志分页不强制新建持久 audit 行，但认证/拒绝 metrics 必须有界。不得以 token ID、subject 或具体 URL 作为高基数 metrics label。

### 6.2 修复 principal-scoped 幂等

所有管理 mutation 的幂等 namespace 必须至少包括：

```text
version + stable principal + HTTP operation + canonical resource identity + idempotency key
request_hash = hash(canonical body, exact write preconditions, relevant semantic parameters)
```

同一用户用 Web、OIDC JWT 或个人 Token 重试同一请求可以取得同一回执；不同用户不能共享回执或因对方的 key 而误命中。Token ID 不作为 namespace，以允许合法轮换后的重试；但每次 replay 之前必须验证**当前 credential 仍有对应权限**。

基线 `service.rs` 的 helper 只传 kind/key/key-value/body/precondition，没有 principal。实施时必须修改调用链、store port 和唯一索引；不能只把 actor 加进 request hash 而保留会造成跨 principal 冲突的旧查询键。

旧记录缺少可证明的 principal 时不得猜测映射或当作新 namespace 的可重放记录。命中潜在 legacy 相同 operation/resource/key 时返回 `409 LegacyIdempotencyConflict` 且不再次执行；操作者检查实际资源后再决定是否使用新 key。不得简单把 legacy record 当 cache miss 导致重复副作用。

### 6.3 Secret 不得扩散

个人 Token、OIDC token、private_key、Forgejo token、bindings secrets：禁止进入 Debug/Display、clap error、普通 JSON/YAML 输出、HTTP debug body/header、telemetry、浏览器 localStorage/IndexedDB/sessionStorage、服务端 idempotency response 或日志。

Token secret 仅出现在首次签发响应和显式用户选择的安全存储/显示路径。用于签发的请求 metadata 不包含 secret。不要将通用 Serde DTO 的派生 Debug 当作脱敏措施；所有 credential wrapper 必须有明确 redacted Debug 和显式 expose 方法。

## 7. Web 行为

新增 `/settings/access-tokens`，通过现有 session 访问。除 React route/menu，还必须同步 `oidc/login.rs` 的 document/return-target allowlist；API/静态文件的原有认证边界不变。

页面显示名称、公开 ID、授予/当前有效 scopes、到期时间、撤销状态、近似 last-used。按当前权限提供查看、签发、撤销；没有 `access-token.read` 但有 write 的用户仍能取得自己本次创建的回执，不能因此读取其他 metadata。

签发表单必须明确展示期限、授权范围和“只返回一次”。禁止默认勾选全部权限。高信任项（template.publish、template.attest、auth.write、retire 类）给出说明，但不能通过 UI 警告替代服务端授权。

新 secret 只保存在页面易失内存中；用户显式 reveal/copy/save 后可离开，一旦关闭不提供重新显示按钮。创建响应不得放入长期 React Query cache、错误报告或 session persistence。请求结果不确定时保留同一个 MutationAttempt key，并使用 §5.3 恢复流程。

Logout 仅结束浏览器 session，不默认撤销用户全部 Token；UI 必须让用户区分这两个动作。Token 撤销结果是同步完成，不渲染为等待 Fleet Change。轮换和失败恢复遵守 §5.5。

## 8. Rust workspace 与 client 契约

### 8.1 依赖方向

```text
shaula binary
  ├── existing serve composition ──> daemon/http/store/template/...
  └── client_cli module ──────────> shaula-client ──> shaula-api-types
                                      └───────────> reqwest / TLS
shaula-http ────────────────────────────────────> shaula-api-types
  └── explicit mapping ─────────────────────────> core registry ports
```

新增 `crates/shaula-client` 和 `crates/shaula-api-types`。CLI presentation 暂放 `crates/shaula/src/client_cli/` 的多个小模块，不要求再建一个 CLI crate。每个 Rust 文件遵守 AGENTS.md 的 400 行拆分约束。

`shaula-api-types` 是 transport contract crate：Serde、基本标识/时间/secret wrappers；不得依赖 Axum、SeaORM、daemon、Terraform 或 CLI。`shaula-client` 不得依赖 `shaula-http` 或直接连接 store。纯 client 可以在外部 Rust 项目单独使用，不要求构建嵌入 UI 或 server assets。现有主二进制的打包依赖不等于运行远程命令需要 Node/Terraform。

server 将 DTO 映射到 core；不能为了 SDK 把 core 的领域模型改成公开 HTTP 契约。已有 `dto.rs` 和零散 handler JSON 必须逐步归并为 **实际输出对应的类型**，不是将目前未使用的 `MutationAcceptedDto` 直接当成 wire 格式。

### 8.2 公开 client 模型

最小公开能力（示意接口名称，不是已编译实现）：

```rust
pub struct Resource<T> {
    pub data: T,
    pub version: Option<ResourceVersion>, // exact opaque strong write version
}

pub enum WritePrecondition {
    CreateOnly,
    Match(ResourceVersion),
}

pub struct MutationOptions {
    pub precondition: WritePrecondition,
    pub idempotency_key: IdempotencyKey,
}

pub enum ChangeKind { Fleet, Profile, TemplatePool }

pub struct ChangeRef {
    pub kind: ChangeKind,
    pub id: String,
}

// 实际 API 必须将 Pending/NoOp/Completed/Uncertain 区分，不用 bool success 代替。
```

`Client::builder()` 显式接收 origin、Credential、timeout、附加 CA 和 User-Agent。Credential 区分 PersonalAccessToken/OidcAccessToken，二者最终都通过 Authorization header 发送；客户端声明类型不构成服务端身份可信依据。

提供矩阵所列的 typed methods 和 typed request/response，而不是只有 `request(path, Value)`。包括 separate normal mutation receipt、FinalizeReceipt、TokenIssueOutcome、NoOp、ProblemDetails、LogsPage、InvocationsPage、Job/Generation queries 和各类型 Change。

### 8.3 Transport 与 error

仅接受固定 HTTPS origin（无 userinfo/query/fragment/path prefix），TLS certificate/hostname validation 始终启用；可增加 private CA，无 `--insecure`。默认不跟随任何 redirect，不将 Authorization 发往 Location、安装链接或 workflow URL。URL path segment 必须编码，禁止调用者通过 key 注入新 origin 或路径穿越。

Client 默认连接超时 10 秒、普通请求总超时 30 秒、artifact upload 300 秒；调用者可以在有界配置内调整。默认控制响应上限 16 MiB、单页 log 2 MiB；超过上限返回明确错误，不静默截断成成功。出错响应最多读取 64 KiB，并由 typed ProblemDetails 提取有限字段；未知 HTML/proxy body 不直接打印。

`/livez` 可以返回无 JSON 的 200/503；`/readyz` 的 503 是可解析的 readiness=false，不当反序列化失败。其他 JSON API 验证 Content-Type。401/403、409/412/428、429、503、网络失败、超时、协议错误有不同 typed error；保留 Retry-After 和经过验证的 request ID，不附带 secret。

只读请求默认最多 3 次传输尝试，指数退避+jitter，遵守合理的 Retry-After 与总 timeout。401/403/404/412/422 不自动重试。mutation 默认一次请求；结果不确定时返回可恢复的 Uncertain，而非更换 key 后自动执行。支持显式用同一个 attempt 重试，见 §8.4。Token 签发不进行透明自动重试。

### 8.4 条件写入、可恢复尝试和兼容

ResourceVersion 优先取 `shaula-resource-version`，其次强 ETag；weak ETag、缺失、多个含混值均不能用于修改。不得重建 revision 字符串、去掉 `W/` 冒充 strong validator，或用 `If-Match: *` 替代已审阅版本。

Create 发送 If-None-Match:*；更新/退休发送原始强写版本。收到 412 时把冲突交给调用者重新审阅，不自动 GET 新版本后重放旧 mutation。正在编辑的文档必须与最初读到的版本绑定。

一个 MutationAttempt 固定 origin、principal、method、path、body bytes、precondition 和 idempotency key；重试保留它们。明确提供 `--idempotency-key` 以支持跨进程重试，输出非秘密 receipt 包含 key；仅对不含 secret 的请求输出 attempt digest，避免公开 secret-bearing body 的离线猜测验证器。含 secret 的请求只在内存保留；不自动写入可逆的 request-body 恢复文件。未知结果不允许被打印成确定失败/确定成功。

所有写请求 serializer 必须保留协议上的 omitted、null、empty object 和具体值差异。特别是 Template Update 的 bindings：省略意味着继承；顶层 null 被拒绝；对象内允许的 null sentinel 遵循现有 spec 0038，不把红acted null 当成要擦除 secret。

Fleet template_inputs 的数值不得经 f64 round-trip。使用经过 fixture 验证的 lossless JSON value/RawValue 实现；仅在确实保证数值/词法契约的范围内提供结构编辑。拒绝无法无损转换的数据，不声称 Rust 默认 serde_json 路径天然保留任意数字 token。

只读 DTO 容忍新增字段，未知枚举保存原始值；未知 lifecycle 状态不得视为成功。写入 DTO 严格验证，避免未知字段被丢弃后形成有损 PUT。更旧 server 不支持的新操作明确 Unsupported，不以空结果代替。

### 8.5 Receipt、等待和日志

普通资源 mutation 返回 accepted receipt 后，默认只表示持久化接收；`--wait` 按 typed ChangeRef 查询 `/fleet-changes/`、`/profile-changes/` 或 `/template-pool-changes/`。

`Converged` 为普通 Change 成功终态；Rejected/Failed/Superseded 为非成功终态。Pending/Accepted/Blocked 等不能被认作成功，Blocked 显示 reason 并在等待预算内继续观察。NoOp 不创建 Change，不对空 changeId 轮询。未知状态保留并受总 timeout 约束。

finalize 的 `Succeeded` 是独立的已提交账本处理结果，不是普通 Profile/Fleet convergence；client 解码嵌套 `MutationAccepted`，可额外 GET Generation 确认 Destroyed，不能轮询不存在的通用 Change API，也不能说外部资源已由该请求删除。

每次 wait/read 重新认证。write-only Token 可以提交对应 mutation，但没有 read scope 时不能 --wait；CLI 在能预检时提前提示，若提交后才失去权限，则保留 accepted receipt 并报告 tracking 失败，不重交 mutation。超时仅代表客户端未观察到目标状态，不能宣称服务器撤销了操作。

分页只用于已有 cursor 的端点；非分页 Fleet/Profile/Pool 列表不得伪造服务器 cursor。流式迭代必须有终止/取消机制并检测重复 cursor。日志保留 invocation/command ordinal/sequence/stream/phase 顺序，展示 capture_status、has_gap、lost_bytes、content_version；无更多页不自动意味着日志完整或执行成功。

`logs --follow` 以有界轮询实现，不假装现有 API 有 SSE/WebSocket。每轮保持过滤条件，按稳定序列去重；401/403 停止，cursor/content version 失效显式报告，禁止静默跳过。Apply/Destroy 是 invocation 的 operation，不是把日志 phase 参数扩展成当前不接受的 `destroy`。

## 9. CLI 用户契约

### 9.1 命令与启动隔离

保留 `shaula serve --config ...` 和 `shaula version`。远程命令使用顶层复数资源名，详见矩阵。`main.rs` 只做 dispatch，业务 HTTP 行为均在 client，表格/用户交互在 client_cli。

help、version、completion 和远程命令不要求 daemon OIDC flags、bindings key、bootstrap config、数据库锁或 Terraform binary。已有 SSH/fence 内部 dispatch 的先后次序、无日志和 stdio 契约不得改变。

### 9.2 配置与凭据输入

提供 `--client-config`、`--context`、`--server`、`--credential-kind pat|oidc`、`--token-file`、`--token-stdin`；没有 `--token <secret>`。CLI 配置独立于 `serve --config`：

```toml
current_context = "prod"

[contexts.prod]
server = "https://shaula.example.test"
credential_kind = "pat"
token_file = "/home/operator/.config/shaula/credentials/prod.token"
```

origin/context 非秘密设置优先级为显式 flag > `SHAULA_SERVER`/`SHAULA_CONTEXT` > context config。凭据来源优先级为本次显式 stdin/file（互斥）> `SHAULA_ACCESS_TOKEN` > context token_file；kind 与来源一起解析。显式来源为空/无效时报错，禁止回落到另一 credential。环境变量为 CI 提供兼容路径，不把它描述为无泄露风险；子进程不得继承 Token 变量。

`shaula auth login --token-stdin` 是导入并验证用户提供的 Token，不是 OAuth 密码登录；调用 session/current 识别 actual principal 后才保存。默认本地 Token 文件在 Unix 为 directory 0700/file 0600，拒绝不安全权限、symlink 和覆盖竞态，原子写入。Windows 需等效 owner-only ACL；无法建立安全权限时 fail closed。首次交付不要求 OS keyring，但不得把明文 Token 写进上述普通 TOML。

`shaula auth logout` 仅移除当前 context 的本地凭据；`--revoke` 才调用远程自撤销，输出清楚区分两种结果。contexts 命令只输出 origin/kind/文件引用，不输出 secret。服务端无法验证的 Token 不覆盖已有有效本地 credential。

### 9.3 命令示例（实施后目标用法）

```bash
# 从 Web 获得一次性 Token 后通过安全输入导入；不把 secret 放进命令历史。
shaula --server https://shaula.example.test auth login --token-stdin
shaula auth whoami
shaula fleets list --output json
shaula fleets get build-linux
shaula fleets status build-linux --watch

# JSON body 是同一 HTTP API 的请求形状；预先审阅的写版本显式绑定到本次操作。
shaula fleets create build-linux --file fleet.json
shaula fleets update build-linux --file fleet.json \
  --if-match '"<incarnation>:7"' --idempotency-key <request-uuid> --wait
shaula fleets edit build-linux

shaula templates sources list
shaula templates variables sha256:<digest>
shaula templates update linux --file template-update.json --if-match '"<incarnation>:3"' --wait
shaula pools list
shaula auth-profiles impact github-main
shaula auth-profiles policy-update github-main --file policy.json --if-match '"<incarnation>:4"'

shaula jobs list --fleet build-linux --limit 50
shaula generations list --association unassigned
shaula invocations list <generation-id>
shaula logs read <invocation-id> --follow
shaula changes wait --kind fleet <change-id> --timeout 300s

# 签发/轮换仍由同一 primary OIDC principal 授权；不是用普通 PAT 自我续命。
shaula --credential-kind oidc --token-file /secure/oidc-api.token tokens create \
  --name ci-read --scope fleet.read --ttl 30d --secret-out /secure/ci-read.token
shaula tokens list
shaula tokens revoke <token-id> --if-match '"<token-id>:1"' --yes
```

示例中的 `<...>` 均为需替换的标识，不是可执行占位认证材料。CLI 应在 --help 中给出完整请求 schema 示例与各命令额外 read 权限。

### 9.4 编辑、危险操作和输出

`edit` 先 GET 数据与版本，再用 `$VISUAL/$EDITOR` 编辑安全临时文件；通过 exec argv 而非拼接 shell 指令启动 editor，清理 SHAULA_ACCESS_TOKEN 等 credential 环境变量，不把所有 daemon/CI secrets 传下去。输入契约、别名选项和 bindings schema 可用 `--interactive` 选择，但 JSON 文件路径必须完整表达 Web 所有业务字段。

`retire`、`finalize`、撤销和影响较大的 auth rotation/policy-update 在 TTY 展示目标与影响，要求确认；非交互模式显式 `--yes`。确认不能取消服务端的 scopes/ETag/依赖检查。finalize 必须提供非空 `--reason`（至多 4096 bytes）和“已在带外确认资源不存在”的明确确认，不提供 `--force`。

支持 `--output table|json`，默认 TTY table、非 TTY json。JSON v1 输出统一 envelope，包含 command、data、receipt/metadata 等稳定字段；保留完整业务字段，不把易变的终端列当机器协议。warnings/progress 写 stderr，stdout 只包含指定数据；`--watch/--follow` 的机器输出使用明确 JSON Lines 事件格式，不串联多个无法解析的 JSON 文档。

所有终端字段包括日志正文默认转义 ANSI/控制序列，防止远端数据操作终端；显式 `logs download --file` 导出的是服务端保留的脱敏文本与缺口元数据，不是原始 Terraform state。分页抓取若超过用户设置的最大项数或发生部分失败，必须标记 partial 而非默默成功。

Token 签发默认仅输出 metadata；调用前必须选择 `--secret-out <new-file>` 或 TTY 下明确 reveal。普通 `--output json` 不包含 token。非 TTY 要输出 secret 至 stdout 必须显式 `--show-secret`，这会改变为单次签发敏感输出；warning 写 stderr。文件目标使用独占创建/安全权限，失败或响应丢失按 §5.3 报告，不丢弃 ID 后假装整个创建未发生。

### 9.5 等待和退出码

| Code | 含义 |
| --- | --- |
| 0 | 读/同步操作成功；或默认模式下 mutation 已 accepted（回执明确 pending），或 wait 达到成功状态 |
| 1 | 未分类的客户端内部错误 |
| 2 | 本地参数/配置错误 |
| 3 | 401：认证无效 |
| 4 | 403：权限不足 / 需要 primary authentication |
| 5 | 409/412/428：冲突或前置条件失败 |
| 6 | 404/410：目标不存在或已终止 |
| 7 | 明确的 400/422 等请求验证失败 |
| 8 | 已得到 accepted receipt，但等待失败/超时/终态非成功；不表示 mutation 未发生 |
| 9 | 传输/服务不可用或结果未知；mutation 输出 uncertain 信息 |
| 10 | Token 已签发但 secret 不可取得/安全交付，需要恢复处理 |
| 130 | 用户取消；保留已知 receipt，不暗示远端已取消 |

code 8/9 的错误 envelope 必须标记是否已有 durable receipt；收到 429 的只读重试预算耗尽归为 code 9，保留 Retry-After。CLI 对 script 的稳定输出/退出码需要 snapshot tests。

### 9.6 机器输出 envelope

非流式 `--output json` 固定包含 schema_version、command、data、metadata、receipt、error 六个顶层字段；不适用时为 null，metadata 至少有 partial=false。命令失败也保持这一形状。以下为普通异步 mutation 的示例：

```json
{
  "schema_version": 1,
  "command": "fleets.update",
  "data": null,
  "metadata": {"partial": false},
  "receipt": {
    "outcome": "accepted",
    "resource_kind": "fleet",
    "resource_key": "build-linux",
    "change": {"kind": "fleet", "id": "example-change-id"},
    "idempotency_key": "example-request-id",
    "version": "\"example-incarnation:8\""
  },
  "error": null
}
```

receipt.outcome 为 accepted、no_op、completed 或 uncertain；uncertain 不得附会不存在的 change ID。error 为 null 或 `{code, message, http_status, retry_after_seconds}`，其中不存在的状态/重试值为 null，message 经过脱敏及控制字符处理。等待失败的 envelope 同时保留 receipt 与 error。列表 data 保留原 endpoint 的 items/next_cursor 或非分页集合结构，CLI 不发明服务端不存在的分页事实。

JSON Lines 事件使用 `{schema_version:1, event, data, metadata}`；event 至少区分 snapshot、log_page、progress、error、end。有限 --watch/--follow 或用户取消的收尾事件包含 partial 与终止原因。默认机器输出不含 credential secret；只有明确 --show-secret 的单次签发输出可在 data 中加入 token，禁止在其他操作继承该选项。

## 10. 兼容、升级与故障边界

### 10.1 HTTP 与构建兼容

不改动旧业务 URL/casing/status，只新增 Token routes/session fields。Forgejo credentials 继续使用当前历史命名 `/github-auth-profiles`；CLI 采用中性 `auth-profiles` 只是命令名称，不创建虚构的后端路径。

修复 finalize 的无效 Content-Location：移除指向未注册 Change 的 header，提供有效的 `Link: </api/v1/generations/{id}>; rel="related"`；保留现有 202 和嵌套回执以避免破坏调用方。该回执中的 Succeeded 表示账本提交完成；不得为了统一 SDK 而把所有 endpoint 无版本地改成同一 JSON envelope。

`shaula-api-types` 的提取必须有 HTTP golden fixtures，覆盖 handler 而不是只测 DTO 自己 round-trip。binary/client 保持 workspace 版本/MSRV 约束；构建 client 的测试不得隐式启动 server 或下载 UI 构建产物。

### 10.2 Migration 与回退

新增 Token 表、audit provenance 和 principal-scoped idempotency 数据格式使用新 migration；现有 browser sessions 不转成 Token，现有 GitHub/Forgejo credentials 不迁移进 Token 表。

迁移前备份完整数据目录，验证 schema compatibility。旧 binary 不认识新增 schema 时必须拒绝或按明确支持路径回滚，不能在新旧 namespace 间静默降级。发布步骤先迁移服务端，再部署 client；未支持 Token API 的旧 server 仍可用现有 OIDC JWT 执行业务命令。

### 10.3 OIDC 不可用、撤权和灾难恢复

OIDC 仍是 `serve` 的 mandatory startup barrier：即使已有个人 Token，Discovery/JWKS 初始化失败也不启动管理服务或 workers。运行中 Provider 暂时不可用时，只要本地 Token registry 和 policy 可验证，个人 Token 可继续调用已运行 daemon；readiness 仍按既有 OIDC readiness 返回 false，不把这个结果抹掉。

IdP 禁用用户、撤销其 refresh token 或退出浏览器，不会神奇地向本地长期 Token 发送撤销通知。离职/失窃的可执行处理是撤销 Token、禁用 Access Token 功能，或删除 principal grant 并重启生效。有限 TTL 不是实时 IdP revocation。

备份恢复可能恢复早先尚未撤销的 Token。灾难恢复规程必须在重新暴露管理 HTTPS 入口之前以 `enabled:false` 启动、完成导入 Token 的撤销，再显式重新开启并重新签发。v1 没有管理员跨用户 Token API：可由各 owner 使用 primary OIDC 撤销；完整灾难恢复则需管理员在服务停止、已备份的维护窗口，通过经审阅的 migration/数据库维护事务批量设置 revoked_at、递增 revision，并记录 system recovery audit。这个主机管理员恢复步骤不属于普通远程 CLI，不能给普通 API 用户 SQL 访问能力。无法证明全量撤销完成时必须保持 enabled:false。不能声称把 revocation 存进同一个被回滚的数据库就能防止复活；v1 不额外引入外部 monotonic revocation service。

## 11. 验收要求

以下全部属于目标测试，不是本次文档编写已执行的测试。

| ID | 必须通过的场景 |
| --- | --- |
| AT-01 | format/CSPRNG 长度、严格 prefix 解析、无 secret 日志/Debug；错误格式不回退 OIDC |
| AT-02 | owner 绑定来自 verified issuer/sub；同 email、不同 issuer/sub 不能访问对方 Token |
| AT-03 | PAT/JWT/session guard matrix：有效、过期、撤销、混用、重复 Authorization、未知路由、assets/internal listener 拒绝 |
| AT-04 | scope attenuation、无 write=>read；logs 两权限；新增 grant 不扩大旧 Token；重启后的撤权生效 |
| AT-05 | PAT 不能签发；primary session/JWT 可按权限签发；配置中的新 scope 正确解析；没有隐式 admin |
| AT-06 | 首次 201 一次性 secret；相同 key 200 无 secret；不同 body 409；并发 retry 仅一行；提交后断网恢复 |
| AT-07 | 事务回滚、配额竞态、到期边界、disabled、数据库失败均 fail closed；last-used 不改变 ETag |
| AT-08 | owner/current 撤销，in-flight 边界，过期 metadata 管理；撤销不取消 accepted Fleet 操作 |
| AT-09 | audit 保留 principal 与 token ID，token raw/hash 均不入日志；幂等 principal v2 隔离与 legacy 拒绝 |
| CL-01 | shaula-client 独立构建；远程命令无 daemon/bootstrap/SQLite/Terraform 启动；隐藏 dispatch 回归 |
| CL-02 | 实际 Axum router 输出的 golden fixtures，mixed casing、非 JSON livez、normal/NoOp/finalize receipt、未知 enum |
| CL-03 | ETag 优先级、proxy weak ETag、缺失版本、412 不重试覆盖、同 attempt 固定 key/body/precondition |
| CL-04 | 所有矩阵业务操作均能通过 PAT+真实 HTTP listener 执行；Web/CLI 相同 scopes 给出相同准入结果 |
| CL-05 | cursor/filters、重复 cursor、日志 gap/lost_bytes/capture_status、follow 断网/撤销/内容版本变化 |
| CL-06 | JSON 数值保真、Template Update omitted/null/empty/keep-sentinel、write-only secrets 和 schema policy |
| CL-07 | 202/NoOp/Blocked/Rejected/Superseded/Failed/未知状态、取消、timeout、提交后撤权与 tracking 失败 |
| CL-08 | CLI stdin/file/env 优先级、无 argv secret、安全文件/权限/symlink、HTTPS/redirect、stdout/stderr、退出码 |
| UI-01 | Token 页认证返回路径、创建/显示一次/撤销、权限裁剪、丢响应恢复、无持久浏览器 secret |
| UI-02 | 每个矩阵 workflow 的 Web/CLI 对等测试；Pool/Forgejo/Finalize/日志不能遗漏 |
| OP-01 | migration、旧幂等记录、安全备份恢复、scope revoke 后重启、OIDC outage 启动与运行期差异 |

每个新管理 route 必须加入 route inventory、client 方法、CLI 映射及测试 ID。CI 对矩阵中未覆盖的业务操作失败，不能用“存在一个通用 HTTP request 命令”抵消缺失。正常成功路径以外，至少包含 read-only principal、不同 owner、只写不读、日志无授权四组身份。

实现验收运行仓库要求的 fmt、clippy、nextest，以及现有 Web 检查与新增 HTTP/CLI/Playwright 测试。真实 OIDC Provider 的 primary-JWT/浏览器签发及 PAT 访问至少进行一轮部署验收；无需为了验证管理协议无条件启动所有云 Runner。

## 12. 完成定义与实施分段

按“契约/类型和幂等安全修复 → Token registry/verifier/Web → client 全 API → CLI 全业务 workflow → 故障与对等验收”推进，详细任务见实施说明。分阶段合入不改变最终 scope：只完成只读 CLI 或遗漏 auth/pool/token lifecycle 的版本不得标记 Web parity 完成。

完成必须同时交付文档修订、配置示例、migrations、Rust client API 文档、CLI help/示例、Token Web 页面、对等矩阵覆盖和测试证据。ARD 的接受状态与 IMPLEMENTATION_STATUS 分开维护；本草案不替代这些证据。
