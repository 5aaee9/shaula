import { useState, type ReactNode } from "react";
import type { Resource } from "@/lib/api";
import { forgejoFleetBody } from "@/lib/fleet-form-body";
import { useTokenFleet } from "@/lib/use-token-fleet";
import type { FleetResource, ForgejoFleetSpec } from "@/lib/types";
import type { FleetFormProps } from "./fleet-form";
import { ForgejoAuthSection } from "./forgejo-fleet-sections";
import { TokenFleetFrame } from "./token-fleet-sections";

export function ForgejoFleetForm({
  resource: current,
  backendSelector,
  scopes,
  onClose,
  onAccepted,
  page = false,
}: Omit<FleetFormProps, "resource"> & {
  resource?: Resource<FleetResource<ForgejoFleetSpec>>;
  backendSelector?: ReactNode;
}) {
  const [resource] = useState(current);
  const section = resource?.data.spec.forgejo;
  const fleet = useTokenFleet({
    backend: "forgejo",
    resource,
    section,
    activeTarget: (state) => state?.forgejo?.target,
    scopes,
    page,
    onClose,
    onAccepted,
  });
  const [labels, setLabels] = useState(() => section?.labels.join(", ") || "");
  return (
    <TokenFleetFrame
      backend="forgejo"
      fleet={fleet}
      backendSelector={backendSelector}
      page={page}
      onClose={onClose}
      placeholder="forgejo-linux"
      buildBody={() => forgejoFleetBody({ ...fleet, fleetKey: fleet.key, labels })}
    >
      <ForgejoAuthSection
        editing={fleet.editing}
        fleetKey={fleet.key}
        choices={fleet.choices}
        canRead={fleet.canReadAuth}
        auth={fleet.auth}
        setAuth={fleet.setAuth}
        target={fleet.target}
        prefix={fleet.prefix}
        setPrefix={fleet.setPrefix}
        labels={labels}
        setLabels={setLabels}
      />
    </TokenFleetFrame>
  );
}
