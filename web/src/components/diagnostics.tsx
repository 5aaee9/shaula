import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api";
import { diagnosticMessages } from "@/lib/diagnostic-messages";

interface Reason {
  id: string;
  code: string;
  severity: string;
  effect: string;
  evidenceIds: string[];
  parameters?: { completionSource?: string };
}
interface Question {
  question: string;
  outcome: string;
  coverage: string;
  primaryReasonId: string | null;
  basis: {
    kind: string;
    observedAt: string | null;
    freshness: string;
    subjectRevision: string | null;
  };
  reasons: Reason[];
  evidence: { id: string; kind: string; observedAt: string | null; freshness: string }[];
  stages: { id: string; evaluation: string; reasonIds: string[] }[];
  capacity: Record<string, string | null> | null;
  cleanupMode?: string;
  rollout?: {
    desiredRevision: string;
    observedRevision: string;
    previousPin: { key: string; revision: string } | null;
    candidatePin: { key: string; revision: string } | null;
  };
  pool?: {
    mode: string;
    scope: string;
    poolRevision: string | null;
    selectedMember: string | null;
    selection: string;
    truncated: boolean;
    members: {
      key: string;
      weight: string;
      cap: string | null;
      occupancy: string;
      excludedAtCap: boolean;
    }[];
  };
  suggestions?: {
    suggestionId: string;
    reference: { kind: string; key?: string; id?: string } | null;
  }[];
}
interface Report {
  schemaVersion: number;
  subject: { kind: string; key?: string; id?: string; fleetIncarnation: string | null };
  generatedAt: string;
  questions: Question[];
  truncated: boolean;
}
const outcomes: Record<string, string> = {
  satisfied: "Satisfied",
  progressing: "In progress",
  blocked: "Blocked",
  unknown: "Unknown",
  not_applicable: "Not applicable",
};
const titles: Record<string, string> = {
  scale_up: "Why are no more runners being created?",
  readiness: "Why is the runner not ready?",
  cleanup: "Why has cleanup not finished?",
  rollout: "Why has the configuration not converged?",
  job_dispatch: "Why has this job not started?",
};
const completionSources: Record<string, string> = {
  provider_cleanup: "Provider cleanup",
  never_started: "Create apply never started",
  operator_attested: "Operator attestation",
  unknown: "Unknown",
};
function message(reason: Reason) {
  return (
    diagnosticMessages[reason.code] ??
    `The current client does not recognize this reason (${reason.code}).`
  );
}

export function Diagnostics({
  kind,
  resourceKey,
  incarnation,
  revision,
  scopes,
}: {
  kind: "fleet" | "generation" | "job";
  resourceKey: string;
  incarnation?: string | null;
  revision?: number;
  scopes: string[];
}) {
  const [expanded, setExpanded] = useState(true);
  const [visible, setVisible] = useState(document.visibilityState === "visible");
  useEffect(() => {
    const update = () => setVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  const permitted = scopes.includes("fleet.read");
  const failures = useRef({ key: "", count: 0 });
  const identity = JSON.stringify([kind, resourceKey, incarnation, revision]);
  const query = useQuery({
    queryKey: ["diagnostics", kind, resourceKey, incarnation, revision],
    enabled: expanded && visible && permitted,
    queryFn: async ({ signal }) => {
      if (failures.current.key !== identity) failures.current = { key: identity, count: 0 };
      try {
        const { data } = await api<Report>(
          `/${kind}s/${encodeURIComponent(resourceKey)}/diagnostics`,
          { signal },
        );
        if (data.schemaVersion !== 1)
          throw new ApiError(
            200,
            "UnsupportedDiagnostics",
            "The server uses an unsupported diagnostics version.",
          );
        if (
          data.subject.kind !== kind ||
          (data.subject.key ?? data.subject.id) !== resourceKey ||
          (incarnation && data.subject.fleetIncarnation !== incarnation)
        ) {
          throw new Error("The subject changed. Waiting for current diagnostics.");
        }
        if (!Array.isArray(data.questions))
          throw new Error("Diagnostics are temporarily unavailable.");
        if (
          kind === "fleet" &&
          revision !== undefined &&
          data.questions.some(
            (q) => q.basis.subjectRevision !== null && q.basis.subjectRevision !== String(revision),
          )
        )
          throw new Error("Waiting for the current Fleet revision.");
        failures.current.count = 0;
        return data;
      } catch (error) {
        if (!signal.aborted && failures.current.key === identity) failures.current.count++;
        throw error;
      }
    },
    retry: false,
    refetchInterval: (q) =>
      Math.max(
        Math.min(30_000, 5_000 * 2 ** failures.current.count),
        q.state.error instanceof ApiError ? (q.state.error.retryAfterSeconds ?? 0) * 1000 : 0,
      ),
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: false,
  });
  const error = query.error;
  const unsupported =
    error instanceof ApiError &&
    (error.code === "InvalidResponse" ||
      error.code === "UnsupportedDiagnostics" ||
      (error.status === 404 && error.code !== "DiagnosticsNotFound"));
  return (
    <section className="details-section" aria-label="Why">
      <button
        type="button"
        className="flex w-full items-center justify-between text-left font-semibold"
        aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}
      >
        <span>Why</span>
        <span aria-hidden="true">{expanded ? "−" : "+"}</span>
      </button>
      {expanded && (
        <div className="mt-4 space-y-4">
          {!permitted ? (
            <p>fleet.read permission is required.</p>
          ) : (
            <>
              {query.isPending && <p>Reading diagnostic evidence…</p>}
              {error && (
                <p role="status">
                  {unsupported
                    ? "This server does not support diagnostics. The existing status remains available."
                    : error instanceof ApiError && error.status === 403
                      ? "You no longer have permission to read diagnostics."
                      : error instanceof ApiError &&
                          (error.code === "DiagnosticsNotFound" || error.code === "DiagnosticsGone")
                        ? "This diagnostic subject no longer exists."
                        : "Diagnostics are temporarily unavailable. Existing status and edits are unchanged."}
                </p>
              )}
              {query.data?.truncated && (
                <p>Partial report: some evidence was omitted to stay within the response budget.</p>
              )}
              {!(error instanceof ApiError && error.status === 403) &&
                query.data?.questions.map((question) => (
                  <QuestionView
                    key={question.question}
                    question={question}
                    unavailable={!!error}
                    scopes={scopes}
                  />
                ))}
            </>
          )}
        </div>
      )}
    </section>
  );
}

function QuestionView({
  question,
  unavailable,
  scopes,
}: {
  question: Question;
  unavailable: boolean;
  scopes: string[];
}) {
  const fresh = question.basis.freshness === "fresh" && !unavailable;
  const outcome = fresh ? (outcomes[question.outcome] ?? "Unknown") : "Unknown";
  const primary = fresh
    ? question.reasons.find((r) => r.id === question.primaryReasonId)
    : undefined;
  const completion = question.reasons.find((r) => r.code === "cleanup.completed")?.parameters
    ?.completionSource;
  return (
    <article className="rounded-md border p-4">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="font-medium">{titles[question.question] ?? question.question}</h3>
        <span
          className={
            outcome === "Blocked" ? "text-amber-700 dark:text-amber-400" : "text-muted-foreground"
          }
        >
          {outcome}
        </span>
      </div>
      <p className="mt-1 text-xs text-muted-foreground">
        {question.coverage} coverage · {unavailable ? "unavailable" : question.basis.freshness} ·
        observed {question.basis.observedAt ?? "never"}
      </p>
      <p className="mt-2 text-sm">
        {primary
          ? message(primary)
          : question.reasons.length
            ? message(question.reasons[0])
            : "No current evidence explains this question."}
      </p>
      {completion && (
        <p className="mt-2 text-sm">
          Completion source: {completionSources[completion] ?? "Unknown"}
        </p>
      )}
      <details className="mt-3 text-sm">
        <summary className="cursor-pointer">Evidence and evaluated stages</summary>
        <p className="mt-2 text-muted-foreground">
          Source: {question.basis.kind}. This explanation does not authorize lifecycle operations.
        </p>
        <ul className="mt-2 space-y-1">
          {question.stages.map((stage) => (
            <li key={stage.id}>
              {stage.id}: {fresh ? stage.evaluation : "unknown"}
            </li>
          ))}
        </ul>
        {question.reasons.map((reason) => (
          <div className="mt-3" key={reason.id}>
            <p>{message(reason)}</p>
            <p className="text-xs text-muted-foreground">
              {reason.code} · {reason.severity} · {reason.effect}
            </p>
            {reason.evidenceIds.map((id) => {
              const evidence = question.evidence.find((e) => e.id === id);
              return evidence ? (
                <p className="text-xs" key={id}>
                  {evidence.kind}: {evidence.observedAt ?? "unknown"} ({evidence.freshness})
                </p>
              ) : null;
            })}
          </div>
        ))}
        {question.capacity && (
          <dl className="mt-3 grid grid-cols-2 gap-2">
            {Object.entries(question.capacity).map(([key, value]) => (
              <div key={key}>
                <dt className="text-xs text-muted-foreground">{key}</dt>
                <dd className="font-mono">{value ?? "Unknown"}</dd>
              </div>
            ))}
          </dl>
        )}
        {question.cleanupMode && <p className="mt-3">Cleanup mode: {question.cleanupMode}</p>}
        {question.rollout && (
          <div className="mt-3">
            <p>
              Desired revision {question.rollout.desiredRevision}; observed revision{" "}
              {question.rollout.observedRevision}
            </p>
            {scopes.includes("template.read") && (
              <>
                <p>
                  Previous pin:{" "}
                  {question.rollout.previousPin
                    ? `${question.rollout.previousPin.key}@${question.rollout.previousPin.revision}`
                    : "Unknown"}
                </p>
                <p>
                  Candidate pin:{" "}
                  {question.rollout.candidatePin
                    ? `${question.rollout.candidatePin.key}@${question.rollout.candidatePin.revision}`
                    : "Unknown"}
                </p>
              </>
            )}
          </div>
        )}
        {scopes.includes("template.read") && question.pool && (
          <div className="mt-3 overflow-auto">
            <p>
              Routing: {question.pool.mode}; cap scope: {question.pool.scope}; selection:{" "}
              {question.pool.selection}
            </p>
            <p>
              Pool revision: {question.pool.poolRevision ?? "Not applicable"}; selected member:{" "}
              {question.pool.selectedMember ?? "None recorded"}
            </p>
            <table className="mt-2 w-full text-left">
              <thead>
                <tr>
                  <th>Member</th>
                  <th>Weight</th>
                  <th>Cap</th>
                  <th>Occupancy</th>
                  <th>Excluded at cap</th>
                </tr>
              </thead>
              <tbody>
                {question.pool.members.map((member) => (
                  <tr key={member.key}>
                    <td>{member.key}</td>
                    <td>{member.weight}</td>
                    <td>{member.cap ?? "None"}</td>
                    <td>{member.occupancy}</td>
                    <td>{member.excludedAtCap ? "Yes" : "No"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {question.pool.truncated && <p>Member evidence is partial.</p>}
          </div>
        )}
        {question.suggestions?.map((suggestion, index) => {
          const ref = suggestion.reference;
          const fleet =
            suggestion.suggestionId === "view_generations" && ref?.kind === "fleet" && ref.key;
          const generation =
            (suggestion.suggestionId === "view_generation" ||
              suggestion.suggestionId === "view_invocations") &&
            ref?.kind === "generation" &&
            ref.id;
          const permitted =
            scopes.includes("fleet.read") &&
            (suggestion.suggestionId !== "view_invocations" || scopes.includes("logs.read"));
          const path = fleet
            ? `/jobs/runners?fleet_key=${encodeURIComponent(fleet)}`
            : generation
              ? `/jobs/runners/${encodeURIComponent(generation)}`
              : null;
          return permitted && path ? (
            <p className="mt-3" key={index}>
              <Link className="underline" to={path}>
                {suggestion.suggestionId.replaceAll("_", " ")}
              </Link>
            </p>
          ) : null;
        })}
      </details>
    </article>
  );
}
