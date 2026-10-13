import type { ReactNode } from "react";
import { ArrowLeft } from "lucide-react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { ErrorNotice, Loading } from "@/components/status";
import { FleetForm } from "@/components/fleet-form";
import { api, resourcePath, type Resource } from "@/lib/api";
import type { ChangeRef, FleetResource } from "@/lib/types";

/** Shows the form only once permissions allow it and, when editing, the fleet has loaded. */
function FleetFormGate({
  editing,
  canRead,
  canWrite,
  query,
  children,
}: {
  editing: boolean;
  canRead: boolean;
  canWrite: boolean;
  query: UseQueryResult<Resource<FleetResource>>;
  children: ReactNode;
}) {
  if (!canWrite) return <ErrorNotice error={new Error("Fleet write permission is required.")} />;
  if (!editing) return children;
  if (!canRead)
    return <ErrorNotice error={new Error("Fleet read permission is required to edit a fleet.")} />;
  if (query.isPending) return <Loading />;
  if (query.error) return <ErrorNotice error={query.error} retry={() => void query.refetch()} />;
  return children;
}

export function FleetManagePage({ scopes, mode }: { scopes: string[]; mode: "create" | "edit" }) {
  const { key } = useParams<{ key: string }>();
  const navigate = useNavigate();
  const location = useLocation();
  const editing = mode === "edit";
  const canRead = scopes.includes("fleet.read");
  const query = useQuery({
    queryKey: ["fleet", key],
    enabled: editing && !!key && canRead,
    queryFn: ({ signal }) => api<FleetResource>(resourcePath("fleets", key!), { signal }),
    refetchInterval: 10_000,
  });
  const backTo = editing && key ? `/fleets/${encodeURIComponent(key)}` : "/fleets";
  function accepted(change: ChangeRef) {
    navigate(backTo, { replace: true, state: { change } });
  }
  return (
    <div className="mx-auto w-full max-w-5xl">
      <Link className="back-link" to={backTo}>
        <ArrowLeft className="size-4" aria-hidden="true" />
        Back to fleets
      </Link>
      <div className="page-heading">
        <div>
          <h1>{editing ? `Edit ${key}` : "New fleet"}</h1>
          <p>
            {editing
              ? "Update capacity, authentication, and template placement."
              : "A fleet manages short-lived runners for GitHub Actions, Forgejo or GitLab."}
          </p>
        </div>
      </div>
      <FleetFormGate
        editing={editing}
        canRead={canRead}
        canWrite={scopes.includes("fleet.write")}
        query={query}
      >
        <FleetForm
          key={location.key}
          resource={editing ? query.data : undefined}
          scopes={scopes}
          page
          onClose={() => navigate(backTo, { replace: true })}
          onAccepted={accepted}
        />
      </FleetFormGate>
    </div>
  );
}
