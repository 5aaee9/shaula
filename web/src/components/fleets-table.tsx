import { ArrowRight, Boxes, CircleDot } from "lucide-react";
import { Link } from "react-router-dom";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import type { FleetStatus } from "@/lib/types";
import type { FleetRow } from "@/lib/use-fleet-rows";
import { fleetTargetName } from "@/lib/runner-backend";
import { Button } from "@/components/ui/button";
import { Tip } from "@/components/icon-tooltip";
import { StatusBadge } from "@/components/status";

/** The populated fleet inventory: one row per fleet that passed the filters. */
export function FleetsTable({ rows, total }: { rows: FleetRow[]; total: number }) {
  return (
    <div className="table-scroll">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Fleet / GitHub target</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Capacity</TableHead>
            <TableHead>Template</TableHead>
            <TableHead>Revision</TableHead>
            <TableHead>
              <span className="sr-only">Open</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row) => (
            <FleetTableRow key={row.key} row={row} />
          ))}
        </TableBody>
      </Table>
      <div className="table-footer">
        <span>
          {rows.length} of {total} fleets
        </span>
        <span className="flex items-center gap-1.5">
          <CircleDot className="size-3 text-emerald-600" />
          Live status
        </span>
      </div>
    </div>
  );
}

function FleetTableRow({ row }: { row: FleetRow }) {
  const resolved = row.resource?.resolved;
  const poolSize = resolved?.templatePool?.length;
  return (
    <TableRow>
      <TableCell>
        <Link to={`/fleets/${encodeURIComponent(row.key)}`} className="resource-name">
          <span className="resource-icon">
            <Boxes />
          </span>
          <span>
            <strong>{row.key}</strong>
            <small>{row.resource ? fleetTargetName(row.resource.spec) : "Loading target..."}</small>
          </span>
        </Link>
      </TableCell>
      <TableCell>
        <StatusBadge value={row.error ? "Unavailable" : row.status?.phase || "Loading"} />
        {row.status?.lastError && (
          <p className="table-detail text-red-700">{row.status.lastError}</p>
        )}
      </TableCell>
      <TableCell>{row.status ? <CapacityCell capacity={row.status.capacity} /> : "--"}</TableCell>
      <TableCell>
        <div className="table-detail">
          {poolSize ? `${poolSize} weighted members` : resolved?.template?.key || "--"}
        </div>
        {resolved?.template && !poolSize && (
          <small className="text-muted-foreground">Revision {resolved.template.revision}</small>
        )}
      </TableCell>
      <TableCell>
        <span className="mono text-xs">r{row.revision}</span>
      </TableCell>
      <TableCell>
        <Tip label={`Open ${row.key}`}>
          <Button asChild size="icon" variant="ghost">
            <Link aria-label={`Open ${row.key}`} to={`/fleets/${encodeURIComponent(row.key)}`}>
              <ArrowRight />
            </Link>
          </Button>
        </Tip>
      </TableCell>
    </TableRow>
  );
}

function CapacityCell({ capacity }: { capacity: FleetStatus["capacity"] }) {
  return (
    <div className="capacity-cell">
      <span>
        <strong>{capacity.effective}</strong>
        <span className="text-muted-foreground"> / {capacity.target}</span>
      </span>
      <div className="capacity-track">
        <span
          style={{
            width: `${Math.min(100, (capacity.effective / Math.max(1, capacity.target)) * 100)}%`,
          }}
        />
      </div>
    </div>
  );
}
