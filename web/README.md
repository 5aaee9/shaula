# Shaula Web UI

React + TypeScript, Vite 8 (Oxc transforms/minification),
Tailwind CSS 4 and official shadcn/ui Radix components, downloaded with the CLI.
Fonts are bundled locally.
Oxc also provides linting (`oxlint`) and formatting (`oxfmt`).
The bitmap brand mark is rendered from Lucide's ISC-licensed Workflow icon.

## UI components

`components.json` selects the official New York style. `src/components/ui/`
contains CLI-downloaded components, composed by the resource pages. The shared
`Modal`, `Field` and `Tip` helpers compose those components for repeated domain
workflows. The application shell is adapted from the official
[dashboard-01 block](https://ui.shadcn.com/blocks#dashboard-01), installed with
`npx shadcn@latest add dashboard-01`. `AppSidebar`, `NavMain`,
`NavUser`, `SiteHeader`, and `SectionCards` retain the block composition and bind
it to Shaula routes, session permissions, and live capacity. The theme uses the
official neutral tokens. Custom CSS is limited to resource tables, details, and
form layouts. Unused demo charts, sample data, document menus, and their dependencies
are removed; the API has no historical capacity series to plot.

```sh
npx shadcn@latest search '@shadcn' --limit 65
npx shadcn@latest view @shadcn/table @shadcn/dialog @shadcn/field
npx shadcn@latest add button input textarea dialog tooltip table tabs badge alert field native-select empty skeleton sheet avatar
```

Tables use Table, status labels use Badge, forms use Field/Input/NativeSelect,
resource editors use Dialog, and mobile navigation uses Sheet. Loading and empty
views use Skeleton and Empty. Dependencies such as Label and Separator are
resolved by the CLI.

## Build the binary

Prerequisites: Node.js 22.12+ (24 LTS recommended), npm, and the Rust toolchain.
From the repository root:

```sh
npm ci --prefix web
npm run build --prefix web
cargo build --release --locked
```

The HTTP crate's build script invokes the installed Vite CLI and watches frontend
source/configuration changes. Missing dependencies or a failed frontend build fail
the Cargo build, rather than embedding a stale UI. Cargo does not install npm
dependencies or access the npm registry. `NODE` can select a Node executable.

Both debug and release binaries embed `web/dist` using `rust-embed` with
`debug-embed`. Deployment needs only the binary and its normal bootstrap/runtime
dependencies, not Node.js or `web/dist`. Build output and `node_modules` are ignored.

## Development

```sh
npm run dev --prefix web
```

Vite listens on loopback and proxies `/api`, `/auth/oidc/`, `/livez`, and `/readyz`
to `http://127.0.0.1:8080`. Set `SHAULA_API_TARGET` in `web/.env.local` to change
the backend:

```dotenv
SHAULA_API_TARGET=http://127.0.0.1:8080
```

Serve Vite through a local HTTPS reverse proxy and register that exact HTTPS
origin/callback with the Provider. Set the daemon's `SHAULA_OIDC_PUBLIC_URL` to
the same origin. Every document and source asset is checked against the backend
session before Vite serves it. Dev and preview have no actor injection or
anonymous mode. HMR/WebSocket transport is disabled; refresh after source edits.
Never expose the development server on a public interface or put secrets in
`VITE_` variables. See [OIDC deployment](../docs/oidc-deployment.md).

## HTTP and authentication

Open `/` on the daemon/proxy. `/fleets`, `/fleets/{key}`, `/templates`, `/auth`,
and `/changes` support direct navigation and reload. Missing asset/API routes
return 404 after authentication, unsupported UI methods return 405, and HEAD
returns no body. All responses, including embedded assets, are private/no-store.
CSP restricts requests and scripts to the same origin.

All UI, assets, API and health routes require OIDC under ADR 0013. An anonymous
document visit redirects to login without returning HTML; anonymous assets/API
return 401. The browser receives an opaque Secure/HttpOnly session cookie, never
OIDC tokens or identity headers. `/api/v1/session` returns `{name, scopes}` plus
an `X-CSRF-Token` header. Mutations and logout include that token. A 401 clears
query caches and credential forms and presents explicit sign-in; failed writes
are never replayed after login.

The UI supports Fleet list/search/detail, create/replace/retire, template archive
upload/publication/revisions/retirement, auth list/search/detail/create/rotation/retirement,
and lookup/polling of accepted changes. The authentication page lists existing
connections with their active target policy and revision status; selecting a row
opens its details. Change lookup still requires an ID. Template attestation
submission remains an API workflow. No demo data is used in the application.

Edits capture the ETag when the dialog opens. Creates use `If-None-Match: *`;
replace/retire uses `If-Match`. Retrying an unchanged uncertain mutation reuses
its idempotency key. Secret fields exist only in transient form state and are
never stored in browser storage. `202 Accepted` is shown as a pending change,
not as completed runtime work. Existing daemon implementation gaps still apply.

## Checks

```sh
npm run build --prefix web
npm run lint --prefix web
npm run fmt:check --prefix web
npm exec --prefix web -- playwright install chromium
npm test --prefix web
npm run test:oidc --prefix web
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
cargo nextest run --manifest-path Cargo.toml --workspace
```

Presentation tests use an isolated test server and intercepted API fixtures.
They cover desktop/mobile layouts, conditional edits, uncertain write retries,
session expiry, CSRF and logout. The separate OIDC suite starts a real Shaula
binary, HTTPS proxy and local test Provider to verify login and protected
embedded files. Production/dev configuration never enables that fixture.
