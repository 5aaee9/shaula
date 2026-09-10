---
status: accepted
date: 2026-09-10
---

# Process telemetry uses bounded OTLP/HTTP export

`shaula-observability` owns the process-wide `tracing` subscriber, local JSON
sink, finite metric labels and the optional OTLP/HTTP exporter. Core no longer
owns a telemetry port: the previous unconsumed counter registry could not
produce traces or metrics and duplicated adapter policy.

The daemon initializes this module after bootstrap validation and before the
database is opened or any remote side effect. `observability.otlp.endpoint`
is optional and bootstrap rejects any `observability.otlp.protocol` other
than `http/json`; omission of the section keeps local structured logs only.
Export requests use a five-second timeout and are spawned on the existing
Tokio runtime; an event observed outside a runtime joins the degraded
counter instead of being silently dropped. A failed or unreachable collector
is logged at debug level, bumps the in-process `degraded_exports()` counter
and never fails a commit, listener, lifecycle operation or shutdown.

Production call sites record through the narrow `TelemetryHandle` facade
(`shaula-daemon`, `shaula-http` and the binary depend on this crate):
HTTP request outcomes at the outermost router layer, registry admission at
the fleet/profile mutation funnels, reconcile per supervised scan tick,
runner outcomes at bounded GitHub operations (scale-set create, label
update, JIT mint), and IaC outcomes at engine create/destroy completions.
`MetricOperation::Exporter` never produces export traffic — it feeds the
degraded counter only, so degradation reporting cannot recurse into the
failing exporter.

Metric dimensions are finite (`operation` and `result`) and exclude resource
identities, URLs, actors, request bodies, credentials and error text. The
OTLP JSON payload posts the `shaula.operations.total` family as a DELTA
monotonic sum to `<endpoint>/v1/metrics`. The exporter is a bounded
plain-HTTP/1.1 POST over Tokio — the crate links no HTTP client, so
`shaula-daemon` may depend on this facade without pulling `reqwest` into
its tree (spec 0007 §2 gate); only `http://` endpoints are accepted and
TLS and authentication are supplied by the configured collector boundary.

Traces export through a `tracing` layer in this crate: spans created under
`shaula` targets are collected on close into a bounded queue (1024; overflow
counts as degraded export) and posted in batches (64 spans or 2 seconds) to
`<endpoint>/v1/traces` by a drain task. No OpenTelemetry SDK is linked — the
traceId is the root span's registry id zero-extended to 16 bytes and
parentSpanId comes from the registry, keeping parent/child correlation
consistent without new dependencies. Only the finite attributes declared on
the span are exported; resource identities may appear as span attributes for
correlation and never become metric labels. `TelemetryGuard::flush(budget)`
gives the daemon a bounded shutdown flush (spec 0001 §12). There is no
sampler knob: collection is always-on and the bounded queue is the pressure
valve.

The initial instrumented span set covers `shaula.daemon.startup` and
`shaula.daemon.shutdown`, `shaula.http.request` (route template + status
code), registry mutations (`shaula.registry.fleet_put`/`template_put`),
bounded GitHub operations (`shaula.github.scale_set_create`,
`scale_set_labels_update`, `jit_mint`), IaC operations
(`shaula.iac.generation_create`/`generation_destroy`) and the reconcile tick.
The remaining spec 0001 §13.1 span families are delivered incrementally.

## Consequences

- Local JSON remains available when OTLP is down.
- Telemetry is observable from production call sites through the narrow
  `TelemetryHandle` facade without coupling `shaula-core` to an exporter.
- OTLP export is best-effort and bounded; SQLite audit and durable state remain
  the source of truth.
- Both signals ship from one optional collector endpoint and reuse the same
  no-HTTP-client boundary, which is what keeps the daemon's dependency tree
  clean under the spec 0007 §2 gate.
