# Non-waiting one-job experiment

Runs official Forgejo **16.0.4** and runner **13.1.0** in disposable containers.
It verifies the upstream protocol, **not** Shaula Pool/Terraform integration or
the full Forgejo A1–A8 acceptance contract. Production bootstrap is unchanged.

Requirements: Linux, Node.js >= 22, a **local** Docker or Podman engine, and the
two official images. The containers use host networking so the test-only fault
injector can bind loopback. Forgejo binds a dynamically selected loopback port;
no existing Forgejo URL, account or registration is used. The only workflows
are private, generated `:host` jobs containing `echo` and `sleep` (no DinD).

```sh
podman pull codeberg.org/forgejo/forgejo:16.0.4
podman pull code.forgejo.org/forgejo/runner:13.1.0
node --test scripts/forgejo-one-job/fault.test.mjs
CONTAINER_ENGINE=podman node scripts/forgejo-one-job/verify.mjs > results.json
# Alternatively: CONTAINER_ENGINE=docker with a local Docker engine.
```

The report contains actual image IDs/digests and separate registration/resource
observations. Credentials and raw RPC payloads are not emitted. Runner tokens
are copied from files inside a private temporary directory, not supplied in
argv or container metadata. The output is written only after every assertion
and fixture teardown succeeds.

Cases:

1. No matching task: non-waiting runner exits with code 2; ephemeral registration
   remains. The fixture can remove it because it controls the entire queue and
   never published a matching job. **That is not a production deletion proof.**
2. Waiting task: create a runner only after observing demand, preserve it while
   executing, observe natural exit and exact registration absence, then remove
   its container and anonymous volume.
3. Failing task: same lifecycle, but the workflow fails. The runner still exits
   with code 0 and its ephemeral registration disappears; exit 0 is NOT evidence
   of workflow success.
4. Fetch unavailable: return HTTP 503 before forwarding FetchTask. The runner
   exits with code 2 while the task remains waiting.
5. Assignment response lost: forward FetchTask, fully receive Forgejo's response,
   then close the client connection without delivering it. The runner exits
   with code 2 while Forgejo has a running task/retained registration (which can
   still report `idle`).
6. `--wait` control: lose the same first response, then allow retries. The runner
   recovers the assignment and completes normally.

The loopback HTTP relay is **fault-injection test equipment only**. It is not a
production task proxy, does not implement drain, and is never wired into Shaula.
Uncertain cases are not authorized for reclamation: their registrations and
containers are retained until final teardown of the disposable experiment.
Cleanup removes only exact container IDs created by this run; it never prunes
the engine or deletes pre-existing resources.

Do not interpret a passing experiment as permission to delete every runner
which exits with code 2. The experiment deliberately demonstrates that this
exit code conflates empty demand with unsuccessful/uncertain acquisition.
