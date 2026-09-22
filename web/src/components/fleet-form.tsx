import { useState } from "react";
import type { Resource } from "@/lib/api";
import type { ChangeRef, FleetResource, ForgejoFleetSpec, GitHubFleetSpec } from "@/lib/types";
import type { RunnerBackend } from "@/lib/runner-backend";
import { BackendSelector } from "./runner-backend-fields";
import { GitHubFleetForm } from "./github-fleet-form";
import { ForgejoFleetForm } from "./forgejo-fleet-form";

export interface FleetFormProps {
  resource?: Resource<FleetResource>;
  scopes: string[];
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
  page?: boolean;
}

export function FleetForm(props: FleetFormProps) {
  const [backend, setBackend] = useState<RunnerBackend>(props.resource?.data.spec.kind || "github");
  const selector = !props.resource && <BackendSelector value={backend} onChange={setBackend} />;
  // Each provider has its own form; switching discards the other provider's draft.
  return backend === "forgejo" ? (
    <ForgejoFleetForm
      {...props}
      resource={props.resource as Resource<FleetResource<ForgejoFleetSpec>> | undefined}
      backendSelector={selector}
    />
  ) : (
    <GitHubFleetForm
      {...props}
      resource={props.resource as Resource<FleetResource<GitHubFleetSpec>> | undefined}
      backendSelector={selector}
    />
  );
}
