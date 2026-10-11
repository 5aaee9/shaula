import { Link } from "react-router-dom";
import { ArrowRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { capacityLabels, humanize, type Question } from "@/lib/diagnostics";

function pin(value: { key: string; revision: string } | null) {
  return value ? `${value.key}@${value.revision}` : "Unknown";
}

export function DiagnosticFacts({ question, scopes }: { question: Question; scopes: string[] }) {
  const templates = scopes.includes("template.read");
  const links = (question.suggestions ?? []).flatMap((suggestion, index) => {
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
    return permitted && path
      ? [{ key: index, path, label: humanize(suggestion.suggestionId) }]
      : [];
  });
  return (
    <>
      {question.capacity && (
        <dl className="grid grid-cols-2 gap-x-6 gap-y-3 sm:grid-cols-3 lg:grid-cols-4">
          {Object.entries(question.capacity).map(([key, value]) => (
            <div key={key} className="min-w-0">
              <dt className="text-xs text-muted-foreground">{capacityLabels[key] ?? key}</dt>
              <dd className="break-all tabular-nums">
                {value === null ? "Unknown" : key === "demandKind" ? humanize(value) : value}
              </dd>
            </div>
          ))}
        </dl>
      )}
      {question.cleanupMode && <p>Cleanup mode: {humanize(question.cleanupMode)}</p>}
      {question.rollout && (
        <dl className="grid grid-cols-2 gap-x-6 gap-y-3 sm:grid-cols-4">
          <div>
            <dt className="text-xs text-muted-foreground">Desired revision</dt>
            <dd className="tabular-nums">{question.rollout.desiredRevision}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Observed revision</dt>
            <dd className="tabular-nums">{question.rollout.observedRevision}</dd>
          </div>
          {templates && (
            <>
              <div>
                <dt className="text-xs text-muted-foreground">Previous pin</dt>
                <dd className="break-all">{pin(question.rollout.previousPin)}</dd>
              </div>
              <div>
                <dt className="text-xs text-muted-foreground">Candidate pin</dt>
                <dd className="break-all">{pin(question.rollout.candidatePin)}</dd>
              </div>
            </>
          )}
        </dl>
      )}
      {templates && question.pool && (
        <div className="space-y-2">
          <p className="text-xs text-muted-foreground">
            Routing {humanize(question.pool.mode).toLowerCase()} · cap scope{" "}
            {humanize(question.pool.scope).toLowerCase()} · selection{" "}
            {humanize(question.pool.selection).toLowerCase()} · pool revision{" "}
            {question.pool.poolRevision ?? "not applicable"} · selected member{" "}
            {question.pool.selectedMember ?? "none recorded"}
          </p>
          <div className="overflow-auto rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Member</TableHead>
                  <TableHead>Weight</TableHead>
                  <TableHead>Cap</TableHead>
                  <TableHead>Occupancy</TableHead>
                  <TableHead>Excluded at cap</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {question.pool.members.map((member) => (
                  <TableRow key={member.key}>
                    <TableCell>{member.key}</TableCell>
                    <TableCell className="tabular-nums">{member.weight}</TableCell>
                    <TableCell className="tabular-nums">{member.cap ?? "None"}</TableCell>
                    <TableCell className="tabular-nums">{member.occupancy}</TableCell>
                    <TableCell>{member.excludedAtCap ? "Yes" : "No"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
          {question.pool.truncated && <p>Member evidence is partial.</p>}
        </div>
      )}
      {links.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {links.map((link) => (
            <Button key={link.key} asChild variant="outline" size="sm">
              <Link to={link.path}>
                {link.label}
                <ArrowRight />
              </Link>
            </Button>
          ))}
        </div>
      )}
    </>
  );
}
