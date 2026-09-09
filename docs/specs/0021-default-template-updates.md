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

## 3. Explicit published update

Templates 已发布模板提供 **Update**，进入独立 review 页面。操作需要 `template.read`
和 `template.publish`；Retiring/Retired Profile 不可更新。

1. 读取 Profile head、其 desired Revision 和当前默认来源列表。更新以打开草稿时的
   incarnation、desired revision 和 ETag 为基准，后台刷新不得悄悄重设草稿的基准。
2. 用户选择同 platform 的默认来源并检查目标。没有持久化来源关系时，不仅凭 platform
   宣称定制模板有“新版本”；文案为 **Update from default**。多个来源必须由用户选择。
3. 展示当前与目标 artifact、来源名称、engine，以及绑定和 Fleet input policy 的保留方式。
   目标与当前 artifact/engine 相同则显示 **Up to date**，禁用更新提交。
4. bindings 始终由服务器从精确 base Revision 复用，不读取到浏览器，也不重新输入或改变。
   Fleet input policy 默认保留；用户可以显式采用目标声明的批准选项作为完整替代 policy。
   读取变量不会自动采用默认值或选项；新 required bindings 或不兼容配置需要普通 New revision
   流程重新配置，失败反馈保留 Update 草稿。
5. 仅提交 **Update** 才执行 mutation。沿用 publication admission、static validation 和
   自动 activation。既有 Active 在新候选通过前保持有效；不兼容 platform/bindings contract、
   schema 或 policy 按原门禁拒绝，不自动修正或扩大授权范围。
6. 成功后展示 Change 及新 revision。历史 revision 和秘密绑定字节保持不可变；所有 Fleet
   仍保留原 pin，采用新 Active 需显式 Fleet replacement 并满足原 occupancy 门禁。

## 4. HTTP update contract

`POST /api/v1/template-profiles/{profileKey}/updates` requires `template.publish`,
the existing browser CSRF protection, `If-Match` and the existing idempotency convention.
`If-None-Match` cannot create a Profile through this operation.

```json
{
  "artifact_digest": "sha256:<reviewed target archive>",
  "engine_ref": "terraform",
  "fleet_input_policy": { "runner_image": ["explicitly-approved-image"] }
}
```

- `artifact_digest` and `engine_ref` are required and fixed by the reviewed target; the server never
  resolves a moving “latest” pointer after the user submits. The archive must already be stored.
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
  and whether policy was supplied plus its exact canonical value. Changing the base also conflicts.
  Bindings stay in protected memory and existing immutable storage, never response/error/audit
  plaintext or a secret-derived public verifier.
- Responses use existing Accepted/NoOp, Change, precondition and validation error conventions.
  UI retains the draft on conflict and requires an explicit reload/review before using a new ETag.
  Missing `If-Match` returns 428, missing Profile/base returns 404, stale/wrong-incarnation bases
  return 412, and conflicting request identity returns 409.

## 5. Acceptance

- Upgrade changes a stable source's digest, removes obsolete aliases, and repeated startup is
  idempotent. Removed/default-disabled catalogs do not remove old archive bytes or published pins.
- One bad source, duplicate key, missing root, cache failure or transaction failure leaves the previous
  catalog intact, including failures after an earlier valid source was packaged.
- Update tests cover inherited bindings/policy, explicit policy replacement, unchanged historical
  revision/Fleet pins, forbidden secrets in reads, scope/strict JSON/preconditions, stale ETags,
  concurrent changes, same-key retry and changed-request conflict.
- Browser tests cover source choice, same-artifact state, retained-config review, explicit policy
  adoption, permission/retirement gates and conflict drafts without a silent retry.
- Deployment acceptance checks exactly the configured default entries, current digests, and unchanged
  existing Fleet pins. Activation alone is not evidence of a real Runner job or platform conformance.
