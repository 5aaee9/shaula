import { ArrowLeft } from "lucide-react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ErrorNotice, Loading } from "@/components/status";
import { FleetForm } from "@/components/fleet-form";
import { api, resourcePath } from "@/lib/api";
import type { ChangeRef, FleetResource } from "@/lib/types";

export function FleetManagePage({ scopes, mode }: { scopes: string[]; mode: "create" | "edit" }) {
  const { key } = useParams<{ key: string }>();
  const navigate = useNavigate();
  const location = useLocation();
  const editing = mode === "edit";
  const canRead = scopes.includes("fleet.read");
  const canWrite = scopes.includes("fleet.write");
  const query = useQuery({
    queryKey: ["fleet", key],
    enabled: editing && !!key && canRead,
    queryFn: ({ signal }) => api<FleetResource>(resourcePath("fleets", key!), { signal }),
    refetchInterval: 10_000,
  });
  const backTo = editing && key ? `/fleets/${encodeURIComponent(key)}` : "/fleets";
  function accepted(change: ChangeRef) {
    navigate(editing && key ? `/fleets/${encodeURIComponent(key)}` : "/fleets", {
      replace: true,
      state: { change },
    });
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
              : "A fleet manages a GitHub Actions scale set — the runners a label can draw from."}
          </p>
        </div>
      </div>
      {!canWrite ? (
        <ErrorNotice error={new Error("Fleet write permission is required.")} />
      ) : editing && !canRead ? (
        <ErrorNotice error={new Error("Fleet read permission is required to edit a fleet.")} />
      ) : editing && query.isPending ? (
        <Loading />
      ) : editing && query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : (
        <FleetForm
          key={location.key}
          resource={editing ? query.data : undefined}
          scopes={scopes}
          page
          onClose={() => navigate(backTo, { replace: true })}
          onAccepted={accepted}
        />
      )}
    </div>
  );
}
