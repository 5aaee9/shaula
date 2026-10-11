import { Link } from "react-router-dom";
import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table";
import type { FleetResource, FleetStatus } from "@/lib/types";
import { fleetTargetName } from "@/lib/runner-backend";
import { KeyValue } from "@/components/status";
import { AuthRouteDetails } from "@/components/auth-route-details";
import { TableColumns } from "@/components/table-columns";

type Resolved = FleetResource["resolved"];
type AuthContext = NonNullable<NonNullable<FleetStatus["githubAuth"]>["context"]>;

/** The fleet's admitted configuration and, when pooled, its member table. */
export function FleetOverview({ fleet, status }: { fleet: FleetResource; status?: FleetStatus }) {
  const { spec } = fleet;
  const forgejo = spec.kind === "forgejo";
  const authContext = spec.kind !== "forgejo" ? status?.githubAuth?.context : undefined;
  return (
    <section className="details-section">
      <h2>Configuration</h2>
      <dl className="details-grid">
        <KeyValue label="Runner backend">{forgejo ? "Forgejo" : "GitHub Actions"}</KeyValue>
        <KeyValue label={forgejo ? "Forgejo target" : "GitHub target"}>
          {fleetTargetName(spec)}
        </KeyValue>
        {spec.kind === "forgejo" ? (
          <KeyValue label="Runner name prefix">{spec.forgejo.runner_name_prefix}</KeyValue>
        ) : (
          <KeyValue label="Runner group">{spec.github.runner_group}</KeyValue>
        )}
        <KeyValue label="Minimum runners">{spec.capacity.min_runners}</KeyValue>
        <KeyValue label="Maximum runners">{spec.capacity.max_runners}</KeyValue>
        <KeyValue label="Template profile">
          <TemplateProfile resolved={fleet.resolved} />
        </KeyValue>
        <KeyValue label={forgejo ? "Forgejo authentication" : "GitHub authentication"}>
          <Link
            className="text-link"
            to={`/auth?key=${encodeURIComponent(fleet.resolved.authDesired.profileKey)}`}
          >
            {fleet.resolved.authDesired.profileKey} / r{fleet.resolved.authDesired.revision}
          </Link>
        </KeyValue>
        {authContext && <AuthContextValue context={authContext} />}
        <KeyValue label="Labels">
          <div className="flex flex-wrap gap-1">
            {spec.kind === "forgejo" ? (
              <LabelChips labels={spec.forgejo.labels} fallback="--" />
            ) : (
              <LabelChips
                labels={spec.github.labels}
                fallback={`${spec.github.scale_set_name} (default)`}
              />
            )}
          </div>
        </KeyValue>
        <KeyValue label="Incarnation">
          <span className="mono text-xs">{fleet.metadata.incarnation}</span>
        </KeyValue>
      </dl>
      {fleet.resolved.templatePool?.length ? (
        <PoolMembers members={fleet.resolved.templatePool} />
      ) : null}
    </section>
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
