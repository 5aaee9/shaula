import {
  ChevronRight,
  CircleCheck,
  CircleDashed,
  CircleHelp,
  CircleMinus,
  Clock,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import { cn } from "cn";
import { Badge } from "@/components/ui/badge";
import { DiagnosticFacts } from "@/components/diagnostic-facts";
import {
  completionSources,
  effectiveOutcome,
  emptyExplanations,
  humanize,
  message,
  outcomes,
  timestamp,
  topics,
  type Question,
} from "@/lib/diagnostics";

// Every state carries an icon and a word; colour is never the only signal.
const tones: Record<string, { icon: LucideIcon; className: string }> = {
  satisfied: {
    icon: CircleCheck,
    className:
      "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900 dark:bg-emerald-950 dark:text-emerald-400",
  },
  passed: { icon: CircleCheck, className: "text-emerald-700 dark:text-emerald-400" },
  progressing: {
    icon: Clock,
    className:
      "border-sky-200 bg-sky-50 text-sky-700 dark:border-sky-900 dark:bg-sky-950 dark:text-sky-400",
  },
  pending: { icon: Clock, className: "text-sky-700 dark:text-sky-400" },
  blocked: {
    icon: TriangleAlert,
    className:
      "border-amber-200 bg-amber-50 text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-400",
  },
  not_applicable: { icon: CircleMinus, className: "text-muted-foreground" },
  not_evaluated: { icon: CircleDashed, className: "text-muted-foreground" },
  unknown: { icon: CircleHelp, className: "text-muted-foreground" },
};
function tone(value: string) {
  return tones[value] ?? tones.unknown;
}

export function QuestionView({
  question,
  unavailable,
  scopes,
}: {
  question: Question;
  unavailable: boolean;
  scopes: string[];
}) {
  const fresh = question.basis.freshness === "fresh" && !unavailable;
  const outcome = effectiveOutcome(question, unavailable);
  const primary = fresh
    ? question.reasons.find((r) => r.id === question.primaryReasonId)
    : undefined;
  const headline = primary ?? question.reasons[0];
  const completion = question.reasons.find((r) => r.code === "cleanup.completed")?.parameters
    ?.completionSource;
  const { icon: Icon, className } = tone(outcome);
  return (
    <article className="rounded-lg border bg-card">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2 p-4">
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-medium">
            {topics[question.question] ?? humanize(question.question)}
          </h3>
          <p className="mt-1 text-sm">
            {headline
              ? message(headline)
              : (emptyExplanations[outcome] ?? "No current evidence explains this question.")}
          </p>
          {completion && (
            <p className="mt-1 text-sm">
              Completion source: {completionSources[completion] ?? "Unknown"}
            </p>
          )}
          <p className="mt-2 text-xs text-muted-foreground">
            Observed{" "}
            <time
              dateTime={question.basis.observedAt ?? undefined}
              title={question.basis.observedAt ?? undefined}
            >
              {timestamp(question.basis.observedAt)}
            </time>{" "}
            · {unavailable ? "unavailable" : question.basis.freshness} · {question.coverage}{" "}
            coverage
          </p>
        </div>
        <Badge variant="outline" className={cn("gap-1.5", className)}>
          <Icon aria-hidden="true" />
          {outcomes[outcome]}
        </Badge>
      </div>
      <details className="group border-t">
        <summary className="flex cursor-pointer list-none items-center gap-1 px-4 py-2 text-xs text-muted-foreground hover:text-foreground [&::-webkit-details-marker]:hidden">
          <ChevronRight className="size-3.5 transition-transform group-open:rotate-90" />
          Evidence and evaluated stages
        </summary>
        <div className="space-y-4 px-4 pb-4 text-sm">
          <ol className="flex flex-wrap gap-2" aria-label="Evaluated stages">
            {question.stages.map((stage) => {
              const evaluation = fresh ? stage.evaluation : "unknown";
              const { icon: StageIcon, className: stageClass } = tone(evaluation);
              return (
                <li
                  key={stage.id}
                  className="inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-xs"
                >
                  <StageIcon aria-hidden="true" className={cn("size-3.5", stageClass)} />
                  <span>{humanize(stage.id)}</span>
                  <span className="text-muted-foreground">
                    {humanize(evaluation).toLowerCase()}
                  </span>
                </li>
              );
            })}
          </ol>
          {question.reasons.length > 0 && (
            <ul className="space-y-2">
              {question.reasons.map((reason) => (
                <li key={reason.id} className="rounded-md bg-muted/50 px-3 py-2">
                  {reason !== headline && <p>{message(reason)}</p>}
                  <p className="text-xs text-muted-foreground">
                    <code className="font-mono">{reason.code}</code> · {reason.severity} ·{" "}
                    {reason.effect}
                  </p>
                  {reason.evidenceIds.map((id) => {
                    const evidence = question.evidence.find((e) => e.id === id);
                    return evidence ? (
                      <p className="text-xs text-muted-foreground" key={id}>
                        {humanize(evidence.kind)} · {timestamp(evidence.observedAt)} ·{" "}
                        {evidence.freshness}
                      </p>
                    ) : null;
                  })}
                </li>
              ))}
            </ul>
          )}
          <DiagnosticFacts question={question} scopes={scopes} />
          <p className="text-xs text-muted-foreground">Source: {humanize(question.basis.kind)}</p>
        </div>
      </details>
    </article>
  );
}
