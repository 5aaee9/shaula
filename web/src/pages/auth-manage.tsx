import { useState } from "react";
import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import { AuthForm, type AuthFormMode } from "@/components/auth-form";
import { ErrorNotice, Loading } from "@/components/status";
import { api, resourcePath, type Resource } from "@/lib/api";
import { authManageCopy } from "@/lib/auth-manage-copy";
import { canManageAuthProfile } from "@/lib/auth-policy";
import type { AuthResource, ChangeRef } from "@/lib/types";

export function AuthManagePage({ scopes, mode }: { scopes: string[]; mode: AuthFormMode }) {
  const { key: profileKey = "" } = useParams<{ key: string }>();
  const navigate = useNavigate();
  const location = useLocation();
  const [reviewId] = useState(() => crypto.randomUUID());
  const canWrite = scopes.includes("auth.write");
  const canRead = scopes.includes("auth.read");
  const creating = mode === "create";
  const backTo = creating ? "/auth" : `/auth?${new URLSearchParams({ key: profileKey })}`;
  const query = useQuery({
    queryKey: ["auth-management", profileKey, mode, reviewId, location.key],
    enabled: !creating && canRead && canWrite,
    staleTime: "static",
    gcTime: 0,
    queryFn: ({ signal }) =>
      api<AuthResource>(resourcePath("github-auth-profiles", profileKey), { signal }),
  });
  function accepted(change: ChangeRef) {
    navigate(`/auth?${new URLSearchParams({ key: change.resource })}`, {
      replace: true,
      state: { change },
    });
  }
  const { title, description } = authManageCopy(mode, profileKey);
  return (
    <div className="mx-auto w-full max-w-4xl">
      <Link className="back-link" to={backTo}>
        <ArrowLeft className="size-4" aria-hidden="true" />
        Back to authentication
      </Link>
      <div className="page-heading">
        <div>
          <h1 className="break-all">{title}</h1>
          <p>{description}</p>
        </div>
      </div>
      {!canWrite || (!creating && !canRead) ? (
        <ErrorNotice
          error={new Error("Authentication profile read and write permissions are required.")}
        />
      ) : creating ? (
        <AuthForm
          key={location.key}
          mode="create"
          onClose={() => navigate(backTo)}
          onAccepted={accepted}
        />
      ) : (
        <ExistingProfileForm
          query={query}
          mode={mode}
          formKey={location.key}
          onClose={() => navigate(backTo)}
          onAccepted={accepted}
        />
      )}
    </div>
  );
}

/** Policy edit and rotation both require a loaded, manageable Active revision. */
function ExistingProfileForm({
  query,
  mode,
  formKey,
  onClose,
  onAccepted,
}: {
  query: UseQueryResult<Resource<AuthResource>>;
  mode: Exclude<AuthFormMode, "create">;
  formKey: string;
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
}) {
  if (query.isPending) return <Loading />;
  if (query.error) return <ErrorNotice error={query.error} retry={() => void query.refetch()} />;
  if (
    !canManageAuthProfile(query.data.data) ||
    (mode === "policy" && query.data.data.kind !== "github_app")
  )
    return (
      <ErrorNotice
        error={
          new Error(
            "A supported Active revision is required. Target policy editing is only available for GitHub Apps.",
          )
        }
      />
    );
  return (
    <AuthForm
      key={formKey}
      mode={mode}
      resource={query.data}
      onClose={onClose}
      onAccepted={onAccepted}
    />
  );
}
