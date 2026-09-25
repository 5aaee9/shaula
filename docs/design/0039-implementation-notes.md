# Spec 0039 — Code Findings and Implementation Plan

- Status: **Draft / proposed**；本 PR 仅增加设计文档，未实现功能。
- Baseline: `5aaee9/shaula@118a732405069908d62fce7a9639c1079d9d348e`
- Inspection date: 2026-09-25
- Main documents: [spec](../specs/0039-user-access-tokens-and-cli.md)、[ARD](../ard/0039-user-access-tokens-and-api-client.md)、[对等矩阵](../specs/0039-cli-parity-matrix.md)

## 1. 调研结论如何影响设计

### F1. 个人 Token 应接在 guard，而不是伪装成 GitHub Auth Profile

S01/S02 显示现有 guard 区分浏览器 cookie 与 API JWT，identity 将 verified iss/sub 编码成 stable Actor。S04 的 `auth.*` 是管理下游平台认证 profile 的业务权限，不是 Shaula 用户的 token 权限。

因此本方案保留既有 primary OIDC 身份，另建 personal-token verifier/store 和 `access-token.*` 权限。新 Token 的 holder 不能凭借 request body 改 owner，也不能仅靠未验证的 prefix 建立 actor。

### F2. 新权限不是只向 enum 追加几个 variant

S03 对 grant.scopes 数量存在 `> 11` 检查，且 scope parser 明列 11 个值；S04 是业务 scope 枚举。添加 3 个新权限后，必须同步 catalog、配置校验、session 投影、Web permission gating、示例配置和测试。

设计使用 explicit grant；没有“所有旧管理员默认有新权限”的角色推断，因为基线只有 scopes 配置，不存在可直接沿用的全局 admin role。

### F3. 现有 Fleet 幂等路径缺少 principal 分区

S07 的 `idempotency_hash` 输入是 kind、resource key、idempotency key、canonical body、precondition；`idempotency_replay` 调用 store 时也未传 actor。S26 的 `IdempotencyInsert` 没有 principal 字段。

这是静态检查确认的当前接口形状；本次未运行跨用户重放复现实验，也没有将它扩大描述为已验证的任意越权。对于此次要支持多种用户 credential 的功能，它足以构成必须修复和回归测试的发布前置项：principal 需进入 namespace/唯一索引，而不是只加进 hash。

原有未知 owner 的幂等记录必须明确隔离。直接丢弃旧记录再接受重试可能重做已完成的操作；spec 规定 LegacyIdempotencyConflict，不依赖拍脑袋回填 owner。

### F4. `dto.rs` 不是完整实际 wire contract

S12 定义了一个 snake_case `MutationAcceptedDto`，但 S05 的 accepted_response 实际输出 `changeId/state/revision`，S18 的 profile Change 则直接序列化 core ChangeView。S08 的 finalize 又直接返回嵌套 MutationAccepted。

因此 shared DTO crate 的提取必须从 handler golden fixtures 出发，不是简单给现有所有 DTO 增加反向 Serde derive。SDK 必须保留跨 endpoint 的现有差异，不能为省代码无版本地重写 Web 所依赖的 API。

### F5. finalize 的 header 和普通异步模型不一致

S08 当前返回 `Content-Location: /api/v1/changes/{id}`；S05 未注册该路径。S09 直接在账本提交后返回 `state: Succeeded`，且提交方法不接收普通 Change row。Web 的 S23 只在接收后刷新 Generation，因此这种不一致不一定在现有 UI 上立即暴露。

设计不凭这个生成的 change ID 假定存在可查询的 Fleet Change，而是保留独立 FinalizeReceipt、删除无效 header、提供 related Generation URL，并明确 Succeeded 是账本处置完成。普通 `--wait` 不能直接复用到这里。

### F6. 有些 HTTP GET 也要求 write scope

S15 的 installation-link handler 要求 auth.read **和** auth.write；S14 的 policy-updates 也要求两者。S08 日志正文需要 fleet.read **和** logs.read。不能把 GET 一律视为“只需 read”，也不能只看顶层菜单来构造 CLI scope 预检。

### F7. Template Update 不是全量 PUT 的别名

S13 明确区分 omitted bindings、禁止的 top-level null、map 内的 keep sentinel。S06 为 Fleet 编辑保留 raw JSON 数字文本。把 Web 表单数据转换成泛化 JSON/YAML 后无条件重新序列化，可能制造错误的 mutation。

设计需要 lossless edit 路径、显式 nullable/omitted 模型，及正常 field/input-contract 查询支持。scope 与状态机规则继续由 server 决定，client 不执行 Terraform validate 来“替代”服务端准入。

### F8. 当前 CLI 和未来 worker 入口不能混为一谈

S10 只有 serve/version，且隐藏 SSH 和 fence dispatch 在 parser 之前。新的远程 CLI 不能启动 server 然后调用本地端口，不能抢占数据目录锁，也不能要求先部署 Terraform。S11 的现有 reqwest/TLS/Serde/clap 依赖能支持分层方向，但不代表 client 已实现。

## 2. 目标改动位置

以下路径除表中明确的既有文件外均为拟新增建议，不声称已经存在。

| 层 | 拟改动 | 必须保持的边界 |
| --- | --- | --- |
| `shaula-core` | Scope catalog；principal/authentication provenance；Token registry/store ports；幂等记录 namespace 参数 | 无 HTTP/JWT/SeaORM 类型渗透；不根据用户输入构造可信 actor |
| `shaula-daemon` | personal-token application service；原子签发/撤销/配额/审计；现有 mutation provenance 和 principal idempotency 传递 | 无外部网络处于数据库事务；Token 撤销不取消业务 cleanup |
| `shaula-store` | Token rows/索引/查询；atomic commits；last-used 合并；幂等查找与唯一约束升级 | digest 不进入 read model；撤销单向；默认不缓存 positive auth |
| `shaula-store-migration` | 新 Token 表、audit provenance、principal-scoped idempotency migration | 旧数据不猜 owner；不把浏览器 session/下游 profile 转成个人 Token |
| `shaula-http/src/oidc/guard.rs` | 提取通用认证 context；加入 prefix 分流与独立 verifier；保留原 JWT/session 分支 | mixed credentials/default deny/CSRF/anonymous allowlist 不变 |
| `shaula-http/src/oidc/config.rs` | 使用扩展的 scope catalog；维持 startup validation | 不引入 no-auth/no-TLS fallback |
| `shaula-http` Token handler modules | 新 6 个 endpoint、strict DTO、rate limit、metadata 投影、一次性响应 | 不复用通用 secret-bearing idempotency body 回放 |
| `shaula-http/src/router/mod.rs` | 注册 endpoints、session additive fields；引用共享实际 wire types | 既有 URL/JSON casing/ETag 保持兼容 |
| `shaula-http/src/router/jobs.rs` | finalize 无效 header 修复 | 不伪造可查询 Change；保留现有账本语义 |
| `shaula-api-types`（新 crate） | 请求/响应 DTO、ProblemDetails、版本/游标、敏感 wrapper | 纯 transport；请求严格，响应兼容新增字段 |
| `shaula-client`（新 crate） | HTTPS transport、credential、typed endpoint modules、MutationAttempt、wait/paging/logs | 不依赖 server/store/Terraform；禁止 credential cross-origin |
| `shaula/src/client_cli/`（新模块目录） | 解析、context、安全凭据文件、typed command、表格/JSON/退出码 | 不放业务规则；不直接调用 ControlPlane |
| `shaula/src/main.rs` / wiring | Remote dispatch；只有 serve 分支加载 server 配置 | SSH/fence 入口次序和 stdio 不变 |
| `web/src` | Token 页面、session 新字段、permission gating、一次性 secret 展示 | 无 browser persistent secret；OIDC 仍是 Web 登录方式 |
| `shaula-http/src/oidc/login.rs` | 新 UI document 与 return-target 入口 | 只添加 exact 已知页面，不放宽任意 return URL |
| 文档与配置 | spec 0007/0008/0009、ARD-0013 amended-by、README/CONTEXT/deployment/IMPLEMENTATION_STATUS | Contract/decision/status/evidence 分开 |

新增 core ports 的具体命名可在实现 review 中调整，但不得把 Token 的 domain policy、store SQL 和 HTTP prefix parser 混进一个巨型模块。仓库 Rust 文件按 AGENTS.md 控制在 400 行内。

## 3. 依赖有序的实施批次

### Batch A — HTTP 契约与身份地基

先冻结矩阵中真实 handler fixtures，完成 transport types 提取的最小骨架；引入强写版本、可信 provenance、scope catalog；实现 principal-scoped 幂等迁移和 legacy handling；修复 finalize header。此阶段不能宣称 Token 可用。

退出条件：旧 Web/API fixtures 无 breaking change；跨 principal 同 key 不共享回执；同 principal 多 credential 能恢复；旧数据不被无证据重放；已登记 mutation 的条件写入没有变弱。

### Batch B — Token 后端和 Web 自助管理

完成 registry/store/migrations、opaque verifier、签发/撤销/metadata、expiry/policy/quota 和一组真实 HTTP 测试；加入 Token 页面与部署授权说明。首发 scope 是显式选择，默认不添加用户 grant。

退出条件：AT 全套测试和 UI-01 完成；一次性签发丢响应、数据库失败、撤销竞态与 old-grant policy 都有可重复测试；用于演示的 UI 不持久缓存 secret。

### Batch C — 独立 Rust client

按 matrix 实现全部 typed API 方法、health、normal/finalize/token receipts、conditional writes、safe errors、pagination 和日志。补 client-alone build、no redirect、secrets redaction、lossless body/edit tests。

退出条件：每个 method+path 都由实际 HTTP listener 驱动的 client tests 覆盖，而不是只比较 mock JSON。新增 DTO 不能为了 client 强行更改现有 server wire。

### Batch D — CLI 全业务 workflow

完成 context/import/logout/身份、所有资源命令、interactive/input-contract 路径、JSON 文件与编辑、强版本、危险确认、secrets file/stdin、receipt/wait/exit codes、logs follow/download，以及 primary OIDC 下的 Token create/rotate。

退出条件：WF-01 至 WF-14 每项有 CLI 验证，不能用通用 raw-request 命令顶替缺失操作。read-only、write-only、无 logs.read 和两个不同 owner 至少四类权限边界被覆盖。

### Batch E — 发布与回退验收

在测试部署执行一次真实 OIDC 浏览器登录、一次 primary API JWT 调用、一次 PAT 创建/访问/撤销。验证 scope 撤销后重启、运行期 Provider outage、启动期 Provider outage、备份恢复默认关闭 Token，以及 schema/client 版本差异。

退出条件：所有 spec 验收条目有实际证据；实施状态文档准确列出完成/未完成项；不能因为本地几条命令能跑就声称完整 Web parity 或生产安全。

这些是实现次序，不是已完成进度，也不包含完成时间承诺。

## 4. 需要接受者明确了解的产品选择

本草案已经给出默认决定，不把未决问题藏在协议中：Token 默认 90 天、最长 365 天；有限 active 配额；不允许个人 Token 自签发；普通 CLI 只需 PAT，签发/轮换需 primary OIDC；初版输出 table/json，原生 YAML 不是必需功能；初版安全文件存储，不强制 keyring 集成。

“Access Token 可以操作 API 资源”不等于“不受权限的 API 万能钥匙”。scope 仍是当前资源类别级别，没有新增每个 Fleet 的对象 ACL。Token 有效期间也不会自动继承用户后来新增的 grants。

修改这些默认决定时应修改 spec + ARD + tests，而不是只在 UI 放宽控件。例如允许 PAT 自签发会需要定义父子凭据、expiry ceiling、级联撤销和审计协议，不是简单删掉一个 if。

## 5. 本次交付和证据边界

本次通过 GitHub connector 读取固定 commit 的源文件、路由、Web 实现和既有规范。没有在本地完成仓库 clone/build，也没有运行 cargo、Playwright、真实 HTTP endpoint 或 Token 安全测试；所以没有声称重放问题已经动态复现。

已经生成的文件是规范/决策/对等清单/实施说明和 machine-readable route inventory。交付时执行的本地检查仅针对这些文档：文件存在、JSON 可解析、operation ID 与 method/path 唯一性、相对链接、引用索引、JSON 示例和 Markdown fences 等一致性。它们不是功能测试证据。

编号 0039 依据本次读取的 spec/ard 最大编号 0038 选择；本 PR 创建前再次确认 main 仍为上述基线。合入前若出现编号冲突，应同步调整本文档组的编号和引用。

## Source index

以下链接全部固定到本次 commit，便于日后检查代码是否已经变化。它们是基线证据，不证明本提案已实现。

| ID | 固定版本源文件 | 与设计的关系 |
| --- | --- | --- |
| S01 | [crates/shaula-http/src/oidc/guard.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/oidc/guard.rs) | Bearer/session 分流、CSRF、匿名 allowlist |
| S02 | [crates/shaula-http/src/oidc/tokens.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/oidc/tokens.rs) | stable actor 编码与 scopes 交集 |
| S03 | [crates/shaula-http/src/oidc/config.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/oidc/config.rs) | scope parser、11 项上限和 grants 配置 |
| S04 | [crates/shaula-core/src/registry.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-core/src/registry.rs) | Actor/Scope、registry ports、回执与错误 |
| S05 | [crates/shaula-http/src/router/mod.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/router/mod.rs) | 实际 routes、accepted_response、ETag 与 health/session |
| S06 | [web/src/lib/api.ts](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/web/src/lib/api.ts) | MutationAttempt、ETag、错误与 raw JSON 编辑 |
| S07 | [crates/shaula-daemon/src/service.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-daemon/src/service.rs) | 幂等 hash/lookup 参数和 ControlPlane 分层 |
| S08 | [crates/shaula-http/src/router/jobs.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/router/jobs.rs) | Jobs/logs scopes、finalize HTTP 回执/header |
| S09 | [crates/shaula-daemon/src/service_generation_finalize.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-daemon/src/service_generation_finalize.rs) | 账本 finalize、Succeeded 与带外确认理由 |
| S10 | [crates/shaula/src/main.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula/src/main.rs) | serve/version 和 hidden SSH/fence dispatch |
| S11 | [Cargo.toml](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/Cargo.toml) | workspace、MSRV、现有 reqwest/TLS/clap/Serde 依赖 |
| S12 | [crates/shaula-http/src/dto.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/dto.rs) | 现有 DTO 与 strict/write-only 输入 |
| S13 | [crates/shaula-http/src/router/template_update.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/router/template_update.rs) | Update 的字段存在性/null 和写前置条件 |
| S14 | [crates/shaula-http/src/router/profile_auth_policy.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/router/profile_auth_policy.rs) | auth.read+auth.write、base_revision、policy update |
| S15 | [crates/shaula-http/src/router/auth_installation_link.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/router/auth_installation_link.rs) | GET 安装导航也要求 auth.read+auth.write |
| S16 | [crates/shaula-http/src/router/profile_auth_reads.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/router/profile_auth_reads.rs) | 两类 backend 元数据与 liveFleets/impact |
| S17 | [crates/shaula-http/src/router/profile_routes.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/router/profile_routes.rs) | artifact/profile/attestation 写协议 |
| S18 | [crates/shaula-http/src/router/profile_reads.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-http/src/router/profile_reads.rs) | profile Change/revision/retirement 的实际 JSON |
| S19 | [web/src/app.tsx](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/web/src/app.tsx) | 当前页面路由与顶部 readiness |
| S20 | [web/src/lib/queries.ts](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/web/src/lib/queries.ts) | Fleet/template/pool/auth selector 与 impact 查询 |
| S21 | [web/src/pages/changes.tsx](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/web/src/pages/changes.tsx) | 按 ID 查询；不是全局 Change 列表 |
| S22 | [web/src/components/change-notice.tsx](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/web/src/components/change-notice.tsx) | 普通 Change 路径与终态停止轮询 |
| S23 | [web/src/pages/job-runners.tsx](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/web/src/pages/job-runners.tsx) | 未关联筛选、runner 详情、finalize 后刷新 |
| S24 | [web/src/lib/jobs.ts](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/web/src/lib/jobs.ts) | 日志/历史/Forgejo 结果投影与轮询行为 |
| S25 | [crates/shaula-core/src/jobs/mod.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-core/src/jobs/mod.rs) | JobsQuery/GenerationsQuery 与历史读模型 |
| S26 | [crates/shaula-core/src/registry/write_records.rs](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/crates/shaula-core/src/registry/write_records.rs) | AuditAppend、IdempotencyInsert 的当前字段 |
| S27 | [AGENTS.md](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/AGENTS.md) | Rust 模块行数与 fmt/clippy/nextest 要求 |
| S28 | [docs/specs/0009-mandatory-openid-connect.md](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/docs/specs/0009-mandatory-openid-connect.md) | 已有 OIDC 管理认证契约与 opaque 排除条款 |
| S29 | [docs/ard/0013-require-openid-connect-for-all-http-access.md](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/docs/ard/0013-require-openid-connect-for-all-http-access.md) | 需部分修订的架构决定 |
| S30 | [docs/README.md](https://github.com/5aaee9/shaula/blob/118a732405069908d62fce7a9639c1079d9d348e/docs/README.md) | 规范/ARD/实现状态的权威分工 |

### 外部协议参考

- [RFC 6750](https://www.rfc-editor.org/rfc/rfc6750.html)：Bearer header/transport/error 的协议背景。本文沿用现有 Bearer 管理接口，而非自行引入新的 OAuth grant。
- [OpenID Connect Core — Subject Identifier Types](https://openid.net/specs/openid-connect-core-1_0.html#SubjectIDTypes)：subject 的 public/pairwise 差异，不应通过 email 合并两个 client 观察到的身份。

Token 默认 TTL、配额、scope 名称、opaque 格式、SQLite 模型和 CLI 命令都是本次设计决定；上述标准不替这些产品选择背书。
