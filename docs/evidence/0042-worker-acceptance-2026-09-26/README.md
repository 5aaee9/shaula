# Production Worker acceptance, 2026-09-26

These receipts exercise actual `serve -> job -> Terraform -> Docker/Forgejo`
with SQLite HTTP state on disposable GitHub-hosted Linux machines. Browser was
used to inspect job results and the target GitHub repository's Runner inventory.
They do not establish a real GitHub Runner lifecycle or the complete LW matrix.

## Verified receipts

The control, interrupted-Create and backup receipts tested PR head
`935a5a02f706662425950801874adbb89c6e79a9` through checkout merge commit
`e3f2fb88b096b389c232c42a4a31cc879deac3ea`. The corrected state-outage and pressure receipts
tested `6697ee3055ac3be60ea13f265e1bffdd6269a58b` through merge commit
`050f1ba95e2b169c1fabad9dff0cca86f9f65214`; the actual binary digest is unchanged.
Each JSON file is the original bounded report extracted from that job's log;
private SQLite, credentials, raw state, plans and daemon logs were not exported.

| Receipt | Actual assertions | Job |
| --- | --- | --- |
| [control.json](control.json) | Two lost committed Spawn ACK replies; one Create and normal cleanup. Crash after exec handshake: no registration/Create, NeverStarted terminal. DELETE before Create-start: no late Create. Lost Create-start reply followed by restart: quarantine and occupancy retained. | [108472981241](https://github.com/5aaee9/shaula/actions/runs/36266820117/job/108472981241) |
| [interrupted-create.json](interrupted-create.json) | Engine commits one container while response is withheld; daemon SIGKILL and two restarts; original cgroup empty before recovery; no Create/registration replay; exact resource and occupancy remain quarantined. | [108472981250](https://github.com/5aaee9/shaula/actions/runs/36266820117/job/108472981250) |
| [backup.json](backup.json) | Writers stopped, SQLite quick_check, 26-file full-set manifest verified, same-path restore and epoch 2, no repeated Create/registration, normal exact-resource reclaim. A later real Create changes authority; restore refuses to replace the newer set, then current authority reclaims that resource. | [108472981224](https://github.com/5aaee9/shaula/actions/runs/36266820117/job/108472981224) |
| [backend-outage.json](backend-outage.json) | All HTTP-state methods unavailable after a real Docker Create; four rejected state requests, actual emergency state retained byte-exact across restart, one Create with no replay, exact external resource and occupancy retained in quarantine after transport recovery. | [108474373880](https://github.com/5aaee9/shaula/actions/runs/36267323379/job/108474373880) |
| [pressure.json](pressure.json) | Four-worker limit, one recovery reserve, six competing Fleets and three admitted Workers with one mutation permit; 20 samples, peak RSS 30,544 KiB and two threads per Worker; recovery at saturation and exact cleanup. 1,104 bounded log replays, 67 state requests, maximum state latency 331 ms; six concurrent status reads at most 13 ms. | [108474373804](https://github.com/5aaee9/shaula/actions/runs/36267323379/job/108474373804) |

The tested binary digest is
`sha256:4df6a0a9e95a3de8c7cc138867501292f21f2b386be8abae660a6cd2feae3445`.
The tuple is Linux `6.17.0-1022-azure`, cgroup v2, Terraform 1.9.8, Docker 28.0.4,
Forgejo 16.0.4, runner 13.1.0 at the report's pinned image digest and the bundled
Docker provider lock (`sha256:541a79ea1f8f922d68c5e0b00dbaaf8d78c3a43bd172d32a8a209ce12c7c951a`).
Archive digests can differ between fixture tar creation times; each actual
published artifact digest is retained in its receipt.

## Failed attempts and remaining gates

- An earlier backup attempt (`6073661`) stopped after external idle observation
  and subsequently encountered quarantine. The accepted run waits for the
  daemon's durable Idle checkpoint before backup; it does not accept quarantine
  as successful restoration or weaken recovery authority checks.
- The first state-outage test at `935a5a0` timed out looking in a guessed path.
  Its apply failed and the Generation retained CleanupRequired. The harness now
  reads the authoritative `workspace_path`; the corrected receipt above passes.
- The first pressure test at `935a5a0` timed out waiting for a fleeting queued
  status even though all three admitted Generations completed. The harness now
  waits for the actual workflow run to exist, observes Busy for a 40-second job,
  and still requires exact cleanup, bounded usage, recovery reserve and state
  progress under log load. The corrected receipt above passes.
- Backup at `6697ee3` reproduced that same queued-status race after restoration;
  the Generation had already reached Destroyed with its completion receipt.
  `589da3a` applies the accepted-run/40-second observation to the backup and
  control suites too. Its final complete CI rerun remains pending; the earlier
  successful receipt is preserved with its original commit, not relabelled.
- Real GitHub lifecycle and Auth Handoff/Create competition remain pending the
  isolated-host authorization and Browser dispatch. The repository inventory
  baseline has no Runner instances and one old offline `shaula-docker-local`
  Scale Set. That old set is not acceptance evidence for this binary.
- The pre-exec crash window is now classified from the attempt's exact cgroup,
  and LW-11/12/14 composition cases have local real-cgroup tests (see
  IMPLEMENTATION_STATUS). They are not hosted-CI receipts: the explicit process
  tests are not run by CI. Real-GitHub Auth Handoff/Create competition remains
  part of LW-32 above.

The backup receipt is a controlled same-host, full-set runbook rehearsal with
explicit fencing and divergence refusal, not a general automatic rollback tool.
The load profile is four Workers with one recovery reserve; no 1,024-Worker or
other deployment-size capacity claim is made. Production has not been deployed
or modified for these tests, and PR #6 remains unmerged.
