import { ArrowLeft, Pencil, RefreshCw, Trash2 } from "lucide-react";
import { Link } from "react-router-dom";
import type { FleetSpec } from "@/lib/types";
import { fleetTargetName } from "@/lib/runner-backend";
import { Button } from "@/components/ui/button";
import { Tip } from "@/components/icon-tooltip";
import { FinalizeQuarantined } from "@/components/finalize-quarantined";

/** Back link, title and the fleet's action bar. */
export function FleetDetailHeader({
  fleetKey,
  spec,
  scopes,
  onRefresh,
  onRetire,
}: {
  fleetKey: string;
  spec: FleetSpec;
  scopes: string[];
  onRefresh: () => void;
  onRetire: () => void;
}) {
  return (
    <>
      <Link to="/fleets" className="back-link">
        <ArrowLeft className="size-4" />
        Fleets
      </Link>
      <div className="page-heading">
        <div>
          <h1 className="break-all">{fleetKey}</h1>
          <p>
            {fleetTargetName(spec)}
            {spec.kind !== "forgejo" && (
              <>
                {" "}
                <span className="mx-2">/</span> {spec.github.scale_set_name}
              </>
            )}
          </p>
        </div>
        <div className="flex gap-2">
          <Button asChild variant="outline">
            <Link to={`/jobs?fleet_key=${encodeURIComponent(fleetKey)}`}>Jobs</Link>
          </Button>
          <Button asChild variant="outline">
            <Link to={`/jobs/runners?fleet_key=${encodeURIComponent(fleetKey)}`}>
              Unassigned runners
            </Link>
          </Button>
          <Tip label="Refresh fleet">
            <Button size="icon" variant="outline" aria-label="Refresh fleet" onClick={onRefresh}>
              <RefreshCw />
            </Button>
          </Tip>
          {scopes.includes("fleet.write") ? (
            <Button asChild variant="outline">
              <Link to={`/fleets/${encodeURIComponent(fleetKey)}/edit`} role="button">
                <Pencil />
                Edit fleet
              </Link>
            </Button>
          ) : (
            <Button variant="outline" disabled>
              <Pencil />
              Edit fleet
            </Button>
          )}
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
        </div>
      </div>
    </>
  );
}
