import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Plus, ShieldCheck } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { api, resourcePath } from "@/lib/api";
import type { AuthResource, ChangeRef } from "@/lib/types";
import { ForgejoAuthDetails } from "@/components/forgejo-auth-details";
import { GitlabAuthDetails } from "@/components/gitlab-auth-details";
import { canManageAuthProfile } from "@/lib/auth-policy";
import { AuthBindings } from "@/components/auth-bindings";
import { AuthConnections } from "@/components/auth-connections";
import { AuthDetailsActions, AuthProfileFacts } from "@/components/auth-details-sections";
import { Button } from "@/components/ui/button";
import { ErrorNotice, Loading } from "@/components/status";
import { RetireDialog } from "@/components/retire-dialog";
import { ChangeNotice } from "@/components/change-notice";

export function AuthPage({ scopes }: { scopes: string[] }) {
  const [params, setParams] = useSearchParams();
  const key = params.get("key") || "";
  const navigate = useNavigate();
  const location = useLocation();
  const publication = location.state as { change?: ChangeRef } | null;
  const [change, setChange] = useState<ChangeRef | null>(publication?.change ?? null);
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Authentication</h1>
          <p>GitHub, Forgejo and GitLab connections, targets, and validated profile revisions.</p>
        </div>
        <Button disabled={!scopes.includes("auth.write")} onClick={() => navigate("/auth/new")}>
          <Plus />
          Create profile
        </Button>
      </div>
      <ChangeNotice change={change} />
      {!scopes.includes("auth.read") ? (
        <ErrorNotice error={new Error("Authentication profile read permission is required.")} />
      ) : (
        <>
          <AuthConnections selectedKey={key} onSelect={(value) => setParams({ key: value })} />
          {key && <AuthDetails key={key} profileKey={key} scopes={scopes} onAccepted={setChange} />}
        </>
      )}
    </>
  );
}
function AuthDetails({
  profileKey,
  scopes,
  onAccepted,
}: {
  profileKey: string;
  scopes: string[];
  onAccepted: (change: ChangeRef) => void;
}) {
  const path = resourcePath("github-auth-profiles", profileKey);
  const query = useQuery({
    queryKey: ["auth", profileKey],
    queryFn: ({ signal }) => api<AuthResource>(path, { signal }),
    refetchInterval: 10_000,
  });
  const [retire, setRetire] = useState(false);
  if (query.isPending) return <Loading />;
  if (query.error) return <ErrorNotice error={query.error} retry={() => void query.refetch()} />;
  const { data } = query.data;
  const canManage = scopes.includes("auth.write") && canManageAuthProfile(data);
  return (
    <section className="details-section">
      <div className="section-heading">
        <h2 className="flex min-w-0 items-center gap-2 break-all">
          <ShieldCheck className="size-5 shrink-0 text-emerald-600" />
          {profileKey}
        </h2>
        <AuthDetailsActions
          profileKey={profileKey}
          profile={data}
          canManage={canManage}
          canRetire={scopes.includes("auth.retire")}
          onRetire={() => setRetire(true)}
        />
      </div>
      {canManage && data.kind === "github_app" && (
        <p className="mb-5 text-sm text-muted-foreground">
          Re-auth opens GitHub to install this App on more accounts. After returning, edit the
          target policy to enable those targets.
        </p>
      )}
      <AuthProfileFacts profile={data} />
      {data.active?.forgejo && <ForgejoAuthDetails state={data.active.forgejo} />}
      {data.active?.gitlab && <GitlabAuthDetails state={data.active.gitlab} />}
      {data.status === "Unsupported" && (
        <p className="text-sm text-muted-foreground">
          This authentication format is no longer supported. Create a GitHub App profile to use it
          with Fleets.
        </p>
      )}
      {data.active?.schema_version === 2 && <AuthBindings revision={data.active} />}
      {retire && (
        <RetireDialog
          name={profileKey}
          path={path}
          etag={query.data.etag}
          type="profile"
          onClose={() => setRetire(false)}
          onAccepted={onAccepted}
        />
      )}
    </section>
  );
}
