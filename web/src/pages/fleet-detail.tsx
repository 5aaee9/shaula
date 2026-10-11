import { Diagnostics } from "@/components/diagnostics";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { useState } from "react";
import { useLocation, useParams } from "react-router-dom";
import { useFleet, useFleetStatus } from "@/lib/queries";
import { resourcePath } from "@/lib/api";
import { type ChangeRef } from "@/lib/types";
import { ErrorNotice, Loading } from "@/components/status";
import { RetireDialog } from "@/components/retire-dialog";
import { ChangeNotice } from "@/components/change-notice";
import { FleetDetailHeader } from "@/components/fleet-detail-header";
import { FleetConditions, FleetStatusSummary } from "@/components/fleet-detail-status";
import { FleetOverview } from "@/components/fleet-detail-overview";
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
  const location = useLocation();
  const [retire, setRetire] = useState(false);
  const [change, setChange] = useState<ChangeRef | null>(
    () => (location.state as { change?: ChangeRef } | null)?.change || null,
  );
  if (fleet.isPending) return <Loading />;
  if (fleet.error) return <ErrorNotice error={fleet.error} retry={() => void fleet.refetch()} />;
  const { data } = fleet.data;
  const live = status.data?.data;
  return (
    <>
      <FleetDetailHeader
        fleetKey={fleetKey}
        spec={data.spec}
        scopes={scopes}
        onRefresh={() => {
          void fleet.refetch();
          void status.refetch();
        }}
        onRetire={() => setRetire(true)}
      />
      {status.error && <ErrorNotice error={status.error} retry={() => void status.refetch()} />}
      <ChangeNotice change={change} />
      {live && <FleetStatusSummary status={live} spec={data.spec} />}
      <Diagnostics
        kind="fleet"
        resourceKey={fleetKey}
        incarnation={data.metadata.incarnation}
        revision={data.metadata.revision}
        scopes={scopes}
      />
      <Tabs defaultValue="overview">
        <TabsList className="max-w-full justify-start overflow-x-auto" aria-label="Fleet views">
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="conditions">
            Conditions <span className="count">{live?.conditions.length || 0}</span>
          </TabsTrigger>
          <TabsTrigger value="spec">Desired specification</TabsTrigger>
        </TabsList>
        <TabsContent value="overview">
          <FleetOverview fleet={data} status={live} />
        </TabsContent>
        <TabsContent value="conditions">
          <FleetConditions pending={status.isPending} conditions={live?.conditions} />
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
