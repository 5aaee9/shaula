# Exec lifecycle workers and HTTP state

This is the deployment and recovery guide for the implementation of
[spec 0042](specs/0042-production-lifecycle-worker-integration.md).
The [implementation status](IMPLEMENTATION_STATUS.md) records the acceptance
boundary. A compiled binary or a successful adapter test is not platform acceptance.

## Supported execution host

The first executor requires Linux, cgroup v2 delegation, and `cgroup.kill`.
The daemon must itself run inside the delegated subtree. Each Generation gets
one child cgroup containing `shaula job`, Terraform, providers and bootstrap
descendants. A process group alone is insufficient. Windows and hosts without
this containment fail startup; the remote CLI can still run there.

The NixOS module sets `Delegate=yes`, keeps systemd control-group termination,
and allows the service to manage its delegated cgroups. Outside NixOS, provide
equivalent service containment. Do not run the daemon at the cgroup root or
point it at another service's delegation. `lifecycle.cgroup_root` is an optional
absolute override for a deliberately delegated test/service subtree.

Fresh bootstrap configurations must explicitly include:

```yaml
lifecycle:
  executor: exec
  internal_listen: "127.0.0.1:0"
  max_workers: 64
  recovery_reserve: 8
```

These worker counts are examples to size for the deployment, not tested capacity
claims or defaults. Require `0 < recovery_reserve < max_workers <= 1024`.
New Generation admission cannot consume the recovery reserve. Long-lived waiting
workers count toward this limit but do not hold Terraform mutation permits.
The internal listener is separate from management OIDC/PAT routes, only binds
loopback, and must not be exposed through a proxy. Control and state capabilities
are distinct; no CI management credentials or SQLite connection enter the worker.

## Budgets implemented by this revision

| Resource | Value / behavior |
| --- | --- |
| Worker admission | Explicit maximum and recovery reserve; hard maximum 1,024 |
| Terraform create/destroy concurrency | Execution settings, default 8 each, range 1–1,024 |
| Operation timeout | Execution setting, default 1,800 seconds, range 1–86,400; also enforced at Worker handoff |
| Worker runtime | Current-thread Tokio, at most two blocking threads |
| Launch/control envelope | 64 KiB |
| Protected material | Separate authenticated transfer, at most 16 MiB |
| State / lock bodies | 16 MiB / 16 KiB |
| Internal state / control / log permits | Separate 16 / 16 / 8 pools |
| Internal request timeout | 30 seconds |
| Worker control client | 5-second connect, 75-second request; three attempts with the same request ID/body, 200 ms between retries |
| Log request | 512 KiB, 5-second timeout; does not authorize effects |
| Launch pipe handoff | 5 seconds; partial handoff is uncertain, never blindly retried |
| Supervision / idle control poll | 1 second / 200 ms |
| Completion replay cache | At most twice the configured worker count; 60-second retention; durable receipts survive restart |
| Effect / log replay entries | At most 256 / 128 per session |
| Worker shutdown grace / tree fence | 5 seconds / 2 seconds per tree, in parallel |
| Private listener drain | At most 75 seconds after worker stop; systemd stop timeout 90 seconds |
| Reaper inventory | 8,192 entries, depth 32; unknown or emergency files retain the workspace |

These are implementation bounds, not completed load acceptance. In particular,
waiting-worker RSS, maximum-count SQLite pressure and log/state isolation under
saturation still require LW-30 evidence before a production capacity is promised.

## Upgrade and explicit offline classification

1. Stop the original controller and fence every old worker/provider/bootstrap
   descendant. Record the original service containment and host/boot identity.
   A missing PID, token revocation or Terraform lock is not a fence.
2. Preserve an offline consistent backup of the entire data directory, SQLite
   database and any WAL/SHM, immutable artifacts, original workspaces, inputs,
   local/backup/emergency state, bootstrap configuration, exact engine/provider
   tuple and external bindings key. Keep credential-grade backups private.
3. Add the explicit lifecycle limits and install the new binary/module. The
   schema migration is forward-only. Starting with unclassified legacy
   Generations permits authenticated management reads but rejects mutations,
   acquisition and new claims with `MigrationRequired`.
4. Stop that process before maintenance. The maintenance command acquires the
   same data-directory ownership lock as `serve`; it does not initialize OIDC,
   register runners, execute Terraform or mutate infrastructure.
5. Inspect into a new private plan file, review every classification, then apply
   exactly that plan with the same configuration and fence evidence:

   ```sh
   shaula maintenance lifecycle-state inspect --config /private/bootstrap.json \
     --plan /private/lifecycle-plan.json \
     --legacy-cgroup /sys/fs/cgroup/original-service-subtree
   shaula maintenance lifecycle-state apply --config /private/bootstrap.json \
     --plan /private/lifecycle-plan.json \
     --legacy-cgroup /sys/fs/cgroup/original-service-subtree
   ```

   The supplied original cgroup must still exist and be verifiably empty on the
   same host/boot at inspect and apply. If this evidence is unavailable, omit the
   option: entries are conservatively quarantined, not imported as empty state.
   Never substitute an unrelated empty cgroup. The operator remains responsible
   for establishing that this was the original execution containment.
6. Apply rejects changed database provenance, CI identity, state/input/artifact
   contents, engine or fence. It rechecks source files before each short atomic
   import. Each Generation gets a content-bound receipt, so repeating an
   interrupted apply resumes committed entries without overwriting them. Use a
   new inspect file after a genuine stale-plan rejection. Activation occurs only
   once every legacy Generation is classified.
7. Restart `serve`, check authenticated health/readiness and diagnostics, and
   verify provider and CI inventories before increasing capacity. Imported
   Generations recover using the original exact materials in cleanup-only mode.
   Quarantine preserves occupancy and source evidence; it is not proof of cleanup.

Automatic import requires trustworthy primary state, consistent backup lineage,
the daemon's retained successful Create lineage/serial, no emergency state,
original immutable input/artifact and compatible engine provenance. The source
must match that original lineage and cannot roll back its serial. An empty source
inherits completion proof only when the original resource-destruction fact also
exists; otherwise it requires delete-only verification. Unknown or incompatible entries stay quarantined. Old source files
remain untouched. A new Generation alone receives a fresh empty HTTP snapshot;
missing state in an old Generation never enables another Create.

## Recovery, completion and rollback

On restart, the daemon fences the recorded old containment before issuing a new
epoch. Unverifiable ownership, missing state/material or emergency files retain
uncertainty and occupancy. A later successful fence does not itself release a
quarantine: resources still need the existing audited operator resolution.
Do not manually clear locks, remove evidence or reset lifecycle tables to unblock
admission. A sealed state, terminal fact, completion receipt and capacity release
commit together. Ordinary workspace reaping requires that durable receipt and a
proven stopped tree; unknown files and emergency state are retained. Cleanup
inventory is limited to 8,192 entries and depth 32. Transient cleanup/storage
errors are reported without secret-bearing details and retried at most three
times during the receipt replay window; unresolved work is reconsidered on restart.

A stopped local process is not proof that its remote Create effects are fully
represented in state. Interrupted Create without the retained successful result
is quarantined even when a partial state exists. Neither command termination nor
deleting the known subset grants permission to release that Generation's capacity.

A normal service stop interrupts local execution after its grace period; it does
not destroy all runners. State/control remain available until worker fencing and
drain finish. Restart may require cleanup or operator reconciliation after a
provider operation was interrupted. Cloud-side asynchronous effects can outlive
the local process tree and must be checked independently.

After activation, returning this data directory to an old binary or local-state
runtime is unsupported. The format marker is enforced by the new software; it
cannot constrain historical binaries that do not understand it. Prefer a forward
fix. Restoring a backup requires fencing all writers first, preserving any newer
emergency/state evidence and reconciling external effects after the checkpoint.
Never overwrite newer authority with an old DB snapshot or run two controllers
against copies referring to the same resources. A package switch alone is not a
safe rollback. Before activation, restoration still requires the entire consistent
backup and proof that no post-checkpoint effects are being discarded.

## Reproducible tests

`scripts/lifecycle-worker-tests.sh` runs the Linux-only real child-process and
pinned Terraform HTTP/SQLite tests. Supply `SHAULA_TEST_CGROUP` and run the test
parent inside that writable delegation; the script obtains Terraform from PATH.
`scripts/lifecycle-acceptance.sh` additionally runs the existing real
Forgejo lifecycle harness for Docker or an explicitly supplied disposable Kind
context. The GitHub Actions matrix provisions both platforms and creates a
dedicated delegated systemd service. A configured job alone does not establish
GitHub or Kubernetes acceptance, all resource platforms, backup/restore fault
coverage or the full LW-01–LW-32 matrix.
