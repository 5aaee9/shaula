# Mandatory OpenID Connect Authentication

- Status: Implemented; local protocol/browser acceptance, registered deployment Provider acceptance pending
- Date: 2026-09-06
- Decision: [ADR-0013](../ard/0013-require-openid-connect-for-all-http-access.md)

## 1. Scope

Shaula MUST 自行验证来自启动时配置的单个 OpenID Connect Provider 的身份。所有 UI documents、嵌入静态资源、API、health endpoints 和未来新增 HTTP routes 都默认要求认证；loopback、反向代理、development mode 和 debug binary 均不能绕过认证。

本规范替代 ADR-0011 的 proxy actor trust model 和 spec 0008 / ADR-0012 的 public UI shell。OIDC 用于访问 Shaula 的用户与自动化客户端；用于访问 GitHub 的 GitHub App/PAT Profile 是另一种身份和凭据，二者不得互相替代。

## 2. Required startup configuration

`shaula serve` MUST 使用 clap derive 解析以下配置。显式 CLI flag 优先于对应 env；空字符串视为配置错误，不能回退到 env/default。OIDC 不存在默认 Provider，也不能从 SQLite、HTTP mutation、请求 Host/header/query 或 GitHub Auth Profile 推导。

| CLI flag | Environment variable | Contract |
| --- | --- | --- |
| `--oidc-provider <issuer-url>` | `SHAULA_OIDC_PROVIDER` | Required. Provider 的 exact issuer URL，非 discovery document URL；HTTPS，无 userinfo/query/fragment，允许 issuer path |
| `--oidc-client-id <id>` | `SHAULA_OIDC_CLIENT_ID` | Required. 预先注册的 Shaula OIDC confidential web client |
| 无 secret-value CLI flag | `SHAULA_OIDC_CLIENT_SECRET` | Required. 仅 daemon 读取，不出现在 argv、clap help/error、Debug 或配置转储中 |
| `--oidc-public-url <origin>` | `SHAULA_OIDC_PUBLIC_URL` | Required. 浏览器访问 Shaula 的固定 HTTPS origin，无 userinfo/path prefix/query/fragment |
| `--oidc-api-audience <audience>` | `SHAULA_OIDC_API_AUDIENCE` | Required. Provider 为 Shaula API 签发的 access token audience；与 web client ID 不同 |
| `--oidc-ca-cert <pem-path>` | `SHAULA_OIDC_CA_CERT` | Optional. 私有 Provider 的附加 PEM trust root；不关闭 TLS certificate/hostname validation |

Provider 与上述 client/public-origin 配置只能通过 CLI/env 提供，bootstrap YAML 不提供另一套同名来源。Secret 不得以 `VITE_` 环境变量、前端 bundle 或子进程完整环境传递。`version`、`--help` 和 completion 不启动 server，不要求 OIDC 配置。

Provider registration MUST 支持 Authorization Code flow、PKCE S256、`openid` scope、asymmetric signed ID Tokens 和 `client_secret_basic` token-endpoint authentication。固定 redirect URI 为 `<public-origin>/auth/oidc/callback`，必须预先在 Provider 注册并 exact match；daemon 不进行 dynamic client registration。API 客户端从同一 Provider 获取符合 §5 的 access token；注册 web client 本身不自动获得机器客户端权限。

当前实现的签名 allowlist 为 `RS256`，ID Token 和 API access token 均需使用带 `kid` 的 RSA signing key。Authorization grants 位于 bootstrap `http.authorization`，每项为 `{issuer, subject, scopes}`；空列表默认不授予资源权限。部署示例见 [OIDC deployment](../oidc-deployment.md)。

`serve` 在 bind HTTP listener、启动 Fleet/Profile workers 或产生远程资源副作用之前 MUST 完成：

1. 校验必填值、URL、authorization policy 和 loopback bind；缺少 Provider 或其他必填项时非零退出，错误只标识配置项，不回显 secret。
2. 按 OIDC Discovery 从配置 issuer 获取 metadata；metadata `issuer` 与配置值 exact match，验证 HTTPS authorization/token/JWKS endpoints、所需 flow、签名算法及 client authentication capability。
3. 通过带 TLS certificate validation、timeout、body/JSON limits 和有限重试的 HTTP client 加载可用 JWKS，构造 verifier、login transaction store、session store 和默认拒绝的 router。不得根据未验证 token 的 `iss`、`jku` 或 `x5u` 动态选择 Provider / key endpoint。
4. Discovery/JWKS 不可用、issuer 不匹配、配置或 capability 不支持时 fail closed；不启动 anonymous server，不 fallback 到 proxy headers、共享 backend token 或其他 Provider。Discovery/JWKS/token 请求禁止自动跟随 HTTP redirects，避免凭据或信任边界变化。

OIDC startup validation 不证明用户能成功登录；发布验收仍需通过真实已注册 Provider 的一次完整登录及 API token 流程。配置变更需要重启，旧 login transactions 和 sessions 全部失效。

## 3. HTTP authentication boundary

唯一匿名 allowlist 为 **`GET /auth/oidc/login` 与 `GET /auth/oidc/callback`**，用于建立认证。它们不提供应用 shell、静态文件或管理数据；只返回受限重定向或最小、no-store 的错误响应。其他 methods、相似前缀和未知 `/auth/oidc/*` 路由不继承例外。

| Surface | Authentication and unauthenticated behavior |
| --- | --- |
| UI document GET，如 `/`、`/fleets`、`/templates`、`/auth`、`/changes` | 有效 OIDC-derived browser session；无 session 时 `302` 到 login，响应不含 UI HTML |
| 静态 JS/CSS/font/image、favicon、robots、source map（若发布）、UI HEAD | 有效 browser session；否则 `401`，不提供资源字节或 `304` |
| `/api/**`，包括 `/api/v1/session`、artifact upload/download、revision/status/change reads 和 mutation | 有效 session 或已验证的 API bearer access token；否则 `401` problem response，不能返回 login HTML 或重定向 |
| `/livez`、`/readyz` 和未来 metrics/diagnostic HTTP endpoints | 同样要求 session 或 API bearer token；无凭据 `401`。Probe 必须使用有效机器凭据，无 anonymous health exception |
| `POST /auth/oidc/logout` | 必须有 browser session 与 CSRF 校验，撤销本地 session 并清 cookie；不允许 GET logout |
| 未知路径、unsupported method、SPA fallback | 先认证再返回 `404` / `405`；不能把 fallback、HEAD、OPTIONS 或条件请求变成匿名入口 |

UI redirect 仅适用于明确的 document GET routes；静态资源、API 和未知路径不能根据 `Accept: text/html` 改变认证规则。Bearer token 仅用于 API/health，不作为 UI document 或静态资源访问凭据。Authorization header 一旦出现就必须严格验证；invalid bearer 或 session 与 bearer 同时提交的 API 请求拒绝，不静默 fallback 到另一身份。API/health 的 `401` 包含 Bearer `WWW-Authenticate` challenge，不泄露 token validation 内部细节；不启用 cross-origin credentialed CORS。

认证 middleware MUST 位于 API router、static handler、fallback、conditional caching 和 body-consuming handlers 之前。拒绝未认证 artifact upload 时不能开始 publication、validation 或资源 mutation。全局 request-size、timeout 和 rate limits 可先于认证运行。所有响应（含 auth redirects/errors、UI、assets、API、health）使用 `Cache-Control: private, no-store`；代理/CDN 不能缓存受保护内容，UI 不注册 service worker 或离线 asset cache。

## 4. Browser login and session

Shaula 是 server-side OIDC relying party。使用 Authorization Code + PKCE S256；不使用 implicit flow、password grant 或在 React 中交换/保存 token。Login 生成 CSPRNG `state`、`nonce`、PKCE verifier 和短期 transaction，绑定发起登录的浏览器；callback 必须同时验证该绑定、一次性 state、code exchange 和 ID Token 后才能发 session。Transaction 最长 10 分钟，消费一次即失效；重放、缺失/错误 state、nonce 或 Provider error 不建立 session。

ID Token MUST 校验签名及允许的 asymmetric algorithm、exact `iss`、web client `aud`、适用时的 `azp`、`exp`、`iat`、`nbf`（若存在）和 transaction nonce，允许至多 60 秒 clock skew。拒绝 `none`、不兼容 key/algorithm、未知 issuer/audience 和无有效 `sub`。使用成熟 Rust OIDC/OAuth2/JWT 库处理协议与密码学，不手写 verifier。

登录后 cookie 只包含高熵 opaque session ID；ID/access tokens、client secret、PKCE verifier 与 identity claims 留在 server。Session store 只保留授权所需身份、scope 和有效期；v1 不持久化 refresh token，不申请 `offline_access`，也不自动延长认证期限。Session 绝对期限不晚于 ID Token `exp` 且不超过 1 小时，idle timeout 15 分钟；到期后重新走登录。进程重启清除 sessions，logout 立即撤销当前 session，登录成功轮换 session ID 防止 fixation。

Session cookie 使用 `__Host-shaula-session`、`HttpOnly`、`Secure`、`SameSite=Lax`、`Path=/`，无 `Domain`。Login transaction cookie 同样受保护。部署浏览器入口始终 HTTPS；本地开发也使用 HTTPS 入口和实际测试 Provider，不提供 production 可启用的 insecure/no-auth 开关。

Cookie-authenticated unsafe requests（所有 PUT/POST/PATCH/DELETE，包括 logout）必须同时验证 exact configured Origin 与绑定 session 的 CSRF token；缺失或不匹配返回 `403`。`GET /api/v1/session` 返回 `{name, scopes}`，并通过 `X-CSRF-Token` response header 提供 token；UI 仅保存在内存中，在 mutation header 回传。纯 bearer 请求不依赖 cookie，可免 CSRF，但仍需授权。

Login 的 return target 只允许本 origin 的已知 UI document path 和经校验的应用 query，不接受外部 URL、scheme-relative URL、反斜杠或编码绕过；未知 target 回到 `/fleets`。固定 public origin 不能从请求 `Host` / `Forwarded` / `X-Forwarded-*` 推导。Login/callback/code、cookie、CSRF、token 和 secret 不写日志、URL telemetry 或错误内容；最小错误页不能加载受保护或外部资源。

## 5. API identity and authorization

API 接受同一 Provider 的 OAuth2 **access token**，使用 `Authorization: Bearer`。v1 支持 RFC 9068 的 asymmetric signed JWT access token，校验 `typ=at+jwt`（或 `application/at+jwt`）、signature/algorithm、exact issuer、configured API audience、`exp`、`iat`、`nbf`（若存在）、`sub`、`client_id` 和 `jti`，时间校验至多允许 60 秒 clock skew。Opaque tokens / introspection 不属于 v1；ID Token 不能用作 API bearer credential。Token 不得放入 URL 或请求 body。

认证成功不等于管理权限。Authorization policy 属于 daemon bootstrap concerns，以 exact `(issuer, subject)` grants 映射现有 `fleet.*`、`template.*`、`auth.*` permissions；未知 principal 默认没有资源权限，不提供首次登录自动 admin。Bearer 的有效权限还必须与已验证 access-token `scope` 中的 Shaula scopes 取交集；`openid/profile/email` 不是管理权限。无权限的已认证调用返回 `403`，不重新登录或提升权限。Health 和 session read 只要求有效身份，不授予资源权限。

Actor 的稳定身份是 `(iss, sub)`，使用 versioned、无歧义编码传递到 authorization、audit 和 idempotency principal scope；display name/email 仅供展示，不作为身份键。没有 legacy actor-name 到新 principal 的隐式映射；旧 audit 保留原 provenance，旧 idempotency records 不能被新身份无条件继承。

现有 `X-Shaula-Actor`、`X-Shaula-Scopes`、`X-Shaula-Backend-Auth` 和 Vite development actor injection MUST 从认证路径移除。即使共享 backend token 正确，也不能建立身份、叠加 scopes 或绕过 OIDC。反向代理只能转发浏览器 cookie 或 bearer token，并负责 TLS；传输本身不授予访问权。

## 6. Runtime failures and ownership

Discovery/JWKS refresh 使用 bounded cache、有限 timeout/backoff 和 unknown-key refresh 去重。Provider outage 时只有仍在有效期内的已验证本地 session、或能通过仍有效 cached metadata/key 验证的 token 可继续访问；新登录、未知 key、过期 cache/token/session 必须拒绝，不能无限使用 stale key 或降级为未验证 claims。Auth service 无法建立/验证新身份时 readiness 为 false，但已提交资源的安全 recovery/cleanup 不因身份服务 outage 被丢弃。`/livez` 的业务检查仍只反映 supervision，HTTP 认证先行。

`shaula` 负责 clap/env 配置与启动顺序；`shaula-http` 内聚 discovery、verifier、login/session/CSRF 与 route guard。Core/Registry 只接收已认证 actor 和有效权限，不依赖 OIDC wire DTO 或前端状态。In-memory login/session stores 必须有容量上限、expiry eviction 与 shutdown cleanup；认证失败不得记录 secret，bounded telemetry 不以 issuer URL、subject、email 或 token 为 metric label。

## 7. Acceptance criteria

1. 子进程启动测试覆盖缺少/空/非法 Provider、client 配置缺失、CLI/env precedence、issuer mismatch、不可用 discovery/JWKS；均非零退出且未监听端口/启动资源 effects。`--help`/`version` 无需 Provider。
2. 枚举所有 production routes 与 fallback：未认证 UI document 只 redirect；JS/CSS/font/image/HEAD/conditional GET、API/artifacts/session、health、OPTIONS 和未知路径不泄露数据或创建副作用。有效旧 backend token 与 forged identity/scopes headers 仍不能访问。
3. 完整浏览器登录覆盖 code + S256、state/nonce/browser binding、重放拒绝、固定 redirect origin、open redirect 拒绝、session fixation、expiry、logout 和 restart invalidation。
4. API 验证拒绝错误 issuer/audience/type/algorithm、ID Token、损坏签名、未知 key、过期 token、冲突 credentials；机器 principal 和用户 principal 权限分别测试，认证成功但 scope 不足是 `403`。
5. Cookie mutations 的跨站 Origin、缺失/错误 CSRF 均 `403`；合法 bearer 调用维持原 ETag、idempotency、redaction、audit 与 `202` convergence semantics。
6. Provider/key rotation、unknown-key 并发、cache expiry 和 outage 测试证明没有 authentication fallback；验证 store bounds、无 secret logs、no-store responses，以及 debug/release binary 静态资源同样受保护。
7. Playwright 经登录后验证全部资源页面与写入流程；未登录不能加载应用 bundle。真实 Provider 验收同时覆盖浏览器与 API client，测试 mock 不能成为运行时配置开关。

## References

- [OpenID Connect Core 1.0](https://openid.net/specs/openid-connect-core-1_0.html)
- [OpenID Connect Discovery 1.0](https://openid.net/specs/openid-connect-discovery-1_0.html)
- [OAuth 2.0 Security Best Current Practice, RFC 9700](https://www.rfc-editor.org/rfc/rfc9700.html)
- [JWT Profile for OAuth 2.0 Access Tokens, RFC 9068](https://www.rfc-editor.org/rfc/rfc9068.html)
