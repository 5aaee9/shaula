# Shaula v1 Fleet HTTP Control-Plane Specification

- Status: Draft
- Date: 2026-09-04
- Persistence: SQLite
- Reconciliation: asynchronous and level-triggered
- Template selection: exact immutable Revision
- Auth handoff visibility: explicit desired/observed Auth Revision Ref
- Observability: Day 0 OpenTelemetry

This specification extends the [Multi-Fleet Runner Scale Set Controller Specification](0001-shaula-runner-scale-set.md). It defines Fleet desired state and HTTP behavior；it does not define direct Runner or Terraform endpoints.

Template/Auth Profile publication, validation and retirement are normative in the [Profile HTTP Control-Plane Specification](0005-profile-http-control-plane.md). The provider-neutral execution seam is normative in the [Template Profile Runtime Specification](0004-template-profile-runtime.md). Kubernetes and Docker details live in their [Kubernetes](0003-kubernetes-runner-resource.md) and [Docker](0006-docker-runner-resource.md) specializations；neither becomes a Fleet HTTP or native daemon object model.

Related decisions：

- [ADR-0005: Manage Fleet desired state through HTTP and SQLite revisions](../ard/0005-manage-fleet-desired-state-through-http-and-sqlite.md)
- [ADR-0008: Keep platform capabilities in Terraform Template Profiles](../ard/0008-keep-platform-capabilities-in-terraform-template-profiles.md)
- [ADR-0009: Manage Template and GitHub Auth Profiles through HTTP and SQLite](../ard/0009-manage-profile-resources-through-http-and-sqlite.md)
- [ADR-0011: Expose v1 HTTP through loopback and an authenticating proxy](../ard/0011-expose-v1-http-through-loopback-and-an-authenticating-proxy.md)

## 1. Outcome

一个 `shaula serve` process 暴露同一 HTTP control Interface，并从 SQLite 管理全部 active Fleets。Template Profile 和 GitHub Auth Profile 也由该 API/SQLite 管理，但由 [Profile HTTP Control-Plane Specification](0005-profile-http-control-plane.md) 定义各自 typed resource contract。

一个 accepted Fleet mutation 只改变 durable desired state。它不等待 GitHub、Template Runtime 或外部平台副作用完成：

```mermaid
flowchart LR
    Client["Authorized HTTP client"] --> HTTP["HTTP Adapter"]
    HTTP --> FleetRegistry["Fleet Registry Module"]
    HTTP --> ProfileRegistry["Profile Registry Module"]
    FleetRegistry -->|"transaction: revision + change + audit + wake"| DB["SQLite"]
    ProfileRegistry <--> DB
    DB --> Supervisor["Per-Fleet supervisor"]
    Supervisor --> GitHub["Rust shaula-scaleset Adapter"]
    Supervisor --> Lifecycle["Runner Lifecycle Module"]
    Lifecycle -->|"Create / Destroy only"| Runtime["Template Runtime"]
    Runtime --> Terraform["Local Terraform subprocess"]
```

Fleet Registry 是 deep Module。HTTP routing、strict JSON、trusted backend actor-context validation、authorization middleware 和 response mapping 位于 driving Adapter；TLS termination 与 caller authentication 位于 deployment reverse proxy，SQLite schema/transactions/migrations 位于 Store Adapter。Registry Interface 负责 canonicalization、Profile reference resolution、authorization facts、optimistic concurrency、idempotency、Revision/Change/audit/outbox commit 和 Decommission rules。HTTP Adapter 不直接调用 Store、GitHub 或 Template Runtime。

Shaula 核心没有 Kubernetes/Docker client 或平台对象语义。Fleet Registry 只认识已验证的 Template Profile key/revision/digest 和 bounded inputs；所有 namespace、socket、endpoint、provider credential 等 binding 固定在 Template Profile Revision 中。

## 2. Required invariants

1. SQLite 的 current Fleet Revision 是唯一 Fleet desired-state source of truth；bootstrap 不含 `fleets`。
2. Fleet mutation 在返回成功前 commit；transaction 和 HTTP handler 中无 GitHub、Terraform 或平台副作用。
3. 每个 effective mutation 创建一个 durable Fleet Revision 和 Fleet Change；有效 no-op 只持久化 replayable response 与 audit，不创建 Revision/Change。
4. Committed Change 在 client disconnect、response loss、daemon crash 或 in-memory wakeup loss 后仍可发现并恢复。
5. Per-Fleet supervisor 周期扫描 outbox、due Change 与 `desired_revision > observed_revision`；in-memory notification 只是 latency optimization。
6. Fleet 精确 pin 一个 immutable Template Profile Revision/artifact digest。以后发布 Profile Revision 不改变 Fleet 或既有 Generation。
7. Fleet 的 desired/observed Auth 状态都是不可拆分的 Auth Revision Ref `(profile_key, revision)`；同 Profile credential promotion 不伪装成 Fleet revision change，跨 Profile replacement 则受零 Resource Occupancy gate 约束。
8. HTTP 不暴露 Runner Update。既有 Runner Generation 只有 Create 和 Destroy，并永久使用原始 Template Revision、Workspace 和 state。
9. Fleet Key 标识一个 incarnation；旧 incarnation 的 ETag/idempotency 不能修改新 incarnation。
10. Direct SQLite write、另一个 writer、bootstrap catalog override 或绕过 Registry 的 CLI 不受支持。
11. PAT/App private key 与 schema-sensitive Template bindings 可以明文存在各自 immutable SQLite Revision；任何 response、audit、error、log、OTel 或 diagnostic 不得包含原值或 prefix/suffix/hash/length 等可推导表示。
12. 普通 `fleet.write` 不能发布 Template artifact 或提交 conformance attestation；digest publication 与 `template.attest` 是独立的 high-trust capabilities。

## 3. Sources of configuration

### 3.1 Daemon bootstrap

`shaula serve --config` 只包含 process-wide native concerns：

- data directory、SQLite filename、artifact/workspace roots 和 ownership lock；
- loopback HTTP bind、trusted reverse-proxy actor contract、authorization policy、body/rate/backlog limits；
- supported IaC executable/install policy；
- Create/Destroy concurrency、timeouts、retry defaults；
- artifact upload/expansion/retention limits；
- OpenTelemetry 和 structured logging configuration。

Bootstrap 在 API ready 前验证并在 process lifetime 内冻结。它 MUST NOT 包含任何 Fleet、Template Profile、GitHub Auth Profile 或平台 binding catalog。缺失或不可用的 persisted resource 只降级其依赖者，不把 static file 变成第二真相源。

### 3.2 HTTP-managed resources

以下都是 SQLite-backed desired resources：

- Fleet；
- Template Profile；
- GitHub Auth Profile。

YAML、Git 或其他系统可以调用 HTTP，但只是 client。Template bytes 先通过 digest endpoint 发布为 immutable artifact，再由 Template Profile Revision 引用。PAT/App private key 通过 Auth Profile PUT 写入 SQLite；schema-sensitive Kubernetes/Docker bindings 通过 Template Profile PUT 写入其 immutable Revision。完整 Profile endpoints 和 state machines 见 [0005](0005-profile-http-control-plane.md)。

### 3.3 Runtime artifacts

SQLite、content-addressed Template artifact store 和 per-runner Workspaces/state 共同构成 durability set。Artifact bytes 不是 desired resource；只有 current `active_revision` 及其已接受 durable conformance attestation 可以接收新的 Fleet reference。已准入 Fleet 的旧 exact Revision/artifact/attestation pin 在引用清除前仍可用于其正常 reconcile、未来 Create、Destroy 与恢复。Workspace/state 只属于一个 Runner Generation，不能成为共享 Fleet configuration。

## 4. Resource model

### 4.1 Fleet Spec

Canonical desired Fleet representation 至少包含：

```json
{
  "key": "linux-x64",
  "spec": {
    "github": {
      "target": {
        "kind": "organization",
        "owner": "example-org"
      },
      "auth_profile_ref": "production-app",
      "scale_set_name": "shaula-linux-x64",
      "runner_group": "Default",
      "labels": ["shaula-linux-x64"]
    },
    "capacity": {
      "min_runners": 0,
      "max_runners": 20
    },
    "template_profile_ref": {
      "key": "kubernetes-linux-x64",
      "revision": 4
    },
    "template_inputs": {
      "size_class": "standard"
    }
  },
  "metadata": {
    "incarnation": "opaque-id",
    "revision": 7,
    "created_at": "timestamp",
    "updated_at": "timestamp"
  },
  "resolved": {
    "template_artifact_digest": "sha256:...",
    "template_conformance_attestation": {
      "id": "opaque-attestation-id",
      "subject_digest": "sha256:..."
    },
    "auth_desired": {
      "profile_key": "production-app",
      "revision": 3
    }
  }
}
```

Repository Target 使用 `{"kind":"repository","owner":"example-org","repository":"example-repo"}`。恰好一种 shape 可接受；fields 是 normalized GitHub names，不是 path fragment、URL 或 generic endpoint。

Template reference 可以显式提交 `{key, revision}`，但该 Revision 必须等于提交时的 current `active_revision`；旧 Revision 不能接收新建或 replacement Fleet reference，但已准入 Fleet 可继续用其 retained exact pin 做正常 reconcile、未来 Create、Destroy 与恢复。Server MAY 接受 bare key，但必须在同一 admission transaction 中解析 current active Revision，并在 Fleet Revision 与返回 representation 中写入 exact key/revision/artifact digest/accepted attestation。Idempotent replay 必须返回第一次解析的 exact subject；以后 Profile promotion 不改变它。

`auth_profile_ref` 是 operator 选择的 Profile key，不是 secret。Fleet admission 要求 Profile 已 Active、Target policy 匹配，并把当时的 current active revision 解析为 durable desired Auth Revision Ref `(profile_key, revision)`。同 Profile promotion 通过独立 Auth Handoff 推进 desired tuple；Fleet Spec 和 desired ETag 不因此改变。把 `auth_profile_ref` 换成另一个 Profile key 是 Fleet replacement：只有 Resource Occupancy 为零且无 active acquisition、GitHub 或 Runner Operation 时才可 admission，并在同一 Fleet transaction 中把新 Profile 的 current active revision 固定为新的 desired tuple。

Template Profile 固定 executable artifact、engine、provider bindings、provider dependency policy、manifest-derived `platform`/`bindings_contract`、accepted conformance attestation 和可用 Fleet input schema。Fleet 只能提交 schema 允许的 bounded values/finite aliases；不能提交 artifact bytes、script、image URL、raw user-data、provider、module、executable、argv、environment、filesystem path、platform endpoint/binding、platform identity、attestation 或 credential。

本地 admission 不声明 GitHub access 此刻可用。Fleet Change 进行 bounded authenticated reads；`401`、`403` 和 access-filtered `404` 形成 access Condition，不能作为 Scale Set 不存在的证据。

### 4.2 Fleet Revision

每次 effective desired mutation append immutable Fleet Revision 并原子推进 desired head。Revision 在一个 Fleet incarnation 内单调递增且不回退。Revision 固定：

- canonical Fleet Spec 和 normalized GitHub Target；
- requested Auth Profile key 及 admission-time desired Auth Revision Ref；
- exact Template Profile key/revision/artifact digest/accepted conformance attestation；
- normalized Template inputs 和 input digest；
- authenticated actor、reason 和 timestamps。

Fast successive capacity changes 可以 supersede 尚未 observed 的中间 Revision；history 保留 audit，已经启动的 Runner Operation 使用其原始 revision 完成或安全 cleanup。

相同 canonical Spec 和相同 resolved Template Revision/digest/attestation 是 no-op。Profile status、同 Profile credential promotion、Fleet Status 或 Assigned Demand 变化不产生 Fleet Revision；跨 Profile Auth replacement 会产生。

### 4.3 Fleet Status

Runtime status 与 desired representation 分离：

```json
{
  "fleet_key": "linux-x64",
  "desired_revision": 7,
  "observed_revision": 7,
  "phase": "Reconciling",
  "dependencies": {
    "template_profile": {
      "key": "kubernetes-linux-x64",
      "revision": 4,
      "artifact_digest": "sha256:...",
      "attestation_id": "opaque-attestation-id",
      "state": "Active"
    },
    "github_auth": {
      "desired": {"profile_key": "production-app", "revision": 3},
      "observed": {"profile_key": "legacy-app", "revision": 5},
      "handoff_state": "Blocked"
    }
  },
  "conditions": [
    {"type": "SpecAccepted", "status": true},
    {"type": "TemplateActive", "status": true},
    {"type": "AuthRevisionObserved", "status": false, "reason": "OwnershipProofFailed"},
    {"type": "GitHubAccessVerified", "status": false},
    {"type": "OwnershipVerified", "status": true},
    {"type": "ListenerReady", "status": false},
    {"type": "Converged", "status": false}
  ],
  "capacity": {
    "assigned_demand": 4,
    "target": 4,
    "effective": 3,
    "occupancy": 4
  },
  "last_error": null
}
```

`observed_revision == desired_revision` 表示 supervisor 已分类该 Fleet Revision，并持久化下一步 intents 或 blocking Condition；不表示 capacity 已收敛。

`observed_auth_ref == desired_auth_ref` 按完整 `(profile_key, revision)` 比较，表示新 credential 的 authenticated access context 与 read-only ownership/absence classification 已持久化。它不表示 Scale Set 已 create/adopt、ID 已绑定、session 已建立或 capacity 已收敛；这些副作用只由普通 Fleet reconciliation 执行。tuple 不同必须作为独立、可恢复、可告警的 Auth Handoff lag 暴露，不能被 Fleet `observed_revision` 掩盖。

Fleet phase 是 `Pending`、`Reconciling`、`Ready`、`Degraded`、`Decommissioning` 或 `Decommissioned`。Conditions 使用 stable finite reason codes 和 sanitized summaries，不含 provider-specific object status。

### 4.4 Fleet Change

Fleet Change 跟踪一个 accepted Fleet mutation 的异步处理，与 Profile Change、Auth Handoff acknowledgement 和 Runner Operation 不同：

```text
Pending -> Running <-> Blocked
            |-----> Succeeded
            |-----> Failed
            \-----> Superseded
```

`Blocked` 非终态，覆盖 Busy Runner、`JobStillRunning`、ownership uncertainty、Quarantine、Profile unavailable、Auth Handoff lag 或其他 dependency。每个 non-terminal Change 持久化 attempt、lease 和 `next_retry_at`；periodic scan 可从 `Blocked` 恢复。

Completion predicate 有界：

- Create：ownership verified、所需 Auth Revision Ref observed、listener established，并在一个 transaction 中记录 Fleet observed revision、所采用 demand checkpoint 和由此派生的 lifecycle intents；不等待不断变化的 capacity 永久静止。
- Capacity replacement：记录 observed Fleet revision、demand checkpoint 和相应 Create/Destroy intents。
- Template replacement：仅在 occupancy 和 active Runner Operations 为零时 admission；新的 current active exact Revision/attestation 被 supervisor observed 后成功。
- Decommission：所有 owned Generations terminal、supervisor stopped、tombstone durable 后成功。

后续 Assigned Demand 触发新的 level reconcile，不 reopen 已完成 Change。同 Profile Auth promotion 触发独立 durable handoff；跨 Profile replacement 是受零占用 gate 保护的 Fleet mutation。

## 5. HTTP Interface

v1 base path 是 `/api/v1`：

| Method | Path | Meaning |
| --- | --- | --- |
| `GET` | `/fleets` | Paginated active Fleet summaries |
| `PUT` | `/fleets/{fleetKey}` | Conditionally create or fully replace desired Fleet state |
| `GET` | `/fleets/{fleetKey}` | Canonical desired Fleet Spec and strong ETag |
| `GET` | `/fleets/{fleetKey}/status` | Runtime status including Auth desired/observed rollout |
| `DELETE` | `/fleets/{fleetKey}` | Request safe terminal Decommission |
| `GET` | `/fleet-changes/{changeId}` | Query asynchronous Fleet Change |
| `GET` | `/livez` | Process liveness without resource details |
| `GET` | `/readyz` | Ability to authenticate management calls, commit and schedule |

同一 server 还提供 `/template-artifacts`、`/template-profiles`、`/github-auth-profiles` 和 `/profile-changes` endpoints；其 request/response 和权限以 [0005](0005-profile-http-control-plane.md) 为准。

v1 不提供 `PATCH`、writable status、raw Runner lifecycle、manual reconcile、force delete、operation cancellation、remote Terraform invocation 或平台对象 endpoint。

### 5.1 Conditional writes

- Create 使用 `If-None-Match: *`。
- Replacement 和 Decommission 要求 desired Fleet `GET` 返回的 strong ETag 进入 `If-Match`。
- 缺失 precondition 返回 `428 Precondition Required`。
- stale precondition 返回 `412 Precondition Failed`，只带 current non-secret revision metadata。
- immutable identity 变化返回 `409 Conflict`。
- syntactically valid but inadmissible Spec 返回 `422 Unprocessable Content`。

Desired ETag 只包含 opaque Fleet incarnation/revision，不含 status、secret 或同 Profile Auth promotion revision。Status churn 和同 Profile promotion 不造成虚假 desired-state conflict；跨 Profile replacement 是真实 Fleet mutation并推进 ETag。

### 5.2 Idempotency

每个 mutation 需要 bounded `Idempotency-Key`，scope 包含 authenticated principal、method 和 canonical Fleet resource。Canonical request hash 包含 method、resource、normalized body 和 normalized precondition；key 本身不是 credential，仍不得写入 log/telemetry。

Transaction 内 replay lookup 在 concurrency check 前：

- 相同 key/hash 返回原 status、headers、resolved exact Template Revision/attestation、admission-time desired Auth Revision Ref 和 Fleet Change；
- 相同 key、不同 request 返回 `409 IdempotencyConflict`；
- 新 key 才执行 conditional validation 与 commit。

No-op PUT 在同一 transaction 中 append audit 并保存 `200 OK` replay response，不创建 Revision/Change。新 idempotency key 仍先检查 ETag 再比较 no-op，避免 stale client 借 no-op 绕过 concurrency。

### 5.3 Asynchronous responses

Effective mutation 仅在 SQLite transaction commit 后返回 `202 Accepted`，包含新 desired ETag、Fleet location 和 Fleet Change location，并明确 acceptance 不等于 external convergence。Request cancellation/disconnect 不取消 committed Change。

有效 identical PUT 返回 durable/replayable `200 OK`。Decommission 完成后 desired `GET` 返回 `410 Gone` 与 audit-safe tombstone metadata；exact DELETE retry 仍由 idempotency record 返回原结果。新 mutation key 不能 revive terminal incarnation。

List endpoint 使用 bounded page size 和 opaque cursor。Tombstone 默认不在 active list；未来 audit view 需要独立 authorization。

## 6. Mutable and immutable behavior

Fleet full replacement 不意味着任意 Update：

- `capacity.min_runners` / `max_runners` 可以在 active 时变化；
- Fleet Key、typed GitHub Target、Scale Set name、runner group 和 labels 在一个 incarnation 内 immutable；
- `auth_profile_ref` 可以替换，但仅在 Resource Occupancy 为零且无 active acquisition、GitHub 或 Runner Operation 时 admission；Target 与 Scale Set identity/ID 不因此改变；
- exact Template Profile Revision 可以显式替换，但仅接受当时的 current active Revision/attestation，且 Resource Occupancy 与 active Runner Operations 都为零；
- Template Profile engine、artifact、manifest-derived `platform`/`bindings_contract` 和 bindings 随 exact Revision 固定，不能由 Fleet field 单独更新；
- incompatible identity 需要新 Fleet；
- capacity/profile replacement 只生成未来 Create/Destroy intents，不调用 Runner 或 Scale Set Update。

Auth Profile 内的 same-identity credential rotation 不是 Fleet replacement。异步 validation 成功后，Profile active head 推进并触发 section 7 handoff；Fleet key、Fleet Revision 和 desired ETag 保持不变。跨 Profile replacement 会产生 Fleet Revision/Change，但复用同一 handoff protocol。

## 7. Auth Revision handoff

Auth Revision Ref 是不可拆分的 `(profile_key, revision)` tuple。每个 Fleet 有 durable `desired_auth_ref`、nullable `observed_auth_ref`、handoff state、attempt、lease、`next_retry_at` 和 sanitized reason；所有 GitHub intent/effect 记录其使用的 exact tuple，不能仅凭裸 revision number 关联。

Desired tuple 有两个合法来源：同 Profile Candidate promotion 将每个 dependent Fleet 从 `(P,n)` 推进到 `(P,n+1)`，写 outbox/Profile Change linkage但不产生 Fleet Revision；零 Resource Occupancy 的 Fleet replacement 将 `auth_profile_ref` 从 `P` 改为 `Q`，解析 `Q` 的 current active revision，并与 Fleet Revision/Change 原子提交 `(Q,m)`。

两种来源都执行同一个可恢复 Auth Handoff：

1. Fleet supervisor 关闭新的 job acquisition，并等待跨越 quiesce fence 的 acquisition 返回或被分类。Decommissioning Fleet 保持 acquisition 永久关闭，但允许 handoff 以 cleanup-only mode 继续。
2. 使用 desired tuple 构造 matching Rust `shaula-scaleset` App/PAT client；不尝试另一 auth kind、旧 revision 或另一 Profile。
3. 已绑定 Scale Set ID 的 Fleet 只执行 authenticated read-only lookup，证明 persisted Target、runner group、Scale Set name/ID/fingerprint 匹配，或持久化可信的 absence classification。尚未绑定 Scale Set 的 Fleet 只验证 Target/runner-group access 并记录 authenticated context。Handoff 本身绝不 create/adopt Scale Set、写或改 Scale Set ID、创建 session、获取 JIT。
4. 只有相应 access/ownership/absence classification durable 后，才在一个 transaction 中推进 `observed_auth_ref` 到完整 desired tuple，并唤醒普通 Fleet reconciliation。
5. 普通 Fleet reconciliation 独占 create-or-adopt、Scale Set ID/fingerprint binding 和 session establish/replace。Active Fleet 只在 desired/observed tuple 相等且新 session ready 后恢复 acquisition；Decommissioning Fleet 不建立 acquiring session，只使用 observed tuple 执行 inventory/removal 等 cleanup。

Production GitHub access 使用 Rust `shaula-scaleset`。测试固定一个 reviewed `github.com/actions/scaleset` Go commit 作为协议与行为 oracle；Go oracle 只运行 conformance/differential suites，不被 production daemon 链接或启动。

Handoff 无法完成时 Fleet 保持 quiesced/Degraded，完整 desired/observed tuples 保持不同并由 durable retry 恢复。Candidate validation 失败则不 promotion，旧 active revision 和 Fleet desired tuple 都不改变；这叫 staged activation，不是 runtime fallback。

Auth Revision 仅在不再是任何 Profile desired/active/observed head 或 Fleet desired/observed tuple，且不存在 in-flight GitHub effect/session、Decommission cleanup 或 recovery record 引用时才可 GC。`Blocked` 是 non-terminal reference：它不能 acknowledgement、释放旧 observed tuple 或使新 desired tuple 被回收。

## 8. Decommission semantics

HTTP `DELETE` 是 safe Fleet Decommission，不是 row deletion：

1. 取得 Fleet exclusive side-effect admission gate；一个 transaction 写 deletion marker、increment mutation fence，并提交 Fleet Revision/Change/audit/wake marker。
2. 立即禁止新 Create 并永久停止 job acquisition；不得取消 durable Auth Handoff，后者只能以 cleanup-only mode 前进。
3. 等待所有 in-flight acquisition 返回或分类后写 `AcquisitionStopped=True`。Inventory 和 Runner removal 使用 observed Auth Revision Ref 的 non-acquiring cleanup path；handoff 不得在 Decommission 中启动 listener、Create/adopt、JIT 或改写 Scale Set ID。
4. 只有此后才让每个 owned Generation 进入正常 Retirement 和 GitHub removal safety gate。
5. `JobStillRunning` 持续 Blocked/retry；只有对仍绑定且可认证读取的 Scale Set 返回 runner-specific already-absent 时才可继续。Scale Set 本身 missing、access-filtered `404` 或 `ScaleSetMissingWithResources` 不能作为 Busy-safe proof，必须保持 Blocked/Quarantined。
6. 使用每个 Generation 原始 Template Revision/artifact/Workspace/state 运行 Destroy。
7. unknown remote Runner、Quarantine、missing state 或 unresolved Busy safety 使 Change 保持可见 `Blocked`，不得 purge。
8. 全部 owned Generations terminal、cleanup Auth references/recovery records 清除后停止 supervisor，写 Decommissioned tombstone，并保留空 GitHub Scale Set。

每个 Create claim 在 JIT/apply spawn 前持 admission gate 重新验证 Fleet revision、mutation fence 和 deletion marker，并持久化 side-effect-start intent。旧 claim 若可证明未开始则 `Superseded`；uncertain start 进入 `CleanupRequired`。Transaction/gate 不跨 child-process lifetime。

Decommission 在 v1 不可逆。Tombstone retention 期间 Fleet Key reuse 被拒绝；无 force path 可遗忘可能残留的基础设施。

## 9. SQLite durability and recovery

Conceptual Fleet schema 包含：

- `fleets`：key、incarnation、desired head、mutation fence、deletion marker；
- `fleet_revisions`：canonical Spec、normalized immutable Target/Scale Set identity、requested Auth Profile key、admission-time desired Auth Revision Ref、exact Template Revision/digest/attestation、input digest、actor/reason/timestamps；
- `fleet_status`：observed revision、phase、Conditions、capacity summary；
- `fleet_auth_handoffs`：desired/observed `(profile_key, revision)` tuples、access/ownership context、state、attempt、lease、retry；
- `fleet_changes`：mutation kind、target revision、state、lease、retry 和 sanitized result；
- `reconcile_outbox`、`idempotency_records`、append-only `audit_records`。

Runner Generation/Operation、Job Observation 和 tombstone records 都由 Fleet Key namespace，并保存 protected opaque `shaula_result` body/digest、exact Template Revision/artifact/attestation/inputs、Workspace/state 和 saved-plan attempt metadata。核心 schema 不含 namespace、Pod、Secret、container、socket 或其他平台 object columns。

Template/Auth Profile typed tables、plaintext credential revisions、attestations、artifact references 和 Profile Changes 由 [0005](0005-profile-http-control-plane.md) 定义。Fleet transaction 的新 Template/Auth reference 只接受相应 current `active_revision`，并通过 reference integrity 防止 desired/observed/in-flight/Decommission/recovery 所需的旧 Revision 或 attestation 提前 retire/GC。

一个短 transaction 完成 precondition、reference resolution、revision append、desired-head change、Fleet Change、audit、outbox 和 idempotency response。它不跨 artifact upload/validation、network、filesystem materialization 或 subprocess。

Commit 后 best-effort notification wake supervisor。Startup/periodic scans 处理 outbox、`desired > observed`、Auth tuple handoff lag 和 due non-terminal Changes。Leases 在 crash 后 expire/recover；因此丢通知、丢 HTTP response 或 crash 只延迟执行，不遗忘 accepted desired state。

SQLite main/WAL/SHM、Template artifact store 和 Workspaces/state 是一个 backup/restore consistency set。单 daemon ownership lock 禁止第二 writer，但不提供 HA 或对 host administrator 的安全隔离。

## 10. Startup and readiness

Startup 顺序：

1. 初始化 local structured logging 和 OpenTelemetry SDK。
2. 验证 bootstrap、HTTP safety、filesystem roots、IaC engines 和 limits；不加载 static resource catalogs。
3. 获取 data-directory ownership lock，migrate SQLite，检查 artifact/workspace consistency。
4. 启动 Fleet/Profile Registries、HTTP server、scheduler、worker pools 和 periodic scanners。
5. 从 SQLite 加载 active Template/Auth/Fleet desired heads，恢复 Profile Changes、Fleet Changes、Auth Handoffs 和 Runner Operations。
6. 为每个 Fleet 独立验证其 exact pinned、仍被 retention/reference rules 保留的 Template Revision/artifact/accepted attestation，以及完整 Auth desired/observed tuples；重启不要求旧 pin 仍是 current Active。随后由普通 reconciliation 恢复 create-or-adopt、ID binding 与 session。

`/livez` 只检查 daemon supervision loop，不访问 SQLite、GitHub、IaC、平台或 OTel exporter。`/readyz` 仅在 authenticated control plane 能 durable commit 与安全调度时 true。Ownership-lock loss、shared storage failure 或 scheduler termination 让 readiness false 并停止新 effects。

Profile/Fleet invalid、Auth Handoff lag、listener failure 或外部依赖 failure 只降级相应资源/消费者。它们不让整个 API unready，也不阻塞无关 Fleet recovery。

## 11. Security

v1 假设一个 administrative security domain，不声明 tenant isolation。若增加 multi-tenancy，object authorization、所有 Store query、artifact ownership 和 metric/log isolation 都需新 ADR。

- v1 只支持 loopback listener；配置 non-loopback address 必须在 startup fail closed。远程访问由 trusted reverse proxy 终止 TLS 并认证 caller；Shaula 暂不内建 TLS、mTLS 或 OIDC，但仍执行 authorization 与 audit。
- Authorization 至少区分 `fleet.read`、`fleet.write`、Fleet retirement、`template.read`、高权限 `template.publish`、独立高权限 `template.attest`、`auth.read`、高权限 `auth.write` 和 Auth retirement。
- 每个 management request，无论经 proxy 还是 direct loopback，都必须携带并通过 selected trusted actor assertion/backend authentication；loopback origin 不会自动产生 actor。缺少或无效 context 的 direct request 必须拒绝，body actor 或普通 caller-supplied identity header 永不可信；proxy 必须 strip 外部 identity headers 后再注入认证结果。具体 assertion/backend-auth format 尚待固定。loopback 不是 tenant boundary。
- Strict JSON 拒绝 unknown fields，并限制 body、strings、lists、pagination、rate 和 backlog。
- Fleet JSON 不接受 PAT、App private key、derived token、JIT、provider credential、platform identity/binding、conformance attestation、raw endpoint 或 executable；它只引用 typed Target、Auth Profile key、current active exact Template Revision 和 bounded inputs。
- Auth Profile PUT 可以携带 write-only PAT/App private key；Template Profile PUT 可以携带 binding schema 标记为 sensitive 的 write-only value。SQLite 允许把两者原始 plaintext 存入对应 immutable Revision。所有 GET/list/status/revision/attestation、Fleet response、audit、errors、logs、traces、metrics 和 panic/diagnostic middleware 永不回显 request body、secret、prefix/suffix/hash/length 或 parser content。
- GitHub control-plane credentials 只交给 GitHub Access Module，永不进入 Template/Terraform/Runner；sensitive Template bindings 只交给 exact Revision 获准的 IaC child，永不进入 Runner/workflow。
- SQLite main DB、WAL/SHM、online copy、backup、migration artifact、crash dump 和 retention/disposal 都是 credential-grade boundary；application-level encryption 不是 v1 requirement，host/filesystem/backup protection 是 deployment responsibility。
- Template artifact upload 是高权限 remote code publication。Server streaming 验证 declared digest/length、archive containment、path/link/device、expansion limit 和 atomic publication；普通 Fleet caller 只能选择已有 accepted attestation 的 current Active Revision，不能注入 code 或自行 attest。
- GitHub Target 只规范化为支持的 `github.com` organization/repository form；不 fetch caller-controlled generic URL。
- IaC subprocess 只接收 operation-specific JIT 和 Profile-declared provider/bootstrap material，不接收 GitHub App key、installation/admin token、PAT、HTTP credential、SQLite access 或 daemon full environment。
- Runner Resource 只接收 JIT。Per-job `GITHUB_TOKEN` 与 workflow 显式选择的 `${{ secrets.* }}` 属于 GitHub/workflow policy，不是 Fleet API 或 Shaula auth data。
- API/telemetry 不输出 JIT、tfvars、state、Profile sensitive bindings、provider raw output、Authorization、idempotency key 或 secret-derived identifier。

## 12. Day 0 observability

Instrumentation 从首个 HTTP implementation 开始存在。Required spans 至少包含：

- `shaula.http.request`，使用 route template 而非 raw path；
- `shaula.fleet.registry.validate`、`shaula.fleet.registry.commit`；
- `shaula.fleet.change.reconcile` 和 state transition；
- Auth Handoff quiesce、access/ownership proof、tuple acknowledgement，以及后续普通 reconcile 的 session handoff；
- request span 到 async Fleet Change、Profile Change 和 Runner Operation traces 的 links。

Incoming W3C trace context 仅在 strict validation 后使用，外部 baggage 丢弃。Shaula 生成 request/Change IDs；audit 不依赖 client trace ID。

HTTP spans 可记录 method、route template、response class、authenticated role category、safe revision 和 stable error code。不得记录 request/response body、Authorization、idempotency key、raw path/query、arbitrary user-agent、credential identity 或 binding values。

Metrics 至少覆盖 bounded request count/duration、admission outcome、precondition/idempotency conflict、active Fleet、Fleet desired/observed lag、Auth desired/observed tuple lag、Change age/state 和 reconcile wake delay。Attributes 使用 finite allowlist；Fleet/Profile key、revision、digest、actor、Target、Change ID、Runner/job、URL、path 和 error text 一律排除。

OTLP failure 不使 committed mutation 失效或阻塞 reconcile。Rate-limited local logs 和 bounded in-process counters 独立报告 degradation。SQLite audit 是 mutation truth；OTel 可 sampled/lost，不能作为唯一 audit。

## 13. Failure behavior

| Failure | Required behavior |
| --- | --- |
| Invalid/unauthorized request | Reject before desired-state mutation or external effect |
| Duplicate idempotency retry | Return original resolved representation/Change；changed replay conflicts |
| Stale ETag | Return `412`；do not merge or overwrite |
| SQLite commit fails | No accepted Revision/Change/effect exists |
| HTTP response lost after commit | Retry resolves through idempotency record |
| In-memory wakeup lost | Outbox/periodic scan eventually observes revision |
| Daemon crashes after commit | Startup resumes Fleet Change/Auth Handoff/Runner Operation from SQLite |
| Client disconnects after commit | Continue asynchronous reconciliation |
| Template key has no Active revision | Reject every new or replacement Fleet reference；an already-admitted Fleet may continue from its retained exact pin without current Active |
| Pinned Template revision/artifact/attestation unavailable or invalid | Block affected Fleet closed and preserve evidence；never substitute a newer revision |
| Auth Candidate validation fails | Keep prior active revision and do not advance dependent desired Auth tuples |
| Auth Handoff cannot prove access/ownership | Keep dependent Fleet quiesced/degraded with explicit desired/observed tuple lag；retain all referenced revisions and never fallback |
| Unbound Fleet or authenticated Scale Set absence during handoff | Record access/absence context only；ordinary Fleet reconciliation exclusively decides create-or-adopt, ID binding and session |
| GitHub `401`/`403`/access-filtered `404` | Publish access Condition；never create on assumed absence |
| One Fleet supervisor retry storm | Circuit-break/degrade that Fleet；bounded scheduler preserves others |
| Busy Runner blocks Decommission | Wait/retry removal；never Destroy early |
| Unknown Runner/Quarantine/missing state | Keep Decommission visibly Blocked；never claim success/purge |
| Shared SQLite/data set unsafe | Mark control plane unready and stop new mutations/effects safely |
| OTLP exporter fails | Continue commit/reconcile with bounded local diagnostics |
| Admission/backlog limit reached | Return `429` with bounded `Retry-After`；do not create unbounded Changes |

## 14. Acceptance criteria

Implementation is incomplete until：

1. 两个使用不同 exact Template Profile Revisions 的 Fleets 通过 HTTP 创建，并在没有 bootstrap resource catalogs 的情况下跨 daemon restart 恢复。
2. Fleet、Template Profile 和 GitHub Auth Profile 都从 HTTP/SQLite 管理；bootstrap 若试图定义这些资源或平台 bindings，validation fail closed。
3. 每个 effective Fleet mutation 原子提交 Revision、Fleet Change、audit、outbox 和 idempotency result；HTTP handler 在 `202` 前不调用 GitHub、Terraform 或平台。
4. Lost response retry 返回原 resolved exact Template Revision；altered replay conflict。并发同 ETag 请求一胜一 `412`，status/Auth Handoff churn 不改变 desired ETag。
5. Bare Template key（若支持）在 admission transaction 中解析为 exact active Revision/digest；新 Template publication 不改变已接受 Fleet 或 Generation。
6. Fleet 无法提交 platform binding、provider credential 或 executable；conceptual Fleet/Runner schema 无平台 object columns。
7. Same-Profile promotion 和 zero-occupancy cross-Profile replacement 都写入完整 desired Auth Revision Ref，经过同一 quiesce/read-only proof/context handoff 后才推进完整 observed tuple；handoff 不 create/adopt、不写 ID、不建 session。
8. Handoff crash 从 SQLite 恢复；失败 Fleet 暴露完整 tuple lag、保持 quiesced/degraded，且 `Blocked` 不释放任一 desired/observed/in-flight/Decommission/recovery reference。
9. PAT/App private key 与 sensitive Template bindings 以 plaintext SQLite bytes 跨重启可用，但任何 Fleet/Profile GET、status、revision、attestation、audit、error、log、trace、metric 或 diagnostic 均不含原文或可推导表示；各自只进入 GitHub Access Module 或 exact-Revision IaC child，绝不进入 Runner/workflow。
10. Template artifact publication 只有 `template.publish` 可执行，conformance attestation 只有 `template.attest` 可提交；Fleet 只能选择由 manifest 派生 platform 且已有 accepted attestation 的 current Active Revision。
11. Dynamic capacity replacement 只产生 Create/Destroy intents，不调用 Runner 或 Scale Set Update。Template 或 Auth Profile replacement 在 occupancy/active operations 非零时拒绝；Target/Scale Set identity 保持不可变。
12. DELETE/Create race 使用 mutation fence：commit 后不 spawn 新 Create-capable subprocess；uncertain older Create 进入 cleanup。
13. DELETE 永久停止 acquisition 但允许 cleanup-only Auth Handoff，等待 `JobStillRunning`，只 Destroy known owned Generations，保留 Scale Set 并写 tombstone；unknown/Quarantine/Auth handoff failure 保持 Blocked。
14. Lost wakeup、crash after commit 和 due `Blocked` Change 都从 periodic scan/lease recovery 前进。
15. 一个 blocked/retrying Fleet 不影响另一个 healthy Fleet 的 HTTP management、session 或 lifecycle；shared unsafe storage 才使 global readiness false。
16. Restart 只需 SQLite、Template artifacts、per-runner Workspaces/state 和 configured engine policy，即可重建 supervisors、Changes、Auth Handoffs 与 Operations；不依赖 mutable source directory 或 native platform inspection。
17. v1 non-loopback bind 始终在 startup fail closed；loopback listener 可由 TLS/authenticating reverse proxy 暴露，且 forged caller identity header 或无 trusted context 的 direct loopback request 不能成为 actor。生产路径没有 native inbound HTTP TLS/mTLS serving 或 OIDC client-auth verification path；GitHub/OTLP 等 outbound TLS 不受此限制。
18. In-memory OTel tests 覆盖 HTTP、commit、async links、Auth Handoff、reconcile 和 exporter outage；credential/request body 不泄漏，metric series 数量有显式上界。
19. Continuously changing Assigned Demand 不让 Fleet Change 永久 open；Change 记录明确 demand checkpoint，后续 demand 触发新 reconcile。
20. Organization/repository Target canonical round-trip；unknown kind、malformed name、arbitrary URL 和 Auth allowlist 外 Target 都 fail admission。

## 15. Open decisions

1. Trusted reverse proxy 使用哪种 actor assertion 与 proxy-to-Shaula backend authentication format？
2. Fleet DELETE 是否一直保留空 Scale Set，还是只删除 fully drained 且明确由 Shaula 创建的 Scale Set？当前 contract 保留。
3. Bare Template Profile key convenience 是否保留，还是要求 client 始终显式提交 revision？两种方式都必须持久化 exact pin。
4. Fleet/Profile Changes、idempotency、audit、tombstones、eligible unreferenced Auth revisions、artifacts、attestations 和 Workspaces 的 retention 时限是什么？
5. Active Fleet、pending Change、HTTP body/rate、artifact size/expansion 的 hard limits 是什么？
6. Binding schema 用哪个 exact annotation 标记 sensitive field，mixed binding 的 GET/revision presence-only representation 如何标准化，`bindings_digest` 的 non-verifier opaque commitment 采用什么构造与编码？
