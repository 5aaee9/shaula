# Spec 0039 Appendix: API / Web / Rust Client / CLI Parity

- Status: **Draft / proposed**；这是目标覆盖清单，不是已实现/测试通过报告。
- Baseline: `118a732405069908d62fce7a9639c1079d9d348e`，2026-09-25 核对。
- Parent contract: [spec 0039](0039-user-access-tokens-and-cli.md)
- Sources: [代码索引](../design/0039-implementation-notes.md#source-index)
- Machine-readable companion: [route inventory JSON](../design/0039-route-inventory.json)

## 1. 使用方式

本表包含 **43 个已注册的管理 API/health method+path 组合，以及 6 个拟新增 Token 组合**。PUT 的创建和更新各有 CLI workflow，但仍是同一个 HTTP operation。数量不包含自动 HEAD、UI/assets、OIDC login/logout/callback 或内部 capability listeners。

“Web 关系”区分业务页面、页面支撑 API 和已有但不一定存在专用 UI 的 API 扩展；不能把所有已注册 API 都宣称为已有可点击按钮。所有 baseline 操作纳入 typed client/CLI，以便完整承接 Web 的组合行为并避免遗漏底层能力。

除按 resource-kind 区分的 profile-change 外，scope 列的 `+` 表示 **同时具备**，不是任选。只有写权限的调用者不能自动拥有 detail、impact、--wait；interactive preview 所需的 read 权限单独加到 workflow 说明，不改变原 API 的最小写权限。

每一行实现至少一个实际 HTTP fixture 与一个授权测试；每个业务 workflow 还要经过 §3 的端到端对等检查。源代码扫描更新 route inventory 时须规范化参数名，只将路径结构和 method 作为一致性键。

## 2.1 身份与健康

| ID / HTTP operation | Rust client → CLI | Scope | 条件 / Web 关系 |
| --- | --- | --- | --- |
| ID-01<br>`GET /api/v1/session` | `identity().session`<br>`shaula auth whoami / auth status` | 有效管理身份 | 有效身份；PAT 不取 CSRF。<br>Web session 与 scope 展示。 |
| ID-02<br>`GET /livez` | `health().live`<br>`shaula health live` | 有效管理身份 | 200/503 可为空 body。<br>受保护 API；非专用 Web 页面。 |
| ID-03<br>`GET /readyz` | `health().ready`<br>`shaula health ready` | 有效管理身份 | 200/503 JSON ready；503 不等于协议失败。<br>Web 顶部健康状态。 |

## 2.2 Fleet

| ID / HTTP operation | Rust client → CLI | Scope | 条件 / Web 关系 |
| --- | --- | --- | --- |
| FL-01<br>`GET /api/v1/fleets` | `fleets().list`<br>`shaula fleets list` | `fleet.read` | 非 cursor 列表；本地搜索/排序须标明。<br>Fleets 列表。 |
| FL-02<br>`GET /api/v1/fleets/{fleetKey}` | `fleets().get`<br>`shaula fleets get / fleets edit 前置读取` | `fleet.read` | 取得强写版本，保留 spec 原始数字。<br>Fleet 详情与编辑。 |
| FL-03<br>`GET /api/v1/fleets/{fleetKey}/status` | `fleets().status`<br>`shaula fleets status [--watch]` | `fleet.read` | 展示 desired/observed、依赖、容量和 conditions。<br>Fleet runtime 详情。 |
| FL-04<br>`PUT /api/v1/fleets/{fleetKey}` | `fleets().put`<br>`shaula fleets create / update / edit` | `fleet.write` | create=*；更新 If-Match；稳定幂等 key；200 NoOp/202。<br>创建与编辑；依赖/输入选择使用支撑 API。 |
| FL-05<br>`DELETE /api/v1/fleets/{fleetKey}` | `fleets().retire`<br>`shaula fleets retire` | `fleet.retire` | If-Match；幂等；异步 Decommission，不 force delete。<br>Fleet 退休。 |
| FL-06<br>`GET /api/v1/fleet-changes/{changeId}` | `changes().get_fleet`<br>`shaula changes get / wait --kind fleet` | `fleet.read` | 按 ID 查询，不是 Change 列表。<br>ChangeNotice / Changes。 |

## 2.3 Template Pool

| ID / HTTP operation | Rust client → CLI | Scope | 条件 / Web 关系 |
| --- | --- | --- | --- |
| PL-01<br>`GET /api/v1/template-pools` | `pools().list`<br>`shaula pools list` | `template.read` | 非 cursor 列表。<br>Pools 页面。 |
| PL-02<br>`GET /api/v1/template-pools/{poolKey}` | `pools().get`<br>`shaula pools get / edit 前置读取` | `template.read` | 保留原始成员/权重和强版本。<br>Pool 编辑支撑。 |
| PL-03<br>`PUT /api/v1/template-pools/{poolKey}` | `pools().put`<br>`shaula pools create / update / edit` | `template.publish` | create=* 或 If-Match；幂等；不能只支持单模板。<br>Pool 创建/更新。 |
| PL-04<br>`DELETE /api/v1/template-pools/{poolKey}` | `pools().retire`<br>`shaula pools retire` | `template.retire` | If-Match；幂等；保留服务端引用约束。<br>Pool 退休。 |
| PL-05<br>`GET /api/v1/template-pool-changes/{changeId}` | `changes().get_pool`<br>`shaula changes get / wait --kind pool` | `template.read` | 专用 pool Change，不能走 profile-change。<br>Pool 变更支撑 API；不假设 Changes 页已有 pool selector。 |

## 2.4 Template

| ID / HTTP operation | Rust client → CLI | Scope | 条件 / Web 关系 |
| --- | --- | --- | --- |
| TP-01<br>`GET /api/v1/template-sources` | `templates().list_sources`<br>`shaula templates sources list` | `template.read` | 读取已登记 source；不伪造同步默认模板 API。<br>Template 发布/更新的 source 选择。 |
| TP-02<br>`GET /api/v1/template-artifacts/{digest}/variables` | `templates().variables`<br>`shaula templates variables <digest>` | `template.read` | 变量/schema/bindings 投影保留服务端规则。<br>可视化变量与 bindings 编辑支撑。 |
| TP-03<br>`PUT /api/v1/template-artifacts/{digest}` | `artifacts().upload`<br>`shaula templates artifacts upload --file ...` | `template.publish` | content-addressed bytes；非 JSON；201；不假设此端点处理 If-Match。<br>底层 API 能力；不假设独立 Web 上传页面。 |
| TP-04<br>`GET /api/v1/template-profiles` | `templates().list`<br>`shaula templates list` | `template.read` | profiles envelope；非 cursor 列表。<br>Templates 列表。 |
| TP-05<br>`GET /api/v1/template-profiles/{profileKey}` | `templates().get`<br>`shaula templates get` | `template.read` | 保留 desired/active/backend/status 与强版本。<br>Template 详情。 |
| TP-06<br>`PUT /api/v1/template-profiles/{profileKey}` | `templates().publish`<br>`shaula templates publish / templates revisions publish` | `template.publish` | create=* 或 If-Match；幂等；完整发布，不等同 Update。<br>新发布 / 新 Revision。 |
| TP-07<br>`POST /api/v1/template-profiles/{profileKey}/updates` | `templates().update`<br>`shaula templates update` | `template.publish` | If-Match；禁止 If-None-Match；bindings omitted/null 语义；幂等。<br>Template Update。 |
| TP-08<br>`GET /api/v1/template-profiles/{profileKey}/revisions/{revision}` | `templates().revision`<br>`shaula templates revisions get` | `template.read` | 指定 revision；非敏感 bindings；不杜撰 revisions list。<br>Template 修订详情与更新基线。 |
| TP-09<br>`GET /api/v1/template-profiles/{profileKey}/revisions/{revision}/input-contract` | `templates().input_contract`<br>`shaula templates input-contract` | `template.read` | 返回批准选项/约束；CLI interactive 使用它。<br>Fleet input 表单支撑。 |
| TP-10<br>`PUT /api/v1/template-profiles/{profileKey}/revisions/{revision}/attestations/{attestationKey}` | `templates().put_attestation`<br>`shaula templates attestations create` | `template.attest` | 必须 If-None-Match:*；immutable；201；不假设幂等 header 被处理。<br>已注册 API 扩展；不假设 Web 存在提交入口。 |
| TP-11<br>`GET /api/v1/template-profiles/{profileKey}/revisions/{revision}/attestations/{attestationKey}` | `templates().get_attestation`<br>`shaula templates attestations get` | `template.read` | subject/result/suite/verified；不触发运行测试。<br>已注册 API/证据读取；不假设独立 Web 页面。 |
| TP-12<br>`DELETE /api/v1/template-profiles/{profileKey}` | `templates().retire`<br>`shaula templates retire` | `template.retire` | If-Match；幂等；异步 retirement。<br>Template 退休。 |
| TP-13<br>`GET /api/v1/profile-changes/{changeId}` | `changes().get_profile`<br>`shaula changes get / wait --kind profile` | `template.read OR auth.read (resource-kind dependent)` | 按实际 resource_kind 检查对应 read，非二者任选。<br>Profile ChangeNotice / Changes。 |

## 2.5 Authentication Profile

| ID / HTTP operation | Rust client → CLI | Scope | 条件 / Web 关系 |
| --- | --- | --- | --- |
| AP-01<br>`GET /api/v1/github-auth-profiles` | `auth_profiles().list`<br>`shaula auth-profiles list` | `auth.read` | 支持 github_app 与 forgejo_token；非 cursor 列表。<br>Auth 列表与 Fleet selector。 |
| AP-02<br>`GET /api/v1/github-auth-profiles/{profileKey}` | `auth_profiles().get`<br>`shaula auth-profiles get` | `auth.read` | desired/active/bindings/liveFleets；secret write-only。<br>Auth 详情/rotation 前置读取。 |
| AP-03<br>`GET /api/v1/github-auth-profiles/{profileKey}/status` | `auth_profiles().status`<br>`shaula auth-profiles status [--watch]` | `auth.read` | 独立 status shape。<br>Auth 状态/支撑 API。 |
| AP-04<br>`GET /api/v1/github-auth-profiles/{profileKey}/impact` | `auth_profiles().impact`<br>`shaula auth-profiles impact` | `auth.read` | 当前 liveFleets；不是长期稳定授权快照。<br>策略修改/轮换影响预览。 |
| AP-05<br>`GET /api/v1/github-auth-profiles/{profileKey}/installation-link` | `auth_profiles().installation_link`<br>`shaula auth-profiles installation-link [--open]` | `auth.read` + `auth.write` | 两项权限都要；GitHub App；不把 Shaula Bearer 发往返回 URL。<br>安装导航。 |
| AP-06<br>`PUT /api/v1/github-auth-profiles/{profileKey}` | `auth_profiles().publish`<br>`shaula auth-profiles publish / rotate` | `auth.write` | create=* 或 If-Match；幂等；secret 从文件/stdin 读取。<br>创建/轮换两种后端凭据。 |
| AP-07<br>`POST /api/v1/github-auth-profiles/{profileKey}/policy-updates` | `auth_profiles().update_policy`<br>`shaula auth-profiles policy-update` | `auth.read` + `auth.write` | If-Match + base_revision + target_policy；幂等；禁止 create header。<br>GitHub 目标策略编辑。 |
| AP-08<br>`GET /api/v1/github-auth-profiles/{profileKey}/revisions/{revision}` | `auth_profiles().revision`<br>`shaula auth-profiles revisions get` | `auth.read` | 历史 Unsupported 只展示真实已有投影。<br>已注册 revision 读取/详情支撑。 |
| AP-09<br>`DELETE /api/v1/github-auth-profiles/{profileKey}` | `auth_profiles().retire`<br>`shaula auth-profiles retire` | `auth.retire` | If-Match；幂等；异步 retirement。<br>Auth 退休。 |

## 2.6 Jobs / Generation / Logs

| ID / HTTP operation | Rust client → CLI | Scope | 条件 / Web 关系 |
| --- | --- | --- | --- |
| JB-01<br>`GET /api/v1/jobs` | `jobs().list`<br>`shaula jobs list` | `fleet.read` | JobsQuery；opaque cursor；保留 backend/result/association/freshness。<br>Jobs 筛选与翻页。 |
| JB-02<br>`GET /api/v1/jobs/{id}` | `jobs().get`<br>`shaula jobs get` | `fleet.read` | observations、truncation、关联 Generation；不猜任务成功。<br>Job 详情。 |
| GN-01<br>`GET /api/v1/generations` | `generations().list`<br>`shaula generations list` | `fleet.read` | GenerationsQuery；association=unassigned；opaque cursor。<br>Unassigned runners。 |
| GN-02<br>`GET /api/v1/generations/{id}` | `generations().get`<br>`shaula generations get` | `fleet.read` | state/subphase/jobs；保留 unverified/ambiguous。<br>Runner 详情。 |
| GN-03<br>`POST /api/v1/generations/{id}/finalize` | `generations().finalize`<br>`shaula generations finalize --reason ... --yes` | `fleet.retire` | 幂等；无 If-Match；仅 Quarantined；账本 Succeeded；修复无效 header。<br>FinalizeRunner。 |
| LG-01<br>`GET /api/v1/generations/{id}/invocations` | `invocations().list`<br>`shaula invocations list <generation-id>` | `fleet.read` | 分页；latest_create/latest_destroy；operation/capture metadata。<br>OperationLogs 尝试选择。 |
| LG-02<br>`GET /api/v1/invocations/{id}/logs` | `logs().read_page / follow`<br>`shaula logs read / download [--follow]` | `fleet.read` + `logs.read` | cursor/limit_bytes/phase/stream；gap/version/lost_bytes。<br>日志查看与翻页。 |

## 2.7 新增个人 Token

| ID / HTTP operation | Rust client → CLI | Scope | 条件 / Web 关系 |
| --- | --- | --- | --- |
| AT-API-01<br>`GET /api/v1/access-tokens` | `access_tokens().list`<br>`shaula tokens list` | `access-token.read` | 仅 owner；新增分页，不套到既有非分页 API。<br>新增 Token 列表。 |
| AT-API-02<br>`POST /api/v1/access-tokens` | `access_tokens().issue`<br>`shaula tokens create / rotate 签发阶段` | `access-token.write (primary OIDC only)` | mandatory idem；201 secret once / 200 metadata-only recovery。<br>新增签发/一次性显示。 |
| AT-API-03<br>`GET /api/v1/access-tokens/current` | `access_tokens().current`<br>`shaula tokens current` | 有效身份；特殊 current 规则见说明 | 仅当前有效 PAT；无额外 scope。<br>CLI 自查；Web 用普通 owner detail。 |
| AT-API-04<br>`DELETE /api/v1/access-tokens/current` | `access_tokens().revoke_current`<br>`shaula tokens revoke --current / auth logout --revoke` | 有效身份；特殊 current 规则见说明 | 仅当前 PAT；同步；成功后原 Token 认证失效。<br>CLI 自撤销；Web 不改成 PAT session。 |
| AT-API-05<br>`GET /api/v1/access-tokens/{id}` | `access_tokens().get`<br>`shaula tokens get` | `access-token.read` | 仅 owner；metadata 无 secret。<br>新增 Token 详情。 |
| AT-API-06<br>`DELETE /api/v1/access-tokens/{id}` | `access_tokens().revoke`<br>`shaula tokens revoke / rotate 撤旧阶段` | `access-token.revoke` | 仅 owner；If-Match；同步；幂等恢复前重新授权。<br>新增撤销。 |

## 3. 组合 workflow 验收（不能用“有这个端点”替代）

| ID | Web 行为 | CLI 必须实现的等价行为 | 关键失败测试 |
| --- | --- | --- | --- |
| WF-01 | Fleet 创建表单选择 auth/template/pool 和批准输入选项 | 从同一 registry 和 input-contract 读取；interactive 或完整 JSON 文件表达；支持 GitHub/Forgejo 和 inline/shared pool | 依赖未 Active、无权读取 selector、非法输入、重复创建、未知字段 |
| WF-02 | Fleet 编辑、容量/labels/inputs 变更 | edit 绑定读取时版本；update 显式 --if-match；保存未编辑字段和大整数；遵守 identity/occupancy 门槛 | 编辑期间他人修改后 412；不能自动覆盖；不能绕过 occupancy |
| WF-03 | Fleet 退休并观察资源收敛 | retire 打印 accepted；可按 fleet Change wait；保留阻塞原因；不删除本地数据库替代 API | busy/quarantine/权限丢失/超时仍保留 receipt |
| WF-04 | 发布模板、新 Revision、默认 source Update | sources → variables → publish/update；明确新发布与继承 bindings 的 Update 区别 | top-level null、空 map、per-field keep sentinel、schema 变化、secret 不回读 |
| WF-05 | 模板详情查看 Active/desired/状态与非敏感 bindings | get/revisions get/variables/input-contract 输出全部可读投影；不把缺字段填成 secret | Unsupported/缺修订/readonly；unknown 字段不丢失到误导性 summary |
| WF-06 | Pool 创建、成员权重编辑、退休 | 读取模板候选、完整成员编辑、条件写入及独立 pool Change wait | 错用 profile-change 路径、无效权重、并发替换、被引用退休 |
| WF-07 | Auth 新建与 rotation | 支持 github_app schema2 和 forgejo_token；敏感输入从 file/stdin；读取影响并确认；发布新 revision，等待原 Change | secret 进 Debug/argv、错误 provider 字段、读取基线变化、部分进展 |
| WF-08 | Auth policy update / installation navigation | impact + base_revision + If-Match + target_policy；安装 URL 只展示或显式打开，绝不附带 Shaula token | 缺 auth.read 或 auth.write；base 失效；跨 origin 凭据泄露 |
| WF-09 | Jobs 搜索、翻页、详情、关联 Runner | 同一 JobsQuery；显示 observed_status、reported_result、GitHub conclusion 或 Forgejo Task result、freshness、association | stale snapshot 不推断结论；unverified 不升级 verified；observation truncated 明示 |
| WF-10 | Unassigned runner、quarantine 提示、FinalizeRunner | association=unassigned；查看证据；带理由且明确确认后 finalize；识别 ledger-only Succeeded | 非 Quarantined、理由为空/超长、无 fleet.retire；不跟不存在的 Change URL |
| WF-11 | 选择 Apply/Destroy 执行尝试并看日志 | invocation list 包含 latest_create/latest_destroy；operation 决定展示名；日志 phase 保持 init/plan/apply | 错把 Destroy 当 phase；缺 logs.read；日志有 gap/expired/capture failure |
| WF-12 | Change 回执及按 ID 查找 | 区分 fleet/profile/pool；保留异步语义、失败原因和资源版本；NoOp 不轮询空 ID | 202 不等于 Converged；Superseded 不算成功；unknown 状态受 timeout 约束 |
| WF-13 | 新个人 Token 签发、一次显示、撤销 | primary OIDC 签发；显式安全 secret 输出；日常 PAT 执行业务；个人 Token 不自签发 | response lost + replay 无 secret；文件保存失败；不同 owner；current 自撤销后 401 |
| WF-14 | 新旧个人 Token 轮换 | primary 签发 → 安全保存 → session/current 验证 → 按用户选择撤旧 | 新 secret 未保存不能撤旧；撤旧失败报告双活；不缓存 primary JWT |

### 3.1 查询与筛选契约

JobsQuery 已有字段为 `limit`、`cursor`、`fleet_key`、`status`、`repository`、`job_name`、`since`、`until`。CLI 映射 `--fleet` → fleet_key、`--job-name` → job_name；time flags 转成当前 history API 所需的 UTC Unix milliseconds，并测试边界，不使用本地化日期字符串作为 wire 值。

GenerationsQuery 已有 `limit`、`cursor`、`fleet_key`、`status`、`association`、`since`、`until`。`--association unassigned` 是已存在的未关联筛选；不能把任意 enum 值直接扩展成新的服务端能力。改变筛选时丢弃原 cursor；跨筛选复用由服务端拒绝。

日志筛选为既有 cursor/limit_bytes/phase/stream；phase 允许 init/plan/apply，stream 为 stdout/stderr。Invocation 的 operation Create/Destroy 和 command phase 是两个维度。

Fleet、Profile、Pool 目前是完整列表；CLI 的 --search/--sort 可以在已返回数据上执行，必须标注本地处理。不得将不支持的 query 悄悄发给服务端再假定得到正确过滤结果。`--all` 只对分页端点循环 cursor，默认有总项数/总时限预算；结果不完整必须显式标记。

### 3.2 精确 wire fixtures

以下差异必须来自真实 handler fixture，不从惯例推断：

| 类型 | 基线实际形状 / 行为 | SDK 策略 |
| --- | --- | --- |
| 普通 mutation | camelCase `changeId`、`state`、`revision`；200 noOp；写版本在 headers | 独立 NormalMutationWire → Receipt；不使用未接入的 MutationAcceptedDto |
| Profile Change | core ChangeView 的 snake_case 字段 | 独立 wire type 显式 rename；不要假设所有 Change 都 camelCase |
| finalize | `{etag, change:{..., state:"Succeeded"}, no_op}`；202；没有可用通用 Change GET | FinalizeReceipt；主 spec 规定 header 修复；不套用普通 Waiter |
| Profile metadata | desiredRevision/activeRevision 与部分 snake_case 同时存在 | 逐字段映射，不进行全局 camelCase 转换 |
| Template Update | 缺字段与 null、空对象有不同含义 | 请求使用显式三态/四态模型；拒绝意外擦除 |
| livez | 200/503 可为空 | 使用 health 专用 handler，不强制 JSON |
| 日志 | capture_status/content_version/has_gap/lost_bytes + entries/cursor | 元数据不能因纯文本输出而丢失；另附 stderr/JSON metadata |
| 新 Token issue | 201 含 secret；200 replay 无 secret | `Issued` / `RecoveredWithoutSecret` 不同类型与 CLI 退出码 |

## 4. 不得伪造的能力

当前没有全局 `GET /api/v1/changes` 列表，也没有通用 `/api/v1/changes/{id}` 路由。Changes 页本质是按 ID 和类型查询。CLI 不应宣称 `changes list` 已有服务端依据。

当前未见通用 revision-list 路由；`templates revisions get`、`auth-profiles revisions get` 读取指定 revision，不把一次 detail 响应伪装成完整修订历史。默认模板同步由现有 daemon 路径负责；CLI 只显式选择更新，不新增未经设计的同步端点。

浏览器登录、cookie logout、主题切换和 URL 页面导航不是普通业务 API parity 项；CLI auth login/import、logout/local-remove 与 primary OIDC credential 输入有自己的安全语义。不能让个人 Token 访问 UI assets 来“实现登录对等”。

## 5. CI 落地要求

随实施把 JSON inventory 转为实际 route-coverage test 的输入：确保所有管理 router method+path 有 inventory entry；已有条目失去 handler、typed method 或命令覆盖时失败；新增 route 未分类时失败。第三方/内部 listener 与 UI fallback 有明确排除列表，不因自动 HEAD 扩大业务数量。

客户端与 Web 的同一 workflow 使用同一套服务端 fixture/主体/scopes，分别通过真实 Axum HTTP listener 执行，比较准入、提交事实和安全投影，不比较截图布局。禁止仅 stub client response 后宣称服务端兼容性验证完成。

JSON inventory 已接入 `personal_tokens_http` 的双向路由登记/鉴权测试及 `shaula-client/tests/routes.rs` 的 SDK method/path 覆盖。该覆盖不等于 14 项组合 workflow 的 Web/CLI 对等验收；详细证据与剩余项见 [实现状态](../IMPLEMENTATION_STATUS.md)。
