# Nix development, packaging and NixOS deployment

The root `flake.nix` only declares inputs and imports `nix/flake-module.nix`.
`flake.lock` pins nixpkgs, flake-parts, fenix and treefmt-nix. All package,
development-shell, service-module and VM-test expressions live under `nix/`.

## Outputs and development

Linux outputs are provided for `x86_64-linux` and `aarch64-linux`:

| Output | Purpose |
| --- | --- |
| `packages.<system>.shaula` / `default` | Release executable with the Vite operator UI embedded |
| `packages.<system>.terraform` | Exact Terraform 1.9.8 release binary used by protocol acceptance |
| `devShells.<system>.default` | Fenix Rust, Cargo, Clippy, rustfmt, nextest, Node/npm, Terraform and treefmt |
| `formatter.<system>` | treefmt-nix formatting and static linting |
| `nixosModules.shaula` / `default` | `services.shaula` systemd service |
| `checks.<system>.rust` | Sandboxed package build, Clippy, workspace nextest and real Terraform HTTP-backend probe |
| `checks.<system>.treefmt` | Non-mutating formatting/static-lint gate |
| `checks.<system>.nixos-module` | Module defaults and invalid secret/owned-path configuration checks |
| `checks.<system>.nixos-e2e` | Two-VM deployment acceptance using the release package and service module |

```sh
nix develop
npm ci --prefix web
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --manifest-path Cargo.toml --workspace
nix fmt
```

Entering the development shell does not run npm or alter global configuration.
The shell and Nix package use the same locked fenix stable toolchain. Clippy is
an additional compiled-code gate, not something run once per file by treefmt.
Treefmt covers Rust formatting; Nix formatting/dead-code/static checks; Python
fixture lint/format; web Oxc lint/format; shell lint/format; and GitHub actionlint.
Generated build trees and the external `references/` checkout are excluded.

```sh
nix build .#shaula
./result/bin/shaula version
nix build .#checks.x86_64-linux.treefmt
nix flake check -L
```

The package vendors Cargo dependencies from `Cargo.lock` and installs the exact
`web/package-lock.json` npm tree from a fixed-output cache. TypeScript checking
and the existing `build.rs` Vite embedding run without network access inside the
build sandbox. Node/npm and frontend sources are not needed at runtime. The
source filter excludes local build artifacts, node_modules, references and env
files. Compiler source paths are remapped so panic locations do not retain the
fenix toolchain; the package rejects references to the toolchain and Node.
Integration tests use the debug profile because local-server constructors
are deliberately absent from release builds; the installed executable remains
release-built.

Terraform is **BUSL-1.1**, separate from Shaula's Apache-2.0 license. The flake's
unfree exception is scoped to the `terraform` package, not all unfree software.
The binary ZIP hashes in `nix/packages/terraform.nix` come from the vendor's
HTTPS-published checksum list. The binary is not stripped; its bytes are part of
attestation authority. This is not an endorsement of every provider/version or
an automatic conformance attestation. Updating Terraform requires updating both
platform hashes and renewing the existing pinned protocol acceptance test.

## NixOS service

Add Shaula as a flake input and import its module into your NixOS configuration:

```nix
{ inputs, ... }:
{
  imports = [ inputs.shaula.nixosModules.default ];

  services.shaula = {
    enable = true;
    bindingsKeyFile = "/run/secrets/shaula-bindings-key";
    oidc = {
      provider = "https://identity.example.com/realms/operations";
      clientId = "shaula-web";
      clientSecretFile = "/run/secrets/shaula-oidc-client-secret";
      publicUrl = "https://shaula.example.com";
      apiAudience = "shaula-api";
      # Public CA material may be a Nix path; secret material must not be.
      # caCertificate = ./private-provider-ca.pem;
    };
    settings.http.authorization = [{
      issuer = "https://identity.example.com/realms/operations";
      subject = "exact-operator-subject";
      scopes = [ "fleet.read" "template.read" "auth.read" ];
    }];
  };
}
```

Provision both files before service startup using your secret manager. Use
**string paths outside `/nix/store`**, never `builtins.readFile`, Nix path literals
for secrets, or plaintext `settings.http.bindings_server_key`. The bindings key
must have at least 32 characters, be unique to the deployment, and persist across
restarts/rebuilds/restores. It is not a disposable startup-generated password.

The module loads these files through `LoadCredential`, renders the final
bootstrap JSON privately under `/run/shaula`, and supplies the OIDC secret only
through the process environment. Public configuration in the Nix store has no
secret values. `DynamicUser`, private state/runtime directory permissions,
`UMask=0077` and systemd filesystem/process hardening apply by default.
`stateDirectory` defaults to `shaula`, giving `/var/lib/shaula`; systemd retains
it across service restarts and OS reboots. The module owns `storage.data_dir`
and `execution.engines`; use `stateDirectory` and `terraformPackage` instead.

The package also installs default Docker, Kubernetes and Proxmox template sources under
`share/shaula/templates`. `services.shaula.templateSourceDirectories` defaults to
that package directory; set it to `[]` for an empty default catalog, or provide trusted
absolute source directories whose direct children contain template modules. Outside
NixOS, the equivalent bootstrap setting is `template_source_dirs` (default `[]`).
Missing configured directories and read or validation errors fail startup and preserve
the previous source catalog. An explicit empty directory list clears the default catalog.

At startup, the complete configured source set atomically replaces the SQLite default
catalog after validation. Stable source keys follow the current package; obsolete entries
are removed. This updates the starting points, while published revisions and their archives
remain immutable. Select **Use template** to publish a Profile, or **Update from default**
on a published template to review a new revision while retaining its bindings. Existing
Fleet pins change only through explicit Fleet editing. See [spec 0021](specs/0021-default-template-updates.md).

SQLite now owns complete immutable template archives, including migrated original
archive sidecars. The execution cache is verified against those bytes and can be
reconstructed when missing. Backups must still preserve the complete data directory
and bindings key: Runner ledger/state and credential bindings remain required.
Do not replace a missing legacy archive by repacking its extracted directory, which
would change the existing artifact identity.

The daemon currently handles SIGINT for graceful shutdown. The module sets
`KillSignal=SIGINT`, retains control-group termination for descendants, and uses
a bounded stop timeout. A forced stop is not proof of infrastructure destruction.
Preserve the whole data directory and external bindings key for recovery/backup.
Never start two controllers against copies referring to the same live resources.

No firewall ports, anonymous health exceptions, Docker socket mounts or TLS
proxy are created by this module. Configure HTTPS ingress separately, forwarding
to the default `127.0.0.1:8080` listener. Follow [OIDC deployment](oidc-deployment.md)
for client registration, exact callback, CA trust, permissions and logging rules.
Do not expose an internal state/worker listener through the management proxy.

## NixOS VM acceptance and CI

```sh
nix build .#checks.x86_64-linux.nixos-e2e -L --no-link
```

Use a native Linux builder with working `/dev/kvm` access and `kvm`/`nixos-test`
in its advertised Nix system features. The tests require real VM execution;
missing KVM is a build failure, not a reason to skip the E2E check. Nix handles
all guest dependencies; no host-installed Docker, Kubernetes or OIDC service is
required.

The machine VM runs the packaged release daemon via the exported module, nginx
HTTPS ingress, and an explicitly test-only HTTPS OIDC issuer. A second VM runs
HTTP assertions, including Authorization Code + PKCE and cookie-session flows.
This checks TLS verification, anonymous/JWT/scope denial, embedded asset serving,
CSRF/logout, conditional/idempotent Profile writes, secret redaction, private
state permissions, graceful restart, reboot persistence, session invalidation,
and startup refusal when credentials or the Provider are missing.

The fixture issuer and disposable PKI are confined to the VM tests and are not
installed in the application package or production module. HTTP session tests
are not JavaScript browser rendering tests. This suite does **not** establish
real GitHub, registered OIDC Provider, Kubernetes/Docker Runner conformance, or
the not-yet-integrated `shaula job`/exec lifecycle. Current implementation and
verification evidence belong in [implementation status](IMPLEMENTATION_STATUS.md).

The [GitHub workflow](../.github/workflows/nix.yml) runs the same flake checks on a
KVM-enabled x86_64 Linux runner, with read-only repository permissions and pinned
third-party actions. Hosted private-repo runners only guarantee a small root
disk, so before installing Nix the workflow moves the store onto the larger
resource disk when one is attached, deletes unused preinstalled toolchains, and
points `build-dir` at `/nix/build`; VM build scratch therefore shares the same
device as the store. It uses
ordinary `pull_request` events, not privileged `pull_request_target` execution,
and needs no production credentials. Remote CI success must be observed on an
actual pushed workflow run; local Nix checks alone are not that evidence.

## Official container bootstrap tools

The NixOS service wrapper includes `docker-client` and `kubectl` in its runtime
PATH for the fixed host bootstrap in [spec 0020](specs/0020-official-container-runner-bootstrap.md).
The daemon resolves those fixed executable names before launching children with
a minimal environment; Fleet input cannot select executables or arguments.
Docker/kubectl availability alone does not grant access: configure the approved
Docker socket or Kubernetes bindings and namespace permissions through the usual
Profile/deployment path. No tools or platform credentials are injected into the
Runner container. Standalone package deployments must provide the corresponding
host CLI dependencies themselves.
