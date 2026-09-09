import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Plus, RotateCw, ShieldCheck, SlidersHorizontal, Trash2 } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { api, resourcePath } from "@/lib/api";
import type { AuthResource, ChangeRef } from "@/lib/types";
import { selectorLabel } from "@/lib/types";
import { canManageAuthProfile, selectorKey } from "@/lib/auth-policy";
import { AuthBindings } from "@/components/auth-bindings";
import { AuthConnections } from "@/components/auth-connections";
import { Button } from "@/components/ui/button";
import { ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import { RetireDialog } from "@/components/retire-dialog";
import { ChangeNotice } from "@/components/change-notice";
import { AuthReauth } from "@/components/auth-reauth";

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
          <p>GitHub connections, target policies, and validated profile revisions.</p>
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
  const navigate = useNavigate();
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
        <div className="flex min-w-0 flex-wrap gap-2">
          {canManage && (
            <>
              <AuthReauth profile={data} />
              <Button
                size="sm"
                variant="outline"
                onClick={() => navigate(`/auth/${encodeURIComponent(profileKey)}/targets/edit`)}
              >
                <SlidersHorizontal />
                Edit target policy
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => navigate(`/auth/${encodeURIComponent(profileKey)}/rotate`)}
              >
                <RotateCw />
                Rotate credential
              </Button>
            </>
          )}
          <Button
            variant="outline"
            size="icon"
            aria-label="Retire authentication profile"
            disabled={!scopes.includes("auth.retire")}
            onClick={() => setRetire(true)}
          >
            <Trash2 />
          </Button>
        </div>
      </div>
      {canManage && (
        <p className="mb-5 text-sm text-muted-foreground">
          Re-auth opens GitHub to install this App on more accounts. After returning, edit the
          target policy to enable those targets.
        </p>
      )}
      <dl className="details-grid">
        <KeyValue label="Status">
          <StatusBadge value={data.status} />
        </KeyValue>
        <KeyValue label="Credential type">
          {data.kind === "github_app"
            ? "GitHub App"
            : data.kind === "pat"
              ? "Personal access token"
              : "--"}
        </KeyValue>
        {data.schema_version === 2 && (
          <KeyValue label="App ID">{data.active?.app_id || data.app_id || "--"}</KeyValue>
        )}
        <KeyValue label="Credential">{data.credential_present ? "Configured" : "Absent"}</KeyValue>
        <KeyValue label="Active revision">
          {data.activeRevision ? `r${data.activeRevision}` : "--"}
        </KeyValue>
        <KeyValue label="Desired revision">r{data.desiredRevision}</KeyValue>
        {data.active?.schema_version === 2 && (
          <KeyValue label="Active target policy">
            {data.active.target_policy?.length ? (
              <div className="flex flex-wrap gap-2">
                {data.active.target_policy.map((selector) => (
                  <span className="label-chip" key={selectorKey(selector)}>
                    {selectorLabel(selector)}
                  </span>
                ))}
              </div>
            ) : (
              "--"
            )}
          </KeyValue>
        )}
        {data.desired && (
          <KeyValue label={`Candidate r${data.desired.revision}: ${data.desired.state}`}>
            {data.desired.reason && <p className="text-red-600">{data.desired.reason}</p>}
            {data.desired.target_policy?.length ? (
              <div className="flex flex-wrap gap-2">
                {data.desired.target_policy.map((selector) => (
                  <span className="label-chip" key={selectorKey(selector)}>
                    {selectorLabel(selector)}
                  </span>
                ))}
              </div>
            ) : (
              "--"
            )}
          </KeyValue>
        )}
      </dl>
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
