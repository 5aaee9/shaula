import { ArrowUpFromLine, LoaderCircle } from "lucide-react";
import { ApiError } from "@/lib/api";
import type { TemplateSource, TemplateVariables } from "@/lib/template-variables";
import type { TemplateResource, TemplateRevision } from "@/lib/types";
import { ErrorNotice, Field, KeyValue, Loading } from "./status";
import { Button } from "./ui/button";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

function DefaultTemplatePicker({
  sourceKey,
  target,
  candidates,
  onSelect,
}: {
  sourceKey?: string | null;
  target?: TemplateSource;
  candidates: TemplateSource[];
  onSelect: (key: string) => void;
}) {
  if (sourceKey)
    return target ? (
      <dl className="text-sm">
        <KeyValue label="Default template">{sourceKey}</KeyValue>
      </dl>
    ) : (
      <p role="status" className="text-sm text-muted-foreground">
        Saved default template {sourceKey} is unavailable for this platform. Use New revision to
        choose another source.
      </p>
    );
  if (!candidates.length)
    return (
      <p role="status" className="text-sm text-muted-foreground">
        No defaults are available for this platform. Use New revision to publish your own template.
      </p>
    );
  return (
    <Field label="Default template">
      <NativeSelect
        required
        value={target?.key || ""}
        onChange={(event) => onSelect(event.target.value)}
      >
        <NativeSelectOption value="">Choose a default template</NativeSelectOption>
        {candidates.map((source) => (
          <NativeSelectOption key={source.key} value={source.key}>
            {source.key}
          </NativeSelectOption>
        ))}
      </NativeSelect>
    </Field>
  );
}

export function UpdateSourceSection({
  sourceKey,
  target,
  candidates,
  platform,
  onSelect,
}: {
  sourceKey?: string | null;
  target?: TemplateSource;
  candidates: TemplateSource[];
  platform: string | null;
  onSelect: (key: string) => void;
}) {
  return (
    <section className="space-y-4 rounded-xl border bg-card p-5 sm:p-6" aria-label="Update source">
      <div>
        <h2>Update from default</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {target
            ? "Review the selected default template."
            : sourceKey
              ? "The saved default template must be available to update."
              : `Choose a default for the ${platform || "current"} platform.`}{" "}
          This replaces the template code; review it before updating a customized template.
        </p>
      </div>
      <DefaultTemplatePicker
        sourceKey={sourceKey}
        target={target}
        candidates={candidates}
        onSelect={onSelect}
      />
    </section>
  );
}

export function RevisionComparison({
  resource,
  revision,
  target,
  upToDate,
}: {
  resource: TemplateResource;
  revision: TemplateRevision;
  target?: TemplateSource;
  upToDate: boolean;
}) {
  return (
    <div className="grid min-w-0 gap-4 md:grid-cols-2">
      <section
        className="min-w-0 rounded-xl border bg-card p-5 sm:p-6"
        aria-label="Current revision"
      >
        <h2>Current revision r{resource.desiredRevision}</h2>
        <dl className="mt-4 space-y-4 text-sm">
          <KeyValue label="Artifact">
            <span className="break-all font-mono text-xs">{revision.artifactDigest}</span>
          </KeyValue>
          <KeyValue label="Engine">{revision.engineRef}</KeyValue>
        </dl>
      </section>
      <section
        className="min-w-0 rounded-xl border bg-card p-5 sm:p-6"
        aria-label="Target revision"
      >
        <h2>Target revision</h2>
        {target ? (
          <dl className="mt-4 space-y-4 text-sm">
            <KeyValue label="Default template">{target.key}</KeyValue>
            <KeyValue label="Artifact">
              <span className="break-all font-mono text-xs">{target.artifactDigest}</span>
            </KeyValue>
            <KeyValue label="Engine">{target.engineRef}</KeyValue>
          </dl>
        ) : (
          <p className="mt-4 text-sm text-muted-foreground">
            {revision.sourceKey
              ? "The saved default template is unavailable."
              : "Choose a default to review its contents."}
          </p>
        )}
        {upToDate && (
          <p role="status" className="mt-4 text-sm font-medium">
            Up to date. This revision already uses the selected default, artifact and engine.
          </p>
        )}
      </section>
    </div>
  );
}

export function TargetDiscoveryStatus({
  loading,
  error,
  variables,
  retry,
}: {
  loading: boolean;
  error: unknown;
  variables: TemplateVariables | null;
  retry: () => void;
}) {
  return (
    <>
      {loading && <Loading />}
      {error !== null && <ErrorNotice error={error} retry={retry} />}
      {variables && !variables.available && (
        <p className="text-sm text-muted-foreground">
          {variables.reason || "Variable discovery is unavailable."} The current policy can still be
          retained.
        </p>
      )}
    </>
  );
}

export function UpdateActionBar({
  busy,
  conflict,
  error,
  submitDisabled,
  onClose,
}: {
  busy: boolean;
  conflict: boolean;
  error: unknown;
  submitDisabled: boolean;
  onClose: () => void;
}) {
  return (
    <div className="sticky bottom-0 z-10 space-y-3 border-t bg-background/95 py-4 backdrop-blur-sm">
      {error !== null && (
        <ErrorNotice
          error={
            error instanceof ApiError && error.status === 412
              ? new Error("This template changed after the review loaded.")
              : error
          }
        />
      )}
      {conflict && (
        <p role="status" className="text-sm">
          Your draft is retained. Return to templates and start a new review to load the current
          version before submitting again.
        </p>
      )}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="max-w-lg text-xs text-muted-foreground">
          The new revision activates after validation. Fleets follow the new Active revision
          automatically once their runners drain (spec 0023); no per-fleet update is needed.
        </p>
        <div className="flex gap-2">
          <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={submitDisabled}>
            {busy ? <LoaderCircle className="animate-spin" /> : <ArrowUpFromLine />}
            {busy ? "Updating…" : "Update"}
          </Button>
        </div>
      </div>
    </div>
  );
}
