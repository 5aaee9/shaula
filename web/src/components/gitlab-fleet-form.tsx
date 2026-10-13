import { useState, type ReactNode } from "react";
import type { Resource } from "@/lib/api";
import { gitlabFleetBody } from "@/lib/fleet-form-body";
import { gitlabTargetName } from "@/lib/runner-backend";
import { useTokenFleet } from "@/lib/use-token-fleet";
import type { FleetResource, GitlabFleetSpec } from "@/lib/types";
import type { FleetFormProps } from "./fleet-form";
import { ProfileSelector } from "./profile-selector";
import { TokenFleetFrame } from "./token-fleet-sections";
import { Input } from "./ui/input";
import { Field } from "./status";

export function GitlabFleetForm({
  resource: current,
  backendSelector,
  scopes,
  onClose,
  onAccepted,
  page = false,
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
    page,
    onClose,
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
      page={page}
      onClose={onClose}
      placeholder="gitlab-linux"
      buildBody={() => gitlabFleetBody({ ...fleet, fleetKey: key, tags, runUntagged })}
    >
      <section className="form-stack border-t pt-6">
        <h2>GitLab authentication</h2>
        <ProfileSelector
          label="GitLab authentication profile"
          query={fleet.choices}
          value={fleet.auth}
          canRead={fleet.canReadAuth}
          permission="auth.read"
          disabled={editing}
          onChange={fleet.setAuth}
        />
        {target && (
          <p className="break-all text-sm" aria-label="GitLab target">
            {gitlabTargetName(target)}
          </p>
        )}
        <p className="text-sm text-muted-foreground">
          The instance and group or project come from the profile's Active revision. Target,
          profile, runner prefix and tags are fixed at fleet creation. Rotate the profile's token to
          replace credentials.
        </p>
        <Field label="Runner name prefix">
          <Input
            disabled={editing}
            maxLength={100}
            value={fleet.prefix}
            onChange={(e) => fleet.setPrefix(e.target.value)}
            placeholder={key ? `shaula-${key}-` : "Uses shaula-<fleet key>-"}
          />
        </Field>
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
          Use a unique prefix per fleet. Jobs run with the shell executor inside the runner
          container, so a job's image: is ignored; choose a template that provides the tools your
          jobs need. Do not share tags with persistent runners: they may pick up the same jobs.
        </p>
      </section>
    </TokenFleetFrame>
  );
}
