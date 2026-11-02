import { useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "cn";
import { QuestionView } from "@/components/diagnostic-question";
import type { Question, Report } from "@/lib/diagnostics";
import {
  diagnosticsErrorMessage,
  isForbidden,
  summarizeOutcomes,
  useDiagnosticsReport,
  type DiagnosticsKind,
} from "@/lib/use-diagnostics-report";

function DiagnosticsBody({
  pending,
  error,
  report,
  questions,
  scopes,
}: {
  pending: boolean;
  error: Error | null;
  report: Report | undefined;
  questions: Question[];
  scopes: string[];
}) {
  return (
    <>
      {pending && <p className="text-sm text-muted-foreground">Reading diagnostic evidence…</p>}
      {error && (
        <p role="status" className="text-sm text-muted-foreground">
          {diagnosticsErrorMessage(error)}
        </p>
      )}
      {report?.truncated && (
        <p className="text-sm text-muted-foreground">Partial report: some evidence was omitted.</p>
      )}
      {questions.map((question) => (
        <QuestionView
          key={question.question}
          question={question}
          unavailable={!!error}
          scopes={scopes}
        />
      ))}
    </>
  );
}

export function Diagnostics({
  kind,
  resourceKey,
  incarnation,
  revision,
  scopes,
}: {
  kind: DiagnosticsKind;
  resourceKey: string;
  incarnation?: string | null;
  revision?: number;
  scopes: string[];
}) {
  const [expanded, setExpanded] = useState(true);
  const permitted = scopes.includes("fleet.read");
  const query = useDiagnosticsReport({
    kind,
    resourceKey,
    incarnation,
    revision,
    enabled: expanded && permitted,
  });
  const error = query.error;
  const questions = (!isForbidden(error) && permitted && query.data?.questions) || [];
  const summary = summarizeOutcomes(questions, !!error);
  // With nothing to explain, the reason sits on the heading line instead of opening a body.
  const inlineError = permitted && expanded && !!error && !questions.length;
  return (
    <section className="details-section" aria-label="Why">
      <h2 className="mb-0 flex flex-wrap items-center gap-x-3 gap-y-1">
        <button
          type="button"
          className="flex items-center gap-2 text-left"
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
        {inlineError && (
          <span role="status" className="text-sm font-normal text-muted-foreground">
            {diagnosticsErrorMessage(error)}
          </span>
        )}
      </h2>
      {expanded && !inlineError && (
        <div className="mt-4 space-y-3">
          {permitted ? (
            <DiagnosticsBody
              pending={query.isPending}
              error={error}
              report={query.data}
              questions={questions}
              scopes={scopes}
            />
          ) : (
            <p className="text-sm text-muted-foreground">fleet.read permission is required.</p>
          )}
        </div>
      )}
    </section>
  );
}
