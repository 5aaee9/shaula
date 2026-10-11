import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api";
import { effectiveOutcome, outcomes, type Question, type Report } from "@/lib/diagnostics";

export type DiagnosticsKind = "fleet" | "generation" | "job";

const summaryOrder = ["blocked", "unknown", "progressing", "satisfied", "not_applicable"];

function usePageVisible() {
  const [visible, setVisible] = useState(document.visibilityState === "visible");
  useEffect(() => {
    const update = () => setVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return visible;
}

function validateReport(
  data: Report,
  kind: DiagnosticsKind,
  resourceKey: string,
  incarnation?: string | null,
  revision?: number,
) {
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
  if (!Array.isArray(data.questions)) throw new Error("Diagnostics are temporarily unavailable.");
  if (
    kind === "fleet" &&
    revision !== undefined &&
    data.questions.some(
      (q) => q.basis.subjectRevision !== null && q.basis.subjectRevision !== String(revision),
    )
  )
    throw new Error("Waiting for the current Fleet revision.");
}

/** Polls the diagnostics report while expanded and visible, backing off on failures. */
export function useDiagnosticsReport({
  kind,
  resourceKey,
  incarnation,
  revision,
  enabled,
}: {
  kind: DiagnosticsKind;
  resourceKey: string;
  incarnation?: string | null;
  revision?: number;
  enabled: boolean;
}) {
  const visible = usePageVisible();
  const failures = useRef({ key: "", count: 0 });
  const identity = JSON.stringify([kind, resourceKey, incarnation, revision]);
  return useQuery({
    queryKey: ["diagnostics", kind, resourceKey, incarnation, revision],
    enabled: enabled && visible,
    queryFn: async ({ signal }) => {
      if (failures.current.key !== identity) failures.current = { key: identity, count: 0 };
      try {
        const { data } = await api<Report>(
          `/${kind}s/${encodeURIComponent(resourceKey)}/diagnostics`,
          { signal },
        );
        validateReport(data, kind, resourceKey, incarnation, revision);
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
}

export function isForbidden(error: Error | null) {
  return error instanceof ApiError && error.status === 403;
}

export function diagnosticsErrorMessage(error: Error) {
  if (
    error instanceof ApiError &&
    (error.code === "InvalidResponse" ||
      error.code === "UnsupportedDiagnostics" ||
      (error.status === 404 && error.code !== "DiagnosticsNotFound"))
  )
    return "This server does not support diagnostics. The existing status remains available.";
  if (isForbidden(error)) return "You no longer have permission to read diagnostics.";
  if (
    error instanceof ApiError &&
    (error.code === "DiagnosticsNotFound" || error.code === "DiagnosticsGone")
  )
    return "This diagnostic subject no longer exists.";
  return "Diagnostics are temporarily unavailable. Existing status and edits are unchanged.";
}

export function summarizeOutcomes(questions: Question[], unavailable: boolean) {
  const counts = new Map<string, number>();
  for (const question of questions) {
    const outcome = effectiveOutcome(question, unavailable);
    counts.set(outcome, (counts.get(outcome) ?? 0) + 1);
  }
  return summaryOrder
    .filter((outcome) => counts.has(outcome))
    .map((outcome) => `${counts.get(outcome)} ${outcomes[outcome].toLowerCase()}`)
    .join(" · ");
}
