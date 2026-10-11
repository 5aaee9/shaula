import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table";
import type { FleetSpec, FleetStatus } from "@/lib/types";
import { Empty, ErrorNotice, Loading, StatusBadge } from "@/components/status";
import { SectionCards } from "@/components/section-cards";
import { TableColumns } from "@/components/table-columns";

/** Phase, revisions and capacity metrics from the fleet's live status. */
export function FleetStatusSummary({ status, spec }: { status: FleetStatus; spec: FleetSpec }) {
  // Forgejo has no assignment step, so its demand is the queue of waiting jobs.
  const demandLabel = spec.kind === "forgejo" ? "Waiting jobs" : "Assigned demand";
  return (
    <>
      <div className="mb-6 flex flex-wrap items-center gap-3">
        <StatusBadge value={status.phase} />
        <span className="text-xs text-muted-foreground">
          Observed r{status.observedRevision} / Desired r{status.desiredRevision}
        </span>
      </div>
      <SectionCards
        metrics={Object.entries(status.capacity).map(([label, value]) => ({
          label: label === "assignedDemand" ? demandLabel : label[0].toUpperCase() + label.slice(1),
          value,
        }))}
      />
      {status.lastError && <ErrorNotice error={new Error(status.lastError)} />}
    </>
  );
}

export function FleetConditions({
  pending,
  conditions,
}: {
  pending: boolean;
  conditions: FleetStatus["conditions"] | undefined;
}) {
  return (
    <section className="details-section">
      <h2>Reconciliation conditions</h2>
      {pending ? (
        <Loading />
      ) : !conditions?.length ? (
        <Empty title="No conditions reported" />
      ) : (
        <div className="table-scroll">
          <Table>
            <TableColumns labels={["Condition", "Status", "Reason"]} />
            <TableBody>
              {conditions.map((condition) => (
                <TableRow key={condition.type}>
                  <TableCell>{condition.type}</TableCell>
                  <TableCell>
                    <StatusBadge value={condition.status ? "True" : "False"} />
                  </TableCell>
                  <TableCell>{condition.reason || "--"}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </section>
  );
}
