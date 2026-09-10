# Default Template Synchronization and Published Template Updates

- Status: Accepted (2026-09-09); implementation and verification tracked separately.
- Decision: [ARD-0025](../ard/0025-sync-default-templates-and-explicitly-update-published-revisions.md).
- Amends: [spec 0015](0015-template-library-and-variable-discovery.md) filesystem import;
  extends [spec 0005](0005-profile-http-control-plane.md) publication and
  [spec 0008](0008-embedded-web-ui.md) Templates UI.

## 1. Outcome

内置 Docker、Kubernetes 模板随 Shaula 版本更新。Default templates 每个平台使用稳定的
`docker`、`kubernetes` 入口，不因镜像或软件版本变化增加带版本号的默认入口。
已发布模板通过显式 **Update** 操作采用默认模板，创建新的不可变 Revision。
从内置模板发布时，将来源 key 随 Revision 保存到数据库；之后进入 Update 自动定位该来源，
即使内置模板内容已经升级，也不要求用户再次选择 Docker 或 Kubernetes。
更新来源库不自动发布 Profile，也不替换 Fleet 或 Runner Generation 的版本绑定。

## 2. Authoritative default catalog

- `template_source_dirs` 的完整目录集合是默认来源库的权威配置。每次启动扫描所有可信根目录
  的直接子目录，以目录名为 source key；相同内容仍产生稳定 digest。
- 先完成所有目录的打包、archive/manifest/variables 校验及持久化，再在一个 SQLite
  transaction 中替换完整 source 列表。同 key 更新其 digest、platform、engine；不再存在于
  配置集合的旧 source 删除。多个根目录声明同 key 时拒绝启动，不依赖扫描顺序选择胜者。
- 配置的根目录不存在、不可读、被重定向或模板校验失败时，启动失败且原 source 列表不变。
  显式空目录集合 `[]` 或有效空目录意味着空默认来源库。空配置不禁用 HTTP archive 上传。
- 替换仅作用于 source 列表，不删除 archive、Profile Revision、Change、attestation、Fleet
  或 Runner Generation。历史 artifact 始终按原 digest 恢复和清理，不能重新指向最新模板。
- 兼容升级无需硬编码 `*-official-*` 别名：部署恢复使用包内稳定目录，下一次同步移除所有
  不在配置中的旧入口。NixOS 默认指向当前 package 的 `share/shaula/templates`。
- 启动期间的部分 archive 持久化可以在重试时复用；失败不得留下部分更新的可见 source 列表。

## 3. Persisted source association

- 在 `template_profile_revisions` 增加可空的 `source_key`，表示该 Revision 关联的稳定
  Template Source key，例如 `docker`。来源属于精确 Revision，不是可变的 Profile head
  属性；artifact digest、engine、bindings 和 policy 仍由该 Revision 固定。
- 从 Default template 首次发布或发布 New revision 时，客户端携带所选 `source_key`。
  服务器验证来源存在，且其 artifact digest、engine 和 artifact manifest 的 platform 与
  提交目标一致；通过现有条件发布事务，一并写入 Revision、来源、Change 和幂等记录。
  不能信任客户端声称的来源，也不能仅凭 Profile 名称、platform 或镜像名称建立关系。
- 普通 archive 上传或 existing artifact 发布不携带来源，新的 Revision 保存 `NULL`，
  不继承旧 Revision 的来源；即使内容恰好与默认模板相同，也不自动标记为来自该默认模板。
  Default 模式使用该来源声明的 engine；若要自定义 engine，使用 existing artifact 流程。
- 来源记录不外键依赖可变的 `template_sources` 列表，不级联删除或置空。同步、撤下、禁用
  默认来源库不会改变已记录的来源；旧 Revision 仍使用自己的 artifact 完成运行和恢复。
  source key 的重命名视为旧来源撤下和新来源加入，不按名称前缀或 platform 自动迁移关系。
- Revision 读取接口返回 `sourceKey`，有来源时为字符串，无来源时为 `null`。此字段仅描述
  更新入口的关联，不代表已激活、配置兼容或存在可用更新；它不包含 bindings 或凭据。
  发布后的来源与 Revision 一起不可变，唯一历史补录例外见 §6。

## 4. Explicit published update

Templates 已发布模板提供 **Update**，进入独立 review 页面。操作需要 `template.read`
和 `template.publish`；Retiring/Retired Profile 不可更新。
`/templates/{profileKey}/update` 支持直接打开、刷新和登录后返回；只接受合法 Profile key，
不把未知 API、asset 或其他路径作为页面处理。

1. 读取 Profile head、其 desired Revision 和当前默认来源列表，一并冻结为本次 review
   的快照。更新以打开草稿时的 incarnation、desired revision 和 ETag 为基准，后台刷新
   不得悄悄重设草稿的基准、候选列表或目标 digest。
2. 若 base Revision 有 `sourceKey`，按该 key 自动选择当前默认来源并固定来源身份；
   digest 变化或同平台存在多个候选不影响选择。来源已撤下或 platform 不兼容时，展示
   原来源不可用并禁用 Update；不得回退到其他来源。更换来源使用普通 New revision。
   若旧 Revision 无来源，优先预选同 platform 中唯一 artifact digest 与 engineRef 均
   匹配的候选；否则只有一个同平台候选时预选该候选，歧义情况由用户手选。
   无来源时的预选只是 review 便利，不写入数据库、不证明历史来源，也不宣称定制模板有
   “新版本”；只有显式提交携带所选 key 才建立关联。预选只初始化一次，手选或清空后不
   重新覆盖，也不自动批准 policy 或提交 mutation。
3. 展示当前与目标 artifact、来源名称、engine，以及绑定和 Fleet input policy 的保留方式。
   目标与当前 artifact、engine、source key 相同且未显式提供替换 policy 时显示 **Up to date**，
   禁用更新提交。显式提供替换 policy 时允许提交，由服务器判断是否 NoOp，无需浏览器读取
   并比较旧 policy。无来源的旧 Revision 即使内容相同，也可以显式 Update 创建带来源的新
   Revision，不能因 artifact 相同而阻止建立关联。
4. bindings 始终由服务器从精确 base Revision 复用，不读取到浏览器，也不重新输入或改变。
   Fleet input policy 默认保留；用户可以显式采用目标声明的批准选项作为完整替代 policy。
   读取变量不会自动采用默认值或选项；新 required bindings 或不兼容配置需要普通 New revision
   流程重新配置，失败反馈保留 Update 草稿。
5. 仅提交 **Update** 才执行 mutation。沿用 publication admission、static validation 和
   自动 activation。既有 Active 在新候选通过前保持有效；不兼容 platform/bindings contract、
   schema 或 policy 按原门禁拒绝，不自动修正或扩大授权范围。
6. 成功后展示 Change 及新 revision。历史 revision 和秘密绑定字节保持不可变；pinned
   Fleet 仍保留原 pin，采用新 Active 需显式 Fleet replacement 并满足原 occupancy 门禁。
   bare key 引用的 follower Fleet 由 daemon 级联自动升级（spec 0023），本条款不再适用。

## 5. HTTP publication and update contract

普通 Template Profile PUT 和下述 Update 请求均增加可选 `source_key`。省略或 `null`
表示新 Revision 不关联来源，不继承 base 的 key；未知字段仍按 strict JSON 规则拒绝。
Web UI 的 Default 发布和 Update 始终携带所选 key，archive/existing artifact 发布省略它。

`POST /api/v1/template-profiles/{profileKey}/updates` requires `template.publish`,
the existing browser CSRF protection, `If-Match` and the existing idempotency convention.
`If-None-Match` cannot create a Profile through this operation.

```json
{
  "source_key": "docker",
  "artifact_digest": "sha256:<reviewed target archive>",
  "engine_ref": "terraform",
  "fleet_input_policy": { "runner_image": ["explicitly-approved-image"] }
}
```

- `artifact_digest` and `engine_ref` are required and fixed by the reviewed target; the server never
  resolves a moving “latest” pointer after the user submits. The archive must already be stored.
- For a new request with `source_key`, validate the exact key against the catalog and its reviewed
  digest, engine and platform before admitting a Revision or NoOp. A missing source or mismatched
  target returns 422 using existing validation errors, without changing the Profile; the UI keeps
  the draft and requires explicit reload/review. A source key is metadata, not an alternative way
  to resolve a target digest or bypass normal publication validation.
- Update of a base with a recorded source may retain that key, or omit it for an unassociated
  artifact update through the API; an explicitly different source key is rejected with 422.
  Reassociation uses ordinary New revision publication. Neither path mutates the base's source.
- `fleet_input_policy` is optional: omission preserves the base policy; an explicit JSON object
  replaces it in full. `null`, non-objects, unknown fields and a `bindings` field are rejected.
- Resolve bindings and omitted policy from the immutable revision named by `If-Match`, within the
  same Profile incarnation. Do not read whatever happens to be the newest revision at execution time.
- Delegate to the existing conditional publication path. Scope checks precede sensitive reads and
  replay classification. Stale ETags without a valid replay fail; deletion/recreation cannot reuse an
  old incarnation. Concurrent updates cannot both advance the same head.
- Retries with the same idempotency key and request replay the original result even after the head
  advances; changed requests conflict. Update and ordinary PUT must not alias each other's request
  identity. Update identity includes the operation, base incarnation/revision, target digest, engine,
  source key, and whether policy was supplied plus its exact canonical value. Ordinary PUT identity
  also includes source key. A key change, including adding/removing one, changes request identity;
  omitted and `null` source normalize to the same absent value. Preserve the existing identity
  encoding for absent-source requests so pre-upgrade accepted requests remain replayable.
  Changing the base also conflicts. Classify authorized, valid replays before checking the current
  source catalog: a later catalog upgrade/removal must not invalidate an accepted request's replay.
  Bindings stay in protected memory and existing immutable storage, never response/error/audit
  plaintext or a secret-derived public verifier.
- NoOp equality includes source key as well as artifact, engine, bindings and policy. Adding,
  removing or changing the association through an allowed publication creates a new Revision,
  even if executable content is unchanged; a replay always returns its original result.
- Responses use existing Accepted/NoOp, Change, precondition and validation error conventions.
  A `200` NoOp shows that no changes were needed and does not create or poll an empty Change ID.
  UI retains the draft on conflict and requires an explicit reload/review before using a new ETag.
  Missing `If-Match` returns 428, missing Profile/base returns 404, stale/wrong-incarnation bases
  return 412, and conflicting request identity returns 409.

## 6. Legacy migration

- 使用一次版本化 SQLite migration 增加 nullable 字段；迁移与 schema history 原子提交。
  升级前备份数据库，失败时不得保留部分 schema 或部分来源补录。
- 迁移在启动同步默认来源库之前，使用数据库中原有 source 列表补录旧 Revision：只有
  artifact digest 与 engine 完全相同、且已知 Revision platform 一致的唯一候选才可回填。
  无匹配、多个 key 匹配、platform 不一致或无法确定 platform 时保持 `NULL`。
  这恢复的是可确定的内容关联，不宣称证明旧发布操作的历史来源。
- 迁移可以补充来源 metadata，但不得修改旧 artifact、engine、bindings、policy、状态、
  Profile head、Change 或 Fleet/Generation pin。迁移后已撤下的旧 source key 仍保留，
  不因它与新的稳定 key 内容相同而重命名；它走 §4 的来源不可用提示和 New revision 流程。
- 不在后续启动、读取或目录同步时重复推断来源。新上传的定制模板保持无来源；迁移时没有
  可用旧来源库的数据仍可通过显式 Update 建立关联。

## 7. Acceptance

- Upgrade changes a stable source's digest, removes obsolete aliases, and repeated startup is
  idempotent. Removed/default-disabled catalogs do not remove old archive bytes or published pins.
- One bad source, duplicate key, missing root, cache failure or transaction failure leaves the previous
  catalog intact, including failures after an earlier valid source was packaged.
- Default publication persists the validated source key on its exact Revision and returns it after
  restart/restore. Default New revision and Update retain the reviewed key; archive/existing artifact
  publication leaves the new source null without changing older revisions. Spoofed keys, mismatched
  digests/engines/platforms and cross-source Update are rejected without partial publication.
- Migration tests cover unique exact recovery, absent/ambiguous matches, unknown or conflicting
  platform, rollback/retry and a removed source after startup synchronization. Catalog removal cannot
  erase recorded source keys or change archive bytes, bindings, policy, heads or existing pins.
- Update tests cover inherited bindings/policy, explicit policy replacement, unchanged historical
  revision/Fleet pins, forbidden secrets in reads, scope/strict JSON/preconditions, stale ETags,
  concurrent changes, same-key retry and changed-request conflict, including changed source keys,
  source-only revisions, absent-source legacy replay and replay after a catalog change/removal.
- Browser tests cover selection by persisted key for Docker/Kubernetes after digest upgrades and
  among multiple same-platform candidates, missing/incompatible recorded sources without fallback,
  legacy exact/sole-candidate preselection, ambiguous/absent candidates and explicit association;
  one-time initialization and frozen catalog/target after manual changes or background refresh;
  same-artifact state including source identity, retained-config review, explicit policy adoption,
  permission/retirement gates and conflict drafts without a silent retry remain covered.
  An unchanged source/artifact/engine still permits explicit policy replacement; Default mode fixes
  the source engine, and switching to archive/existing artifact removes source key from the request.
  A real daemon/SQLite browser flow covers source readback after navigation, direct Update page
  loading, source-only publication and unchanged-policy NoOp without a spurious Change.
- Deployment acceptance checks exactly the configured default entries, current digests, persisted
  revision source keys, Update preselection and unchanged existing Fleet pins. Activation alone is
  not evidence of a real Runner job or platform conformance.
