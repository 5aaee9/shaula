import { Trash2, Upload } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { isSensitiveMarker, type TemplateResource, type TemplateRevision } from "@/lib/types";
import { ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

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

export function TemplateDetailActions({
  profileKey,
  canPublish,
  canRetire,
  retired,
  onRetire,
}: {
  profileKey: string;
  canPublish: boolean;
  canRetire: boolean;
  retired: boolean;
  onRetire: () => void;
}) {
  const navigate = useNavigate();
  return (
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
        onClick={onRetire}
      >
        <Trash2 />
      </Button>
    </div>
  );
}

export function TemplateProfileSummary({ data }: { data: TemplateResource }) {
  return (
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
  );
}

function RevisionBindings({ revision }: { revision: TemplateRevision }) {
  const bindings = revision.bindings ?? {};
  return (
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
        <p className="mt-3 text-sm text-muted-foreground">No bindings configured.</p>
      )}
    </div>
  );
}

function RevisionDetails({ revision }: { revision: TemplateRevision }) {
  return (
    <>
      <dl className="details-grid mt-4">
        <KeyValue label="Artifact digest">
          <span className="mono break-all text-xs">{revision.artifactDigest}</span>
        </KeyValue>
        <KeyValue label="Engine">{revision.engineRef}</KeyValue>
        <KeyValue label="Revision status">
          <StatusBadge value={revision.state} />
        </KeyValue>
        {revision.sourceKey && <KeyValue label="Default source">{revision.sourceKey}</KeyValue>}
        {revision.reason && <KeyValue label="Validation reason">{revision.reason}</KeyValue>}
      </dl>
      <RevisionBindings revision={revision} />
    </>
  );
}

export function TemplateRevisionSection({
  revision,
  onRevisionChange,
  error,
  data,
}: {
  revision: string;
  onRevisionChange: (value: string) => void;
  error: Error | null;
  data: TemplateRevision | undefined;
}) {
  return (
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
          onChange={(event) => onRevisionChange(event.target.value)}
        />
      </div>
      {error ? (
        <ErrorNotice error={error} />
      ) : data ? (
        <RevisionDetails revision={data} />
      ) : (
        <Loading />
      )}
    </section>
  );
}
