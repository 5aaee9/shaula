---
status: proposed
date: 2026-09-25
amends: [0010, 0012, 0013]
---

# ARD-0039: User-bound Access Tokens and an API-only Rust Client / CLI

## Context

Shaula 已有管理 HTTP API、API-driven Web UI、OIDC session 和同一 Provider 的 API JWT access token。当前 Rust binary 的公开命令只有 `serve`、`version`；现有 workspace 没有可独立使用的管理 client。用户需要为自己签发可用于脚本的 Access Token，并以 CLI 完成 Web 能做的读取和业务操作。

本决定依据 `5aaee9/shaula@118a732405069908d62fce7a9639c1079d9d348e` 的实际代码，而不是只依据目标架构。完整 [代码索引和实现边界](../design/0039-implementation-notes.md) 列出文件与固定 commit 链接。

当前可利用的基础包括：稳定 `(issuer, subject)` actor、独立的业务 scopes、条件写入、异步 Change、持久化 audit、以数据库为基础的控制平面以及嵌入式 Web。然而几个边界必须在方案中正面解决：

- spec 0009 的 opaque-token 排除条款与个人 Access Token 冲突，需要有限度修订，而不是隐藏在 JWT verifier 中。
- “代表谁”必须与“凭据是哪一个”分离。直接用 token ID 替换 actor 会破坏 Web/CLI 审计和重试的一致性。
- 当前 Fleet 幂等 helper 没有 principal 参数，不满足增加多种用户 credential 后必须明确的 principal 分区要求。
- handler 的实际 JSON 形状并不等于 `dto.rs` 中所有声明；finalize 更有独立 `Succeeded` 回执和无效 Change URL。client 不能依靠理想化 DTO。
- 当前业务 UI 不只是 CRUD：还有 bindings 的 omitted/null 语义、日志保留缺口、策略更新的影响预览、未知任务关联和异步退休。

## Decision

采用 **绑定既有 OIDC principal 的 Shaula opaque Personal Access Token**，服务端持久化不可逆验证摘要，每个请求检查撤销和当前 grants；新增共享 transport types 与独立 async Rust client；CLI 仅通过这个 client 访问同一个管理 HTTP API。

协议、字段、默认值、CLI 矩阵和验收要求的唯一维护位置为 [spec 0039](../specs/0039-user-access-tokens-and-cli.md) 及其 [对等矩阵](../specs/0039-cli-parity-matrix.md)。本 ARD 解释选择的理由、代价和否决的方案，不维护第二套协议。

### D1. Token 是用户的一种 API credential，不是另一套用户体系

Token owner 使用当前的 exact issuer/sub；继续使用既有 versioned actor 编码。显示名和 email 不参与所有权匹配。新 Token 不创建本地账户，不读取 GitHub/Forgejo profile 来决定 Shaula 权限。

服务端持久化 token ID、owner、固定 scopes、期限、撤销状态和摘要。有效权限为签发 scopes 与当前生效配置 grants 的交集；不能超过签发者当时的有效授权，也不能因后续用户获得新权限而自动扩大旧 Token。

**理由：**这保留现有身份的唯一性，支持同一用户在 Web、CLI 和 Rust 程序间使用不同 credential，而不用迁移 Fleet/Profile 的主体模型。配置仍是授权来源，没有引入一个权限更高的 Token registry。

**代价：**当前 grants 需重启生效；本地 Token 不自动接收 IdP 用户禁用事件。必须清楚披露撤权边界，并采用有限 TTL、明确的撤销流程和灾难恢复规程。不同 OIDC client 的 pairwise subject 不保证相同；不按 email 自动合并。

### D2. 用高熵 opaque Token，不用 Shaula 自签 JWT

选择带版本前缀、公开随机 ID 和 256-bit 随机 secret 的 opaque Token；服务端存储 domain-separated SHA-256 verifier，并 constant-time 比较摘要。绑定部署 realm，公开 origin/issuer/audience 改变时不能把旧 Token 默默搬到新 realm。

**理由：**这不是分布式无状态授权场景。单控制平面本来已有 SQLite，用户需要可撤销的长期凭据。JWT 为立即撤销和当前权限检查仍要读状态，签名 key/JWKS/claim 更新只增加复杂度。高熵随机 secret 也不需要采用面向低熵人类口令的昂贵密码 KDF。

**代价：**每个已认证 API 请求增加一次有索引的本地验证读取。v1 接受这个成本，不引入正向认证 cache 后再宣称撤销立即生效。高并发与 last-used 写放大通过边界清晰的负载测试和聚合观测处理。

本选择不意味着摘要可以当公开数据，也不对数据库写入、进程内存或 bearer 被窃后的使用提供额外防护。

### D3. 保留 OIDC，个人 Token 不继续签发个人 Token

浏览器继续 OIDC Authorization Code/PKCE + server-side session。API 同时接受既有 IdP access JWT 和新个人 Token。签发个人 Token 必须是 primary OIDC 认证，并持有单独的 `access-token.write`；个人 Token 不可委托这个权限。

CLI 提供签发和轮换命令，但这些命令使用外部安全提供的 OIDC API access token。日常 Fleet/Profile/Jobs/日志等操作使用普通个人 Token，不要求 primary authentication。

**理由：**避免一个泄露的个人 Token 自我复制、改变到期时间、永久延续权限。签发与业务使用是不同风险操作。既有 OIDC API JWT 路径可直接用于 CLI，无需增加匿名设备配对端点或向 CLI 下发 confidential web client secret。

**代价：**用户不能只拿一个个人 Token 完成无限续期。完全自动化轮换必须另外安排 primary OIDC credential；某些 Provider 需要配置 API audience/scopes。CLI 初版不内置 Provider-specific token acquisition，也不假设 Device Flow 可用。这是明确的安全选择，不把 token-create 功能留作 Web-only。

后续若确有无浏览器 CLI 登录需求，可单独评估标准 native flow 或安全的浏览器授权手递交；不能直接在本提案里使用密码 grant 或复用 Web client secret。

### D4. 一次性明文交付优先于完整 secret-response 幂等回放

签发原子提交 Token 验证器、元数据、audit 和无 secret 的幂等记录。只有首次成功响应包含 secret；同 key 重试恢复原 ID/metadata，但不恢复 secret，也不生成另一个 Token。

**理由：**“服务端不可恢复 secret”和“响应丢失后能无限原样回放 secret”不能同时成立。通用 mutation 回执可以持久化，但包含凭据的签发回执必须是明确例外。

**代价：**提交之后响应丢失会产生用户拿不到 secret 的 Token。恢复路径是原 key 找到 ID、撤销并重签发；SDK/CLI 必须报告这一状态，不把 metadata-only 200 当作成功取得 secret。新旧 Token 轮换以保存新 credential 成功后再撤旧为顺序，承认其中的部分失败。

### D5. Client 使用 HTTP 边界，而不是本地调用 ControlPlane

新增纯 transport 的 `shaula-api-types`，由 server DTO 和 `shaula-client` 共享；client 不依赖 HTTP handler、SeaORM 或 daemon。现有 `shaula` binary 增加薄 `client_cli` presentation 模块，远程分支不初始化 serve。

**理由：**HTTP 是已被 Web 使用、带有统一认证、CSRF/非 cookie 分流、授权、ETag、限流和错误语义的边界。直接调用 `ControlPlane` 会绕过这些规则，使本地 CLI 得到不同能力，也无法在独立工作站上远程操作。

共享 wire types 减少 Rust server/client 漂移，但必须提取实际 handler 输出；不能把 core structs 全部标为 Serialize 就宣称契约完成。请求严格、响应兼容新增字段，HTTP fixtures 才是兼容性证据。

**代价：**增加两个 crates 和显式 domain/transport mapping；类型提取需要覆盖目前手写 JSON 的 handler。收益是外部 Rust 用户可以单独依赖 client，不引入 Terraform、数据库和 Web 构建依赖。

### D6. 对等以 workflow 和协议行为验收，不以命令数量验收

CLI 必须覆盖所有已有业务操作以及读取辅助数据所需的 API。矩阵记录每个 HTTP operation、Web 入口/支撑关系、SDK 方法、CLI 命令、scope、写入条件和测试要求。

保留 NoOp、Accepted、Converged、Blocked、Failed、Superseded 等区别；finalize 是账本处置，不是普通任务成功，更不是远程 force destroy。日志缺口、retention、未验证 Job 关联不得因 CLI 简化输出而丢失。

**理由：**一个能发送任意 JSON 的通用 request 命令不等于可用的操作工具。用户需要可靠脚本、可审阅编辑、确定的退出码和安全的重试，而不是学习 Web 内部所有 HTTP 细节才能完成工作。

**代价：**首版工作量高于“加一个列出 Fleet 的命令”。可以分阶段合入，但未覆盖的 mutation 不得标记 full parity。现有 Web 不具备的 API 操作可作为 CLI/API 扩展，不虚构它们原本有 Web 入口。

### D7. 凭据轮换不应破坏合法重试，也不应绕过现在的授权

幂等 namespace 包含 stable principal 而不是 token ID；每次 replay 先用当前 credential 授权。历史没有 principal 证据的记录隔离处理，不能猜测 owner 或自动当 cache miss 再执行。

**理由：**同一用户在 Token 轮换后继续恢复同一已接收的请求是合理需求；不同用户使用同一个幂等 key 不应获得对方回执。把 Token ID 混入 actor 或只增加 client UUID 都不能解决服务端 namespace 的边界。

**代价：**需要修改 store port、唯一索引、helper 和所有相关 mutation 调用链，并制定 legacy-record 兼容规则。这是本次发布的前置安全工作，不应拖到 Token 已上线之后。

## Alternatives considered

| 方案 | 本次不采用的原因 |
| --- | --- |
| 只支持 IdP API JWT，不增加个人 Token | 继续要求每个用户/脚本理解 Provider 的 API client 与 token 获取，不满足用户的自助 Token 管理需求 |
| 一个全局静态 API key / proxy headers | 没有用户归属、独立撤销和明确 scopes；违背现有身份边界 |
| 将个人 Token 保存在 GitHub auth profile | 把 Shaula 管理身份和下游平台凭据混为一谈；scope、生命周期与所有权都不相同 |
| Shaula 自签 JWT，并只依赖 exp | 不能满足本地撤销/权限下降；补 denylist 后丧失纯无状态收益 |
| 个人 Token 可任意创建其他个人 Token | 泄露后可自我延续；加入 parent tree、子代级联撤销和祖先 expiry 又显著扩大范围 |
| 可解密保存 secret 或存完整签发回执 | 简化丢响应恢复，但数据库/备份保留可直接使用的 credential，不接受此代价 |
| CLI 直接调用 Rust domain ports 或 SQLite | 绕过 HTTP 权限和协议；不能保证与 Web 同一行为 |
| 让 client 依赖 shaula-http 并复用所有 DTO | 把 server/build 依赖带给消费者，且目前并非所有 DTO 对应真实 wire 输出 |
| 首版只有 read-only CLI | 不满足用户要求的业务操作对等；只能作为内部实施里程碑 |
| 首版增加通用 `/changes` 与 `/changes` list | 不解决核心需求，扩大服务端聚合授权/分页范围；按现有 typed change routes 实现更明确 |
| 首版额外实现设备授权或浏览器 handoff | 当前认证边界没有相应流转；另起完整协议比复用既有 primary API JWT 更重，留作独立决定 |

## Security and operational consequences

Access Token 是 bearer credential，持有者可在其有效权限内使用；TLS、无 redirect、保密存储和日志脱敏仍是必要条件。个人 Token 不用于 UI/assets，不进入 Runner、Terraform、下游平台或 worker/state 环境。

撤销是本地已提交状态变化，不保证取消已经通过认证的请求，也不取消已提交业务 mutation。Provider 停机不自动撤销个人 Token，但首次 daemon 启动仍需要 mandatory OIDC 初始化；运行期 readiness 与个人 Token 是否可验证是不同事实。

备份 rollback 可能恢复旧 Token 状态。必须先禁用个人 Token 认证，完成各 owner 的 primary OIDC 撤销或管理员在维护窗口的受审计离线批量撤销，再重新开放；初版没有跨用户管理员 API，不能假设一个用户就能通过 HTTP 撤销所有人的 Token。只把 revocation 写回同一份数据库无法构成防回滚的外部证据。

当前 bootstrap authorization 是权限政策，不具备用户目录实时同步。未来需要 IdP 禁用实时传播、管理员跨用户 Token 管理、资源级 Token ACL 或 service account 时，应独立定义身份/授权/撤销协议，不从本次 `access-token.*` scopes 暗中扩展。

## Required amendments

接受本 ARD 时同步修订 spec 0007（crate ownership）、spec 0008（Web Token 页与 CLI 对等）、spec 0009（新增凭据种类及有限 opaque 例外），并为 ARD-0013 添加 amended-by 引用。spec 0002/0005 的幂等协议与实现须统一到 principal-scoped namespace；spec 0019/0028 的既有日志/finalize 安全语义保持不变。

`docs/README.md` 增加新规范的权威入口，`CONTEXT.md` 增加 Personal Access Token / primary authentication / credential provenance 术语，`oidc-deployment.md` 解释授权配置与撤权边界，`IMPLEMENTATION_STATUS.md` 分项记录实际实现和证据。不能把本 proposed ARD 当成已经落地的凭据系统。

## Validation and acceptance

接受架构决定不等于发布。发布需完成 spec 0039 的 AT/CL/UI/OP 验收矩阵、真实 HTTP handler fixtures、Web/CLI 同权限同效果测试和一次真实 OIDC 部署验证。所有 secret-bearing 类型和一次性签发恢复路径需要单独安全审查。

本文编写阶段只做源代码研究和文档一致性检查，没有提交功能实现，也没有运行 Rust、Web 或真实 Provider 测试。
