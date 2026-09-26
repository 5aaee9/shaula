# DX-30: isolated Hyper-V / Forgejo / Docker

**Passed:** all four DX-30 scenarios completed in one run on 2026-09-26 UTC.
The [actual report](report.json) records the binary/artifact digests and observed
fault counts: one image-read failure, one Declare failure and one resource
DELETE failure. No registration-DELETE failure is claimed.

| Scenario | Paired evidence and result |
| --- | --- |
| no-create | [JSON](no-create.json), [UI](no-create.png): real plan exit 1, no apply command or container Create/Destroy, `Destroyed`, zero occupancy, `never_started`. |
| waiting-online | [Fault JSON](waiting-online.json), [UI](waiting-online.png): original offline registration/container retained, `WaitingOnline`, occupancy 1. [Recovery](waiting-online-recovered.json) / [UI](waiting-online-recovered.png): same resource executes a job and is reclaimed with `provider_cleanup`. |
| destroy-failure | [Fault JSON](destroy-failure.json), [UI](destroy-failure.png): failed actual apply, durable expiry, `hard_lifetime`, apply uncertainty and occupancy 1 during retry. [Recovery](destroy-recovered.json) / [UI](destroy-recovered.png): daemon restart and restored transport converge to both sides absent, zero occupancy, `provider_cleanup`. |
| rollout-lag | [Fault JSON](rollout-lag.json), [UI](rollout-lag.png): Active template revision 2, old Generation/pin 1, occupancy 1, `rollout.waiting_zero_occupancy`. [Recovery](rollout-recovered.json) / [UI](rollout-recovered.png): legitimate task completion/reclaim permits pin 2 and clears the barrier. |

The final private run directory is `/tmp/shaula-lifecycle-1fed0db6-2Heu8Q`
inside the test VM. After assertions and teardown, Docker had **zero containers
and zero volumes**, all four domain Generations were `Destroyed`, no Shaula
process remained, and SQLite `PRAGMA quick_check` returned `ok`. The raw directory
is mode 0700 and its database 0600. The VM is retained powered off for inspection.
See [VM state](vm.json), [tested source hashes](tested-source-sha256.txt) and
[evidence checksums](SHA256SUMS).

## Environment

The user selected a local Hyper-V VM for fault injection. The dedicated VM is
`shaula-dx30-20260926`, Generation 2, with 8 vCPU, 16 GiB fixed RAM, a 100 GiB
dynamic VHDX, Secure Boot using the Microsoft UEFI CA, and the existing Default
Switch. Automatic startup and automatic checkpoints are disabled. No existing
VM, production Shaula instance or host container engine was used.

| Component | Observed version / identity |
| --- | --- |
| Guest | Ubuntu 24.04.5 LTS, kernel `6.17.0-1022-azure` |
| Docker Engine | 29.1.3, dedicated guest daemon |
| Forgejo | `16.0.4+gitea-1.22.0` |
| Forgejo server image | `codeberg.org/forgejo/forgejo@sha256:a3e33d03e771d3e58b27de5573c3a25dc4f670583a6724c1878a6d0bbecf3556` |
| Official Runner | `13.1.0@sha256:c4af85fd9f0dd03788676a534781a87c71aa2c6a37737143e017eb94d4312952` |
| Terraform / Docker provider | 1.9.8 / 3.0.2, original readonly provider lock |
| Build tools | Rust 1.98.1, Node 22.22.0 |
| Shaula source | `0c3b97f` plus the diagnostics projection/UI corrections in this evidence change |

The guest disk came from [Ubuntu's dated official cloud image](https://cloud-images.ubuntu.com/noble/20260911/noble-server-cloudimg-amd64-azure.vhd.tar.gz).
Its downloaded archive SHA-256 was verified as
`bcf5f2e60e55b3eb0eb57201fd57c0e34ced8b2dd1cdf4718084f41854786195`.
Before first boot, its datasource was changed from Azure to NoCloud and a local
seed supplied the dedicated SSH public key, DHCP configuration and guest
dependencies. The sparse attribute of the extracted VHD was cleared before
Hyper-V conversion. No custom Runner image or Terraform template was used.

Only this guest's Docker service permits minimum API 1.24, matching the existing
CI compatibility requirement for provider 3.0.2. Forgejo binds to the guest's
Docker bridge address; Shaula and the HTTPS browser relay bind to loopback.
All repositories, credentials, registrations, databases and containers are
disposable fixture resources. Browser requests complete authorization-code/PKCE
login and use real Shaula session cookies; API probes use validated OIDC tokens.

## Reproduction and evidence boundary

Build the current checkout in the isolated Linux guest, then follow the
[DX-30 harness instructions](../../../scripts/forgejo-lifecycle/README.md#dx-30-diagnostics-acceptance):

```sh
npm ci --prefix web
npm run build --prefix web
cargo build --locked -p shaula
node web/node_modules/playwright/cli.js install --with-deps chromium
node --test scripts/forgejo-lifecycle/*.test.mjs
SHAULA_DX30_DISPOSABLE_VM=1 \
  DOCKER_HOST=unix:///var/run/docker.sock \
  TERRAFORM_BIN=/absolute/path/to/terraform \
  node scripts/forgejo-lifecycle/diagnostics.mjs
```

The real `shaula serve` composition owns admission, registration, Terraform,
bootstrap, readiness, cleanup and follow. Fault proxies alter only image reads,
initial Declare and DELETE responses; they never manufacture resource or
diagnostic responses, mutate domain rows, or intercept task acquisition.
Each receipt pairs actual UI text/screenshot and HTTP diagnostics with a
read-only SQLite domain snapshot, capacity and provider inventory.

Raw fixture directories retain credential-grade SQLite, plans, state, private
keys and logs under owner-only permissions. They are not repository artifacts.
Only bounded JSON receipts and cropped Why-panel screenshots are publishable.
An earlier failed run or forced fixture teardown never counts as successful
controller cleanup. This acceptance covers the stated Forgejo/Docker tuple;
Hyper-V hosts the test VM and is not the Runner resource backend under test.
It does not establish real GitHub, Kubernetes, cloud/VM Runner platforms, or
busy-safe ordinary drain.

## Findings corrected during acceptance

- The API already distinguished `never_started`, `provider_cleanup` and
  `operator_attested`, but the Why panel omitted completion provenance. The UI
  now renders that distinction and treats unknown future sources conservatively.
- With process-local diagnostic capture unavailable after restart, the domain
  projection retained hard expiry intent but omitted `cleanupMode`. The reader
  now carries `hard_lifetime` at both resource and registration checkpoints.
  A real SQLite test failed before the fix and passes afterward with the
  optional projection table absent.
- Harness assertions were corrected to use failed **plan invocations** before
  Create admission, and failed **apply invocations** plus retained uncertainty
  during Destroy. `ApplyStarting` must not be rewritten as proof of no effects.
  Browser probes use the real OIDC session flow; Bearer authentication is
  intentionally restricted to API and health routes.
- Forgejo can remove an ephemeral registration when the one-job process exits
  during resource destruction. DX-30 therefore verifies actual disappearance
  of both resource and registration, not a mandatory management DELETE call.
  A separate registration-DELETE failure is not claimed by this run.

## Local regression checks

- `cargo fmt --all -- --check` and strict workspace/all-target Clippy passed.
- Full unfiltered workspace nextest: **1,005 passed, 2 skipped**.
- The separately requested `cargo nextest run --manifest-path Cargo.toml
  --workspace test`: **770 passed, 237 skipped** (the trailing word is a filter).
- Web lint, format check, production build and **9 diagnostics browser tests**
  passed, including all completion-source variants and the unknown fallback.
- **6** Linux harness fault/redaction tests passed.
- Both changed Rust modules remain below 400 lines (304 and 337).

These checks are separate from the real-platform receipts.
