import { useState, type FormEvent } from "react";
import { useQuery } from "@tanstack/react-query";
import { Plus, RotateCw, Search, ShieldCheck, Trash2 } from "lucide-react";
import { useSearchParams } from "react-router-dom";
import { api, resourcePath } from "@/lib/api";
import type { AuthResource, ChangeRef } from "@/lib/types";
import { selectorLabel } from "@/lib/types";
import { selectorKey } from "@/lib/auth-policy";
import { AuthBindings } from "@/components/auth-bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Empty, ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import { RetireDialog } from "@/components/retire-dialog";
import { ChangeNotice } from "@/components/change-notice";
import { AuthForm } from "@/components/auth-form";

export function AuthPage({ scopes }: { scopes: string[] }) {
  const [params, setParams] = useSearchParams();
  const key = params.get("key") || "";
  const [search, setSearch] = useState(key);
  const [create, setCreate] = useState(false);
  const [change, setChange] = useState<ChangeRef | null>(null);
  function lookup(event: FormEvent) {
    event.preventDefault();
    setParams({ key: search.trim() });
  }
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Authentication</h1>
          <p>Target-bound credentials and validated profile revisions.</p>
        </div>
        <Button disabled={!scopes.includes("auth.write")} onClick={() => setCreate(true)}>
          <Plus />
          Create profile
        </Button>
      </div>
      <ChangeNotice change={change} />
      {!scopes.includes("auth.read") ? (
        <ErrorNotice error={new Error("Authentication profile read permission is required.")} />
      ) : (
        <>
          <form className="toolbar" onSubmit={lookup}>
            <div className="search-input">
              <Search />
              <Input
                className="pl-9"
                required
                aria-label="Authentication profile key"
                placeholder="Profile key"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            </div>
            <Button variant="outline" type="submit">
              Open profile
            </Button>
          </form>
          {key ? (
            <AuthDetails key={key} profileKey={key} scopes={scopes} onAccepted={setChange} />
          ) : (
            <Empty title="No authentication profile selected" />
          )}
        </>
      )}
      {create && (
        <AuthForm
          onClose={() => setCreate(false)}
          onAccepted={(value) => {
            setChange(value);
            setParams({ key: value.resource });
            setSearch(value.resource);
          }}
        />
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
  const [rotate, setRotate] = useState(false);
  if (query.isPending) return <Loading />;
  if (query.error) return <ErrorNotice error={query.error} retry={() => void query.refetch()} />;
  const { data } = query.data;
  return (
    <section className="details-section">
      <div className="section-heading">
        <h2 className="flex min-w-0 items-center gap-2 break-all">
          <ShieldCheck className="size-5 shrink-0 text-emerald-600" />
          {profileKey}
        </h2>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={!scopes.includes("auth.write")}
            onClick={() => setRotate(true)}
          >
            <RotateCw />
            Rotate credential
          </Button>
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
        {data.active?.schema_version === 2 ||
        (!data.activeRevision && data.schema_version === 2) ? (
          <KeyValue label="App ID">{data.active?.app_id || data.app_id || "--"}</KeyValue>
        ) : (
          <KeyValue label="Identity">{data.active?.identity || data.identity || "--"}</KeyValue>
        )}
        <KeyValue label="Credential">{data.credential_present ? "Configured" : "Absent"}</KeyValue>
        <KeyValue label="Active revision">
          {data.activeRevision ? `r${data.activeRevision}` : "--"}
        </KeyValue>
        <KeyValue label="Desired revision">r{data.desiredRevision}</KeyValue>
        {data.active?.schema_version === 2 || data.active?.target_policy ? (
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
        ) : (
          <KeyValue label="Allowed targets">
            <div className="flex flex-wrap gap-2">
              {(data.active?.target_allowlist || data.target_allowlist || []).map((target) => (
                <span className="label-chip" key={target}>
                  {target}
                </span>
              ))}
            </div>
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
            ) : data.desired.target_allowlist?.length ? (
              <div className="flex flex-wrap gap-2">
                {data.desired.target_allowlist.map((target) => (
                  <span className="label-chip" key={target}>
                    {target}
                  </span>
                ))}
              </div>
            ) : (
              "--"
            )}
          </KeyValue>
        )}
      </dl>
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
      {rotate && (
        <AuthForm resource={query.data} onClose={() => setRotate(false)} onAccepted={onAccepted} />
      )}
    </section>
  );
}
