# Embedded Web UI

The operator UI lives in `web/` and is served from `/` by the existing loopback
HTTP listener. It uses React, Vite 8's Oxc toolchain and official shadcn/ui
components downloaded with the CLI and composed into resource views.
The UI is an API client, not a second source of Fleet/Profile desired state.

## Contracts

**Jobs** 导航及 `/jobs`、`/jobs/{id}` 以 GitHub workflow job 为主，显示已观测状态、关联 Runner 和 Apply/Destroy 各次尝试的日志。未关联的预热/创建失败 Runner 有次级排障入口。身份与结论的来源、日志保留与权限、轮询/分页及失败状态由 [spec 0019](0019-workflow-jobs-and-operation-logs.md) 唯一维护；本 UI 不把 Terraform operation 当成 Workflow Job。

The multi-account GitHub authentication form and per-binding status
are specified in [spec 0011 §6](0011-multi-account-github-authentication.md#6-http-and-ui-contract).
Authentication inventory and detail discovery are specified in
[spec 0012](0012-github-authentication-inventory.md). Implementation and verification
status belongs in [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md).

Fleet Template inputs 的可视化选择、exact Revision 和草稿保留目标见
[spec 0014](0014-visual-template-inputs.md)；该增量以批准值控件替换 JSON
textarea，并将全部模板输入（包括可选参数和 presets）直接展示在主表单。

默认模板来源库、从 Terraform 声明发现 variables，以及发布草稿显式采用默认值/批准选项，
遵循 [spec 0015](0015-template-library-and-variable-discovery.md)。来源库不代表 Active Profiles。

Fleet authentication/template Profile 从服务端列表选择、自动加载模板输入、列表失败时
保留原引用的行为见 [spec 0016](0016-fleet-profile-selection.md)。

Template 静态校验通过后自动激活的状态和说明遵循
[spec 0017](0017-automatic-template-activation.md)；详情显示暂时 Ready 的等待说明和
Revision 的静态校验失败原因，不提供额外 Activate 操作。

- Cargo builds and embeds the frontend into debug and release binaries. A failed
  frontend build fails the binary build. Runtime serving requires no frontend files.
- UI document routes support refresh and direct links. Unknown API paths and
  missing static assets remain errors, never successful SPA documents.
- The UI shell, deep links and all embedded assets require an OIDC-derived
  browser session under [spec 0009](0009-mandatory-openid-connect.md) and
  [ADR-0013](../ard/0013-require-openid-connect-for-all-http-access.md). `serve`
  requires Provider/client configuration through clap/env and fails before
  listening when it is missing or invalid. The public shell is removed.
- Unauthenticated UI document GETs redirect to OIDC login before sending HTML.
  API, asset and HEAD requests return 401; unauthorized authenticated API
  requests return 403. Authentication precedes routing fallback and cache checks.
  All responses use private/no-store, including hashed assets; no service worker
  or offline cache may retain the protected UI.
- Rust owns login, callback, token validation and the opaque HttpOnly/Secure
  cookie session. Same-origin browser requests carry no backend token or
  caller-asserted actor; proxy identity injection and development bypasses are
  removed. React never receives OIDC tokens or the client secret.
- `GET /api/v1/session` returns `{name, scopes}` and `X-CSRF-Token` in a response
  header. UI mutations echo the CSRF header; the server validates it together
  with Origin. A session menu supports authenticated, CSRF-protected logout.
  Lease expiry first renews through the Provider in the server guard under
  [spec 0013](0013-provider-backed-browser-session-renewal.md). Successful renewal
  preserves the mounted page, URL and draft state. Terminal 401 or logout clears
  in-memory resource/query and credential form state; authentication failure must
  not replay a mutation. Temporary renewal 503 does not mark authentication expired.
- UI creates, edits and retirement preserve conditional mutation and idempotency
  semantics. An editor keeps the version originally displayed even as background
  queries refresh. Conflicts retain user input and require explicit reload/review.
- Creation dialogs start with the required identity, source/target and credential
  fields. Optional settings other than Template inputs live in a collapsed
  **Advanced settings** section; all Template input controls remain visible.
  Collapsing preserves values and submission semantics. Hidden invalid controls
  expand their section before native validation focuses them; invalid advanced
  JSON also reveals its editor. Existing resource values are never reset by folding.
  New Fleets default the scale set name to their key unless explicitly overridden;
  Runner group `Default`, zero minimum runners and empty labels remain defaults.
  Template inputs start unset, without applying schema defaults; new Fleets capture
  the displayed Active revision for submission. Maximum capacity stays visible. Templates
  expose an archive/existing-digest choice; Terraform, bindings and input policy
  are advanced. Authentication retains a visible first target and required
  credentials; additional targets may fold, but policy-change previews and live
  Fleet impact/errors stay visible when modifying an existing policy. Folding
  does not infer broader authorization or supply unknown template-specific bindings.
- Accepted changes are polled and display their actual state. UI readiness or
  `202` acceptance does not establish runner provisioning or daemon completeness.
- Credentials/bindings are transient write-only form inputs. They are not stored
  in browser storage or included in query URLs, analytics or error reporting.
- Loading, empty, permission-denied, disconnected and conflict states are visible.
  Read/write/retire controls respect the session's scopes; backend authorization
  remains authoritative.

See [web development and deployment](../../web/README.md) for commands and scope.
The README describes development/deployment commands, not a second authentication
contract. Current implementation coverage and outstanding real-Provider acceptance
are recorded only in [implementation status](../IMPLEMENTATION_STATUS.md).
