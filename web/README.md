# Shaula Web UI

React + TypeScript, Vite 8 (Oxc transforms/minification and React Refresh),
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

Vite listens on loopback and proxies `/api`, `/livez`, and `/readyz` to
`http://127.0.0.1:8080`. Set `SHAULA_API_TARGET` in `web/.env.local` to use a different
backend (or an existing authenticating proxy). For an isolated local daemon,
development-only actor injection is available:

```dotenv
SHAULA_API_TARGET=http://127.0.0.1:8080
SHAULA_DEV_BACKEND_TOKEN=replace-with-the-local-daemon-backend-token
SHAULA_DEV_ACTOR=web-developer
SHAULA_DEV_SCOPES=fleet.read,template.read,auth.read
```

These variables are read only by the Vite server. Do not prefix them with `VITE_`,
which exposes values to browser JavaScript. Grant additional scopes explicitly
when testing mutations. Never expose this development server on a public interface.

## HTTP and authentication

Open `/` on the daemon/proxy. `/fleets`, `/fleets/{key}`, `/templates`, `/auth`,
and `/changes` support direct navigation and reload. Missing asset/API routes
return 404, unsupported UI methods return 405, and HEAD returns no body. Hashed
assets are immutable-cacheable; the HTML shell is revalidated. The shell is public
and contains no data or credentials. CSP restricts requests and scripts to same origin.

All management calls, including `GET /api/v1/session`, still require the trusted
actor context in ADR 0011. Deploy behind the same authenticating reverse proxy as
the API. The browser uses same-origin cookies/requests, never backend tokens or
identity headers. A direct unauthenticated visit displays the shell and a 401
state. `/api/v1/session` returns only `{name, scopes}` with `Cache-Control: no-store`.

The UI supports Fleet list/search/detail, create/replace/retire, template archive
upload/publication/revisions/retirement, auth lookup/create/rotation/retirement,
and lookup/polling of accepted changes. Auth and change APIs currently provide
lookup by key/ID, not collection listing. Template attestation submission remains
an API workflow. No demo data is used in the application.

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
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
cargo nextest run --manifest-path Cargo.toml --workspace
```

Playwright uses intercepted API fixtures only inside tests. It covers desktop and
mobile layouts, deep links, authorization errors, conditional edits, uncertain
write retries, profile lookup and retirement. Rust tests exercise the real router
and embedded files, including preservation of API authorization.
