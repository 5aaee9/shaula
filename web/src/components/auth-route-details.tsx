import type { AuthRoute } from "@/lib/types";
import { targetName } from "@/lib/types";

export function AuthRouteDetails({
  label,
  route,
  reference,
}: {
  label: string;
  route?: AuthRoute | null;
  reference: { profileKey: string; revision: number } | null;
}) {
  const matches =
    route &&
    reference &&
    route.profileKey === reference.profileKey &&
    route.revision === reference.revision;
  return (
    <div className="mt-2 text-xs" aria-label={label}>
      <strong>{label}</strong>
      <p>{reference ? `${reference.profileKey} / r${reference.revision}` : "No revision"}</p>
      {matches ? (
        <>
          <p>
            {route.githubHost} · App #{route.appId}
          </p>
          <p>
            {route.account.login} ({route.account.kind}) #{route.account.id} · installation #
            {route.installationId}
          </p>
          <p>
            {targetName(route.target)} ({route.target.kind})
          </p>
          {route.target.kind === "repository" ? (
            <p>
              Repository #{route.repositoryId ?? "unresolved"} · owner #
              {route.repositoryOwnerId ?? "unresolved"}
            </p>
          ) : (
            <p>Organization #{route.organizationId ?? "unresolved"}</p>
          )}
        </>
      ) : (
        <p className="text-muted-foreground">Route metadata unavailable</p>
      )}
    </div>
  );
}
