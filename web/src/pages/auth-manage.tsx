import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import { AuthForm, type AuthFormMode } from "@/components/auth-form";
import { ErrorNotice, Loading } from "@/components/status";
import { api, resourcePath } from "@/lib/api";
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
  const title = creating
    ? "Create authentication profile"
    : mode === "policy"
      ? `Edit target policy for ${profileKey}`
      : `Rotate credential for ${profileKey}`;
  return (
    <div className="mx-auto w-full max-w-4xl">
      <Link className="back-link" to={backTo}>
        <ArrowLeft className="size-4" aria-hidden="true" />
        Back to authentication
      </Link>
      <div className="page-heading">
        <div>
          <h1 className="break-all">{title}</h1>
          <p>
            {creating
              ? "Connect a GitHub App and choose the targets fleets may use."
              : mode === "policy"
                ? "Choose allowed targets using the credential from the current Active revision."
                : "Replace this App's private key while retaining the current Active target policy."}
          </p>
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
      ) : query.isPending ? (
        <Loading />
      ) : query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : !canManageAuthProfile(query.data.data) ? (
        <ErrorNotice
          error={
            new Error(
              "A supported Active GitHub App revision is required to manage this connection.",
            )
          }
        />
      ) : (
        <AuthForm
          key={location.key}
          mode={mode}
          resource={query.data}
          onClose={() => navigate(backTo)}
          onAccepted={accepted}
        />
      )}
    </div>
  );
}
