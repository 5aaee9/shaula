import { useState, type ReactNode } from "react";
import type { Resource } from "@/lib/api";
import { gitlabFleetBody } from "@/lib/fleet-form-body";
import { gitlabTargetName } from "@/lib/runner-backend";
import { useTokenFleet } from "@/lib/use-token-fleet";
import type { FleetResource, GitlabFleetSpec } from "@/lib/types";
import type { FleetFormProps } from "./fleet-form";
import { TokenAuthStage, TokenFleetFrame } from "./token-fleet-sections";
import { Input } from "./ui/input";
import { Field } from "./status";

export function GitlabFleetForm({
  resource: current,
  backendSelector,
  scopes,
  onClose,
  onAccepted,
}: Omit<FleetFormProps, "resource"> & {
  resource?: Resource<FleetResource<GitlabFleetSpec>>;
  backendSelector?: ReactNode;
}) {
  const [resource] = useState(current);
  const section = resource?.data.spec.gitlab;
  const fleet = useTokenFleet({
    backend: "gitlab",
    resource,
    section,
    activeTarget: (state) => state?.gitlab?.target,
    scopes,
    onAccepted,
  });
  const [tags, setTags] = useState(() => section?.tags.join(", ") || "");
  const [runUntagged, setRunUntagged] = useState(section?.run_untagged ?? false);
  const { editing, key, target } = fleet;
  return (
    <TokenFleetFrame
      backend="gitlab"
      fleet={fleet}
      backendSelector={backendSelector}
      onClose={onClose}
      placeholder="gitlab-linux"
      buildBody={() => gitlabFleetBody({ ...fleet, fleetKey: key, tags, runUntagged })}
    >
      <TokenAuthStage
        provider="GitLab"
        fleet={fleet}
        targetLabel={target ? gitlabTargetName(target) : undefined}
      >
        <Field label="Tags">
          <Input
            required={!runUntagged}
            disabled={editing}
            value={tags}
            onChange={(e) => setTags(e.target.value)}
            placeholder="linux, docker"
          />
        </Field>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            disabled={editing}
            checked={runUntagged}
            onChange={(e) => setRunUntagged(e.target.checked)}
          />
          Run untagged jobs
        </label>
        <p className="text-sm text-muted-foreground">
          Jobs use the shell executor, so image: is ignored. Don't share tags with persistent
          runners; they may take the same jobs.
        </p>
      </TokenAuthStage>
    </TokenFleetFrame>
  );
}
