# Forgejo busy-safe drain: release blocker

Status: **unresolved**, 2026-09-12. The owner selected the full spec 0026 contract,
not a conservative-preview release. This note records source evidence and a
proposed prerequisite; it does not claim an implemented or accepted upstream
extension. [Implementation status](IMPLEMENTATION_STATUS.md#forgejo-runner-backend-in-progress-pool-implementation-2026-09-11)
remains authoritative for delivery progress.

## Why the current protocol cannot authorize idle expiry

Pinned Forgejo: `e27d0384e5e2d143843cee3d7741506c846f9229`.
Pinned runner: `667c8d975b9255e7bf32164e012146f68f0022c6`.

- [The scoped HTTP routes](https://code.forgejo.org/forgejo/forgejo/src/commit/e27d0384e5e2d143843cee3d7741506c846f9229/routers/api/v1/api.go)
  expose registration, inventory, exact-ID lookup, jobs and DELETE. They do not
  expose a pause, acquisition fence, or busy-conditional deletion.
- [DELETE](https://code.forgejo.org/forgejo/forgejo/src/commit/e27d0384e5e2d143843cee3d7741506c846f9229/routers/api/v1/shared/runners.go)
  checks visibility/editability, then deletes the registration without an atomic
  task-ownership check.
- [The runner's single-task poller](https://code.forgejo.org/forgejo/runner/src/commit/667c8d975b9255e7bf32164e012146f68f0022c6/internal/app/poll/single.go)
  derives polling and task contexts from the same parent. Cancellation can stop
  an already assigned task. `Shutdown` is not an externally callable idle-only
  drain interface.
- [The one-job command](https://code.forgejo.org/forgejo/runner/src/commit/667c8d975b9255e7bf32164e012146f68f0022c6/internal/app/cmd/job.go)
  calls `Poll` before `Shutdown`; [its flags](https://code.forgejo.org/forgejo/runner/src/commit/667c8d975b9255e7bf32164e012146f68f0022c6/internal/app/cmd/cmd.go)
  expose `--wait`, not an idle-only deadline/drain acknowledgement.
- [FetchTask](https://code.forgejo.org/forgejo/forgejo/src/commit/e27d0384e5e2d143843cee3d7741506c846f9229/routers/api/actions/runner/runner.go)
  can assign a task even when its response is lost. The runner explicitly retries
  the same request key to recover such assignments. A disconnected or cancelled
  fetch is therefore not evidence that no task was assigned.

Counterexample to "observe idle, then stop":

1. Shaula reads `idle`.
2. An already issued FetchTask assigns a task; its response is delayed.
3. Shaula signals the process or deletes its registration.
4. A real task loses its execution identity or is cancelled.

Repeating step 1, sleeping for a grace period, checking Pod readiness, parsing
logs, pausing a container, or blocking new network connections does not close
the in-flight assignment race. Replacing labels via Declare is also not a
transactional fence against an already authorized FetchTask.

## Proposed prerequisite: an upstream Runner Acquisition Fence

The preferred seam is the Forgejo control-plane module, where task assignment
can be serialized with a durable fence for an exact scoped runner ID + UUID.
This is a proposed behavior contract, **not an endpoint that exists today**.

A small `drain(exact_identity, idempotency_key)` interface should hide the
assignment/retry implementation and return one of:

- **FencedIdle**: no task is held and no in-flight or future acquisition can
  assign one to this identity. A durable fence reference identifies the proof.
- **Busy**: an assignment exists or its outcome remains unresolved. New
  acquisitions are fenced, but the existing task keeps all execution/reporting
  authority; no cancellation or credential revocation occurs.
- **Absent**: exact identity is absent after independently establishing scope
  access. This is registration evidence, not resource-destruction evidence.
- **Unknown**: no deletion authority; preserve registration and occupancy.

Required upstream properties:

1. Fence and assignment serialize in the same authoritative transaction/lock
   protocol. Checking a flag outside the assignment transaction is insufficient.
2. All acquisition paths, including request-key recovery and FetchSingleTask,
   participate. A lost response cannot turn an assigned task into FencedIdle.
3. The fence survives both server and daemon restarts. Repeating the same drain
   request is idempotent, and runner reconnect/Declare cannot remove it.
4. In-flight task updates, logs, artifacts and completion remain functional.
   A fence is not revocation of a busy runner's token.
5. Results bind scope identity, runner ID, UUID and the fence reference. A
   same-name replacement cannot inherit deletion authority.

Once an upstream implementation is available, Shaula can bind the fence proof
into its durable cleanup intent, revalidate the Fleet mutation fence and exact
resource identity, and allow the local runtime to stop an idle runner before
DELETE/Destroy. The existing `forgejo_template_evidence` default must remain
`Unknown` until a production adapter can satisfy the entire proof, not just
report a process snapshot. Do not implement a hypothetical successful adapter.

An upstream runner-only drain could be an alternative, but it must separately
prove resolution of every outstanding FetchTask request key without cancelling
assigned work. Merely separating cancellation contexts does not settle a fetch
whose server-side outcome is unknown. A host-owned protocol proxy would add a
new credential-bearing execution path and requires a separate reviewed design;
it is not a transparent implementation of the current official-image contract.

## Acceptance before unblocking release

Run against the supported real Forgejo/official-runner/image tuples, not only
fakes:

- Drain races with assignment immediately before, during and after the fence.
  Either Busy preserves the job, or FencedIdle prevents any later assignment.
- Lose the fetch response and the drain response independently; repeat and
  restart both sides. No duplicate assignment, false idle proof or blind POST.
- Drain a busy runner; the job completes normally and its ephemeral registration
  disappears before resource cleanup.
- Drain an idle runner; cleanup meets the configured deadline without waiting
  for a sacrificial workflow job.
- Exercise permission loss, 404-hidden targets, scope/name reuse, stale Fleet
  fences, contradictory evidence and replacement container/Pod identities.
- Record registration and resource terminal evidence separately, retain
  occupancy on Unknown, and verify that all credential leak surfaces remain
  clean. Finish the rest of spec 0026 A1–A8 as well.

**Unblocking action:** supply an upstream fence/drain implementation with the
above evidence (and an admitted official image if the runner changes), or
explicitly authorize a different protocol design. No upstream change has been
submitted or tested by this worktree.
