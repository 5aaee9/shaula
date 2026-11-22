import { useState, type ReactNode } from "react";
import type { Resource } from "@/lib/api";
import { forgejoFleetBody } from "@/lib/fleet-form-body";
import { forgejoTargetName } from "@/lib/runner-backend";
import { useTokenFleet } from "@/lib/use-token-fleet";
import type { FleetResource, ForgejoFleetSpec } from "@/lib/types";
import type { FleetFormProps } from "./fleet-form";
import { TokenAuthStage, TokenFleetFrame } from "./token-fleet-sections";
import { Input } from "./ui/input";
import { Field } from "./status";

export function ForgejoFleetForm({
  resource: current,
  backendSelector,
  scopes,
  onClose,
  onAccepted,
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
    onAccepted,
  });
  const [labels, setLabels] = useState(() => section?.labels.join(", ") || "");
  return (
    <TokenFleetFrame
      backend="forgejo"
      fleet={fleet}
      backendSelector={backendSelector}
      onClose={onClose}
      placeholder="forgejo-linux"
      buildBody={() => forgejoFleetBody({ ...fleet, fleetKey: fleet.key, labels })}
    >
      <TokenAuthStage
        provider="Forgejo"
        fleet={fleet}
        targetLabel={fleet.target ? forgejoTargetName(fleet.target) : undefined}
      >
        <Field label="Labels">
          <Input
            required
            disabled={fleet.editing}
            value={labels}
            onChange={(e) => setLabels(e.target.value)}
            placeholder="linux:host"
          />
        </Field>
      </TokenAuthStage>
    </TokenFleetFrame>
  );
}
