import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ArrowLeft, Trash2, Upload } from "lucide-react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { api, resourcePath } from "@/lib/api";
import { isSensitiveMarker, type TemplateResource, type TemplateRevision } from "@/lib/types";
import { ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RetireDialog } from "@/components/retire-dialog";

function BindingValue({ value }: { value: unknown }) {
  if (isSensitiveMarker(value)) {
    return (
      <span className="text-sm text-muted-foreground">
        {value.set ? "Configured" : "Not set"} (secret)
      </span>
    );
  }
  return <span className="mono break-all text-xs">{JSON.stringify(value)}</span>;
}

export function TemplateDetailPage({ scopes }: { scopes: string[] }) {
  const { key: profileKey = "" } = useParams<{ key: string }>();
  const navigate = useNavigate();
  const canRead = scopes.includes("template.read");
  const canPublish = scopes.includes("template.publish");
  const canRetire = scopes.includes("template.retire");
  const path = resourcePath("template-profiles", profileKey);
  const backTo = `/templates?${new URLSearchParams({ key: profileKey })}`;

  const query = useQuery({
    queryKey: ["template", profileKey],
    enabled: !!profileKey && canRead,
    queryFn: ({ signal }) => api<TemplateResource>(path, { signal }),
    refetchInterval: 10000,
  });
  const [selected, setSelected] = useState("");
  const desired = query.data?.data.desiredRevision;
  const revision = selected || (desired ? String(desired) : "");
  const revisionQuery = useQuery({
    queryKey: ["template-revision", profileKey, revision],
    enabled: !!revision && canRead,
    queryFn: ({ signal }) =>
      api<TemplateRevision>(`${path}/revisions/${encodeURIComponent(revision)}`, { signal }),
    refetchInterval: 10000,
  });
  const [retire, setRetire] = useState(false);

  if (!canRead) return <ErrorNotice error={new Error("Template read permission is required.")} />;
  if (query.isPending) return <Loading />;
  if (query.error) return <ErrorNotice error={query.error} retry={() => void query.refetch()} />;
  const { data } = query.data;
  const retired = ["Retiring", "Retired"].includes(data.status);
  const bindings = revisionQuery.data?.data.bindings ?? {};

  return (
    <div className="mx-auto w-full max-w-5xl">
      <Link className="back-link" to={backTo}>
        <ArrowLeft className="size-4" aria-hidden="true" />
        Back to templates
      </Link>
      <div className="page-heading">
        <div>
          <h1 className="break-all">{profileKey}</h1>
          <p>Template profile detail and published revisions.</p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={!canPublish || retired}
            onClick={() => navigate(`/templates/${encodeURIComponent(profileKey)}/update`)}
          >
            Update
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={!canPublish || retired}
            onClick={() => navigate(`/templates/${encodeURIComponent(profileKey)}/revisions/new`)}
          >
            <Upload />
            New revision
          </Button>
          <Button
            variant="outline"
            size="icon"
            disabled={!canRetire || retired}
            aria-label="Retire template"
            onClick={() => setRetire(true)}
          >
            <Trash2 />
          </Button>
        </div>
      </div>

      <section className="rounded-xl border bg-card p-5 sm:p-6">
        <dl className="details-grid">
          <KeyValue label="Platform">{data.platform || "--"}</KeyValue>
          <KeyValue label="Bindings contract">{data.bindingsContract || "--"}</KeyValue>
          <KeyValue label="Status">
            <StatusBadge value={data.status} />
          </KeyValue>
          <KeyValue label="Active revision">
            {data.activeRevision ? `r${data.activeRevision}` : "--"}
          </KeyValue>
          <KeyValue label="Desired revision">r{data.desiredRevision}</KeyValue>
        </dl>
        {data.status === "Ready" && (
          <p className="mt-4 text-sm text-muted-foreground" role="status">
            Validation passed. Waiting for automatic activation.
          </p>
        )}
      </section>

      <section className="mt-6 rounded-xl border bg-card p-5 sm:p-6">
        <div className="flex items-center gap-3">
          <h2 className="text-sm font-medium">Revision</h2>
          <Input
            className="w-24"
            aria-label="Template revision"
            type="number"
            min={1}
            step={1}
            value={revision}
            onChange={(event) => setSelected(event.target.value)}
          />
        </div>
        {revisionQuery.error ? (
          <ErrorNotice error={revisionQuery.error} />
        ) : revisionQuery.data ? (
          <>
            <dl className="details-grid mt-4">
              <KeyValue label="Artifact digest">
                <span className="mono break-all text-xs">
                  {revisionQuery.data.data.artifactDigest}
                </span>
              </KeyValue>
              <KeyValue label="Engine">{revisionQuery.data.data.engineRef}</KeyValue>
              <KeyValue label="Revision status">
                <StatusBadge value={revisionQuery.data.data.state} />
              </KeyValue>
              {revisionQuery.data.data.sourceKey && (
                <KeyValue label="Default source">{revisionQuery.data.data.sourceKey}</KeyValue>
              )}
              {revisionQuery.data.data.reason && (
                <KeyValue label="Validation reason">{revisionQuery.data.data.reason}</KeyValue>
              )}
            </dl>

            <div className="mt-6">
              <h3 className="text-sm font-medium">Bindings</h3>
              <p className="mt-1 text-sm text-muted-foreground">
                Non-secret values are shown; secrets are never returned.
              </p>
              {Object.keys(bindings).length ? (
                <dl className="details-grid mt-4">
                  {Object.entries(bindings).map(([name, value]) => (
                    <KeyValue key={name} label={name}>
                      <BindingValue value={value} />
                    </KeyValue>
                  ))}
                </dl>
              ) : (
                <p className="mt-3 text-sm text-muted-foreground">
                  {revisionQuery.data.data ? "No bindings configured." : ""}
                </p>
              )}
            </div>
          </>
        ) : (
          <Loading />
        )}
      </section>

      {retire && (
        <RetireDialog
          name={profileKey}
          path={path}
          etag={query.data.etag}
          type="profile"
          onClose={() => setRetire(false)}
          onAccepted={() => navigate("/templates", { replace: true })}
        />
      )}
    </div>
  );
}
