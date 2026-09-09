import { AuthRouteDetails } from "@/components/auth-route-details";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { useState } from "react";
import { ArrowLeft, Pencil, RefreshCw, Trash2 } from "lucide-react";
import { Link, useParams } from "react-router-dom";
import { useFleet, useFleetStatus } from "@/lib/queries";
import { resourcePath } from "@/lib/api";
import { targetName, type ChangeRef } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Tip } from "@/components/icon-tooltip";
import { Empty, ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import { FleetForm } from "@/components/fleet-form";
import { RetireDialog } from "@/components/retire-dialog";
import { ChangeNotice } from "@/components/change-notice";
import { SectionCards } from "@/components/section-cards";
export function FleetDetail({ scopes }: { scopes: string[] }) {
  const { key = "" } = useParams();
  return scopes.includes("fleet.read") ? (
    <FleetView key={key} fleetKey={key} scopes={scopes} />
  ) : (
    <ErrorNotice error={new Error("Fleet read permission is required.")} />
  );
}
function FleetView({ fleetKey, scopes }: { fleetKey: string; scopes: string[] }) {
  const fleet = useFleet(fleetKey);
  const status = useFleetStatus(fleetKey);
  const [edit, setEdit] = useState(false);
  const [retire, setRetire] = useState(false);
  const [change, setChange] = useState<ChangeRef | null>(null);
  if (fleet.isPending) return <Loading />;
  if (fleet.error) return <ErrorNotice error={fleet.error} retry={() => void fleet.refetch()} />;
  const { data } = fleet.data;
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
            {targetName(data.spec.github.target)} <span className="mx-2">/</span>{" "}
            {data.spec.github.scale_set_name}
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
            <Button
              size="icon"
              variant="outline"
              aria-label="Refresh fleet"
              onClick={() => {
                void fleet.refetch();
                void status.refetch();
              }}
            >
              <RefreshCw />
            </Button>
          </Tip>
          <Button
            variant="outline"
            onClick={() => setEdit(true)}
            disabled={!scopes.includes("fleet.write")}
          >
            <Pencil />
            Edit fleet
          </Button>
          <Tip label="Retire fleet">
            <Button
              size="icon"
              variant="outline"
              aria-label="Retire fleet"
              onClick={() => setRetire(true)}
              disabled={!scopes.includes("fleet.retire")}
            >
              <Trash2 />
            </Button>
          </Tip>
        </div>
      </div>
      <ChangeNotice change={change} />
      {status.error && <ErrorNotice error={status.error} retry={() => void status.refetch()} />}
      {status.data && (
        <>
          <div className="mb-6 flex flex-wrap items-center gap-3">
            <StatusBadge value={status.data.data.phase} />
            <span className="text-xs text-muted-foreground">
              Observed r{status.data.data.observedRevision} / Desired r
              {status.data.data.desiredRevision}
            </span>
          </div>
          <SectionCards
            metrics={Object.entries(status.data.data.capacity).map(([label, value]) => ({
              label:
                label === "assignedDemand"
                  ? "Assigned demand"
                  : label[0].toUpperCase() + label.slice(1),
              value,
            }))}
          />
          {status.data.data.lastError && (
            <ErrorNotice error={new Error(status.data.data.lastError)} />
          )}
        </>
      )}
      <Tabs defaultValue="overview">
        <TabsList className="max-w-full justify-start overflow-x-auto" aria-label="Fleet views">
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="conditions">
            Conditions <span className="count">{status.data?.data.conditions.length || 0}</span>
          </TabsTrigger>
          <TabsTrigger value="spec">Desired specification</TabsTrigger>
        </TabsList>
        <TabsContent value="overview">
          <section className="details-section">
            <h2>Configuration</h2>
            <dl className="details-grid">
              <KeyValue label="GitHub target">{targetName(data.spec.github.target)}</KeyValue>
              <KeyValue label="Runner group">{data.spec.github.runner_group}</KeyValue>
              <KeyValue label="Minimum runners">{data.spec.capacity.min_runners}</KeyValue>
              <KeyValue label="Maximum runners">{data.spec.capacity.max_runners}</KeyValue>
              <KeyValue label="Template profile">
                {data.resolved.template ? (
                  <Link
                    className="text-link"
                    to={`/templates?key=${encodeURIComponent(data.resolved.template.key)}`}
                  >
                    {data.resolved.template.key} / r{data.resolved.template.revision}
                  </Link>
                ) : (
                  "--"
                )}
              </KeyValue>
              <KeyValue label="GitHub authentication">
                <Link
                  className="text-link"
                  to={`/auth?key=${encodeURIComponent(data.resolved.authDesired.profileKey)}`}
                >
                  {data.resolved.authDesired.profileKey} / r{data.resolved.authDesired.revision}
                </Link>
              </KeyValue>
              {status.data?.data.githubAuth?.context && (
                <KeyValue label="Auth context (this target)">
                  <p>
                    {status.data.data.githubAuth.context.state}
                    {status.data.data.githubAuth.context.reason &&
                      `: ${status.data.data.githubAuth.context.reason}`}
                  </p>
                  <AuthRouteDetails
                    label="Desired route"
                    route={status.data.data.githubAuth.context.desiredRoute}
                    reference={status.data.data.githubAuth.context.desired}
                  />
                  <AuthRouteDetails
                    label="Observed route"
                    route={status.data.data.githubAuth.context.observedRoute}
                    reference={status.data.data.githubAuth.context.observed}
                  />
                  <p className="text-xs text-muted-foreground">
                    Observed records the last accepted route. Access is verified again before GitHub
                    operations.
                  </p>
                </KeyValue>
              )}
              <KeyValue label="Labels">
                <div className="flex flex-wrap gap-1">
                  {data.spec.github.labels.length
                    ? data.spec.github.labels.map((label) => (
                        <span className="label-chip" key={label}>
                          {label}
                        </span>
                      ))
                    : "None"}
                </div>
              </KeyValue>
              <KeyValue label="Incarnation">
                <span className="mono text-xs">{data.metadata.incarnation}</span>
              </KeyValue>
            </dl>
          </section>
        </TabsContent>
        <TabsContent value="conditions">
          <section className="details-section">
            <h2>Reconciliation conditions</h2>
            {status.isPending ? (
              <Loading />
            ) : !status.data?.data.conditions.length ? (
              <Empty title="No conditions reported" />
            ) : (
              <div className="table-scroll">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Condition</TableHead>
                      <TableHead>Status</TableHead>
                      <TableHead>Reason</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {status.data.data.conditions.map((condition) => (
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
        </TabsContent>
        <TabsContent value="spec">
          <section className="details-section">
            <h2>
              Desired specification <span className="count">r{data.metadata.revision}</span>
            </h2>
            <pre className="json-view">{JSON.stringify(data.spec, null, 2)}</pre>
          </section>
        </TabsContent>
      </Tabs>
      {edit && (
        <FleetForm
          resource={fleet.data}
          scopes={scopes}
          onClose={() => setEdit(false)}
          onAccepted={setChange}
        />
      )}
      {retire && (
        <RetireDialog
          name={fleetKey}
          path={resourcePath("fleets", fleetKey)}
          etag={fleet.data.etag}
          type="fleet"
          onClose={() => setRetire(false)}
          onAccepted={setChange}
        />
      )}
    </>
  );
}
