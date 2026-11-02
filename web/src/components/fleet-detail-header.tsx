import { Pencil, RefreshCw, Trash2 } from "lucide-react";
import { Link } from "react-router-dom";
import type { FleetSpec, FleetStatus } from "@/lib/types";
import { fleetTargetName } from "@/lib/runner-backend";
import { Button } from "@/components/ui/button";
import { Tip } from "@/components/icon-tooltip";
import { PageHeader } from "@/components/page-header";
import { StatusBadge } from "@/components/status";
import { FinalizeQuarantined } from "@/components/finalize-quarantined";

/** Title, live phase, target and related views, beside the fleet's actions. */
export function FleetDetailHeader({
  fleetKey,
  spec,
  status,
  scopes,
  onRefresh,
  onRetire,
}: {
  fleetKey: string;
  spec: FleetSpec;
  status?: FleetStatus;
  scopes: string[];
  onRefresh: () => void;
  onRetire: () => void;
}) {
  const query = `fleet_key=${encodeURIComponent(fleetKey)}`;
  return (
    <PageHeader
      back={{ to: "/fleets", label: "Fleets" }}
      title={fleetKey}
      meta={
        <>
          {status && <StatusBadge value={status.phase} />}
          <span>
            {fleetTargetName(spec)}
            {(!spec.kind || spec.kind === "github") && ` / ${spec.github.scale_set_name}`}
          </span>
          {status && (
            <span className="text-xs">
              Observed r{status.observedRevision} / Desired r{status.desiredRevision}
            </span>
          )}
          <span className="flex gap-3">
            <Link className="text-link" to={`/jobs?${query}`}>
              Jobs
            </Link>
            <Link className="text-link" to={`/jobs/runners?${query}`}>
              Unassigned runners
            </Link>
          </span>
        </>
      }
      actions={
        <>
          <Tip label="Refresh fleet">
            <Button size="icon" variant="outline" aria-label="Refresh fleet" onClick={onRefresh}>
              <RefreshCw />
            </Button>
          </Tip>
          {scopes.includes("fleet.retire") && (
            <FinalizeQuarantined fleetKey={fleetKey} onAccepted={onRefresh} />
          )}
          <Tip label="Retire fleet">
            <Button
              size="icon"
              variant="outline"
              aria-label="Retire fleet"
              onClick={onRetire}
              disabled={!scopes.includes("fleet.retire")}
            >
              <Trash2 />
            </Button>
          </Tip>
          {scopes.includes("fleet.write") ? (
            <Button asChild>
              <Link to={`/fleets/${encodeURIComponent(fleetKey)}/edit`} role="button">
                <Pencil />
                Edit fleet
              </Link>
            </Button>
          ) : (
            <Button disabled>
              <Pencil />
              Edit fleet
            </Button>
          )}
        </>
      }
    />
  );
}
