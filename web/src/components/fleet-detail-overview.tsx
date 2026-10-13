import { Link } from "react-router-dom";
import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table";
import type { FleetResource, FleetSpec, FleetStatus } from "@/lib/types";
import { backendLabel, fleetTargetName } from "@/lib/runner-backend";
import { KeyValue } from "@/components/status";
import { AuthRouteDetails } from "@/components/auth-route-details";
import { TableColumns } from "@/components/table-columns";

type Resolved = FleetResource["resolved"];
type AuthContext = NonNullable<NonNullable<FleetStatus["githubAuth"]>["context"]>;

/** The fleet's admitted configuration and, when pooled, its member table. */
export function FleetOverview({ fleet, status }: { fleet: FleetResource; status?: FleetStatus }) {
  const { spec } = fleet;
  const backend = backendLabel(spec.kind);
  const github = !spec.kind || spec.kind === "github";
  const provider = github ? "GitHub" : backend;
  const authContext = github ? status?.githubAuth?.context : undefined;
  return (
    <section className="details-section">
      <h2>Configuration</h2>
      <dl className="details-grid">
        <KeyValue label="Runner backend">{backend}</KeyValue>
        <KeyValue label={`${provider} target`}>{fleetTargetName(spec)}</KeyValue>
        <BackendFacts spec={spec} />
        <KeyValue label="Minimum runners">{spec.capacity.min_runners}</KeyValue>
        <KeyValue label="Maximum runners">{spec.capacity.max_runners}</KeyValue>
        <KeyValue label="Template profile">
          <TemplateProfile resolved={fleet.resolved} />
        </KeyValue>
        <KeyValue label={`${provider} authentication`}>
          <Link
            className="text-link"
            to={`/auth?key=${encodeURIComponent(fleet.resolved.authDesired.profileKey)}`}
          >
            {fleet.resolved.authDesired.profileKey} / r{fleet.resolved.authDesired.revision}
          </Link>
        </KeyValue>
        {authContext && <AuthContextValue context={authContext} />}
        <KeyValue label="Incarnation">
          <span className="mono text-xs">{fleet.metadata.incarnation}</span>
        </KeyValue>
      </dl>
      {spec.kind === "gitlab" && (
        <p className="mt-4 text-xs text-muted-foreground">
          Demand comes from a paused, tokenless anchor runner Shaula keeps for this fleet. Each
          runner claims one job with gitlab-runner run-single and the shell executor inside the
          runner container, so .gitlab-ci.yml image: is ignored. Runners are paused after claiming a
          job and deleted once it finishes.
        </p>
      )}
      {fleet.resolved.templatePool?.length ? (
        <PoolMembers members={fleet.resolved.templatePool} />
      ) : null}
    </section>
  );
}

/** Provider-specific identity and routing fields. */
function BackendFacts({ spec }: { spec: FleetSpec }) {
  if (spec.kind === "gitlab")
    return (
      <>
        <KeyValue label="Runner name prefix">{spec.gitlab.runner_name_prefix}</KeyValue>
        <KeyValue label="Tags">
          <div className="flex flex-wrap gap-1">
            <LabelChips labels={spec.gitlab.tags} fallback="--" />
          </div>
        </KeyValue>
        <KeyValue label="Untagged jobs">{spec.gitlab.run_untagged ? "Run" : "Not run"}</KeyValue>
      </>
    );
  if (spec.kind === "forgejo")
    return (
      <>
        <KeyValue label="Runner name prefix">{spec.forgejo.runner_name_prefix}</KeyValue>
        <KeyValue label="Labels">
          <div className="flex flex-wrap gap-1">
            <LabelChips labels={spec.forgejo.labels} fallback="--" />
          </div>
        </KeyValue>
      </>
    );
  return (
    <>
      <KeyValue label="Runner group">{spec.github.runner_group}</KeyValue>
      <KeyValue label="Labels">
        <div className="flex flex-wrap gap-1">
          <LabelChips
            labels={spec.github.labels}
            fallback={`${spec.github.scale_set_name} (default)`}
          />
        </div>
      </KeyValue>
    </>
  );
}

function TemplateProfile({ resolved }: { resolved: Resolved }) {
  if (resolved.templatePool?.length)
    return (
      <div className="form-stack gap-1">
        <span>{resolved.templatePool.length} weighted members</span>
        {resolved.templatePool.map((member) => (
          <span key={member.key} className="text-sm text-muted-foreground">
            {member.key}: {member.template_profile_key} / r{member.template_revision} · weight{" "}
            {member.weight}
          </span>
        ))}
      </div>
    );
  if (resolved.template)
    return (
      <Link
        className="text-link"
        to={`/templates?key=${encodeURIComponent(resolved.template.key)}`}
      >
        {resolved.template.key} / r{resolved.template.revision}
      </Link>
    );
  return "--";
}

function AuthContextValue({ context }: { context: AuthContext }) {
  return (
    <KeyValue label="Auth context (this target)">
      <p>
        {context.state}
        {context.reason && `: ${context.reason}`}
      </p>
      <AuthRouteDetails
        label="Desired route"
        route={context.desiredRoute}
        reference={context.desired}
      />
      <AuthRouteDetails
        label="Observed route"
        route={context.observedRoute}
        reference={context.observed}
      />
      <p className="text-xs text-muted-foreground">
        Observed records the last accepted route. Access is verified again before GitHub operations.
      </p>
    </KeyValue>
  );
}

function LabelChips({ labels, fallback }: { labels: string[]; fallback: string }) {
  if (!labels.length) return fallback;
  return labels.map((label) => (
    <span className="label-chip" key={label}>
      {label}
    </span>
  ));
}

function PoolMembers({ members }: { members: NonNullable<Resolved["templatePool"]> }) {
  return (
    <div className="mt-6">
      <h3 className="font-medium">Template pool members</h3>
      <div className="table-scroll mt-2">
        <Table>
          <TableColumns labels={["Member", "Weight", "Template"]} />
          <TableBody>
            {members.map((member) => (
              <TableRow key={member.key}>
                <TableCell>{member.key}</TableCell>
                <TableCell>{member.weight}</TableCell>
                <TableCell>
                  {member.template_profile_key} / r{member.template_revision}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      <p className="mt-2 text-xs text-muted-foreground">
        Each Runner is assigned by an independent weighted random draw. This does not guarantee
        exact ratios; the table shows the admitted member configuration.
      </p>
    </div>
  );
}
