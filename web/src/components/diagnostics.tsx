import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { api, ApiError } from "@/lib/api";
import { QuestionView } from "@/components/diagnostic-question";
import { effectiveOutcome, outcomes, type Report } from "@/lib/diagnostics";

const summaryOrder = ["blocked", "unknown", "progressing", "satisfied", "not_applicable"];

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
  const forbidden = error instanceof ApiError && error.status === 403;
  const questions = (!forbidden && permitted && query.data?.questions) || [];
  const counts = new Map<string, number>();
  for (const question of questions) {
    const outcome = effectiveOutcome(question, !!error);
    counts.set(outcome, (counts.get(outcome) ?? 0) + 1);
  }
  const summary = summaryOrder
    .filter((outcome) => counts.has(outcome))
    .map((outcome) => `${counts.get(outcome)} ${outcomes[outcome].toLowerCase()}`)
    .join(" · ");
  return (
    <section className="details-section" aria-label="Why">
      <h2 className="mb-0">
        <button
          type="button"
          className="flex w-full items-center gap-2 text-left"
          aria-expanded={expanded}
          onClick={() => setExpanded(!expanded)}
        >
          <ChevronDown
            aria-hidden="true"
            className={cn("size-4 transition-transform", !expanded && "-rotate-90")}
          />
          <span>Why</span>
          {summary && <span className="text-sm font-normal text-muted-foreground">{summary}</span>}
        </button>
      </h2>
      {expanded && (
        <div className="mt-4 space-y-3">
          {!permitted ? (
            <p className="text-sm text-muted-foreground">fleet.read permission is required.</p>
          ) : (
            <>
              {query.isPending && (
                <p className="text-sm text-muted-foreground">Reading diagnostic evidence…</p>
              )}
              {error && (
                <p role="status" className="text-sm text-muted-foreground">
                  {unsupported
                    ? "This server does not support diagnostics. The existing status remains available."
                    : forbidden
                      ? "You no longer have permission to read diagnostics."
                      : error instanceof ApiError &&
                          (error.code === "DiagnosticsNotFound" || error.code === "DiagnosticsGone")
                        ? "This diagnostic subject no longer exists."
                        : "Diagnostics are temporarily unavailable. Existing status and edits are unchanged."}
                </p>
              )}
              {query.data?.truncated && (
                <p className="text-sm text-muted-foreground">
                  Partial report: some evidence was omitted to stay within the response budget.
                </p>
              )}
              {questions.map((question) => (
                <QuestionView
                  key={question.question}
                  question={question}
                  unavailable={!!error}
                  scopes={scopes}
                />
              ))}
              {questions.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  Explanations describe recorded controller decisions; they never authorize
                  lifecycle operations.
                </p>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}
