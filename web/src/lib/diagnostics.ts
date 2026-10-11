import { diagnosticMessages } from "@/lib/diagnostic-messages";

export interface Reason {
  id: string;
  code: string;
  severity: string;
  effect: string;
  evidenceIds: string[];
  parameters?: { completionSource?: string };
}
export interface Question {
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
export interface Report {
  schemaVersion: number;
  subject: { kind: string; key?: string; id?: string; fleetIncarnation: string | null };
  generatedAt: string;
  questions: Question[];
  truncated: boolean;
}

export const outcomes: Record<string, string> = {
  satisfied: "Satisfied",
  progressing: "In progress",
  blocked: "Blocked",
  unknown: "Unknown",
  not_applicable: "Not applicable",
};
// Topic names stay neutral so a satisfied outcome never reads as an unanswered problem.
export const topics: Record<string, string> = {
  scale_up: "Scale-up",
  readiness: "Runner readiness",
  cleanup: "Cleanup",
  rollout: "Configuration rollout",
  job_dispatch: "Job dispatch",
};
export const emptyExplanations: Record<string, string> = {
  satisfied: "Nothing is holding this back.",
  not_applicable: "This does not apply to the resource right now.",
  progressing: "Work is in progress.",
};
export const completionSources: Record<string, string> = {
  provider_cleanup: "Provider cleanup",
  never_started: "Create apply never started",
  operator_attested: "Operator attestation",
  unknown: "Unknown",
};
export const capacityLabels: Record<string, string> = {
  demandKind: "Demand source",
  min: "Minimum",
  max: "Maximum",
  demand: "Demand",
  target: "Target",
  effective: "Effective",
  occupancy: "Occupancy",
  deficit: "Deficit",
  occupancyHeadroom: "Occupancy headroom",
  arithmeticCreateAllowance: "Create allowance",
  actuallyAdmitted: "Admitted",
};

export function humanize(value: string) {
  const words = value.replaceAll(/[_.]/g, " ").trim();
  return words ? words[0].toUpperCase() + words.slice(1) : value;
}
export function message(reason: Reason) {
  return (
    diagnosticMessages[reason.code] ??
    `The current client does not recognize this reason (${reason.code}).`
  );
}
export function timestamp(value: string | null) {
  if (!value) return "never";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
// Stale or unavailable evidence is shown conservatively as unknown, as are outcomes this client does not know.
export function effectiveOutcome(question: Question, unavailable: boolean) {
  const fresh = question.basis.freshness === "fresh" && !unavailable;
  return fresh && outcomes[question.outcome] ? question.outcome : "unknown";
}
