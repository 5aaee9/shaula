import { useEffect, useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowUpFromLine, LoaderCircle } from "lucide-react";
import { api, ApiError, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import { useTemplateVariables, type TemplateSource } from "@/lib/template-variables";
import type { Accepted, ChangeRef, TemplateResource, TemplateRevision } from "@/lib/types";
import { TemplateUpdatePolicy } from "./template-update-policy";
import { ErrorNotice, Field, KeyValue, Loading } from "./status";
import { Button } from "./ui/button";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

export function TemplateUpdateForm({
  resource,
  revision,
  sources,
  onClose,
  onAccepted,
}: {
  resource: Resource<TemplateResource>;
  revision: TemplateRevision;
  sources: TemplateSource[];
  onClose: () => void;
  onAccepted: (change: ChangeRef | null) => void;
}) {
  const [snapshot] = useState(() => ({
    resource,
    revision,
    candidates: sources.filter(
      (source) => source.platform === (revision.platform || resource.data.platform),
    ),
  }));
  const client = useQueryClient();
  const { candidates } = snapshot;
  const sourceKey = snapshot.revision.sourceKey;
  const [target, setTarget] = useState<TemplateSource | undefined>(() => {
    if (sourceKey) return candidates.find((source) => source.key === sourceKey);
    const matching = candidates.filter(
      (source) =>
        source.artifactDigest === snapshot.revision.artifactDigest &&
        source.engineRef === snapshot.revision.engineRef,
    );
    return matching.length === 1
      ? matching[0]
      : candidates.length === 1
        ? candidates[0]
        : undefined;
  });
  const [policy, setPolicy] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const discovery = useTemplateVariables(target?.artifactDigest || "", null, "default", true);
  const variables =
    discovery.variables?.artifactDigest === target?.artifactDigest ? discovery.variables : null;
  const platform = snapshot.revision.platform || snapshot.resource.data.platform;
  const upToDate =
    !!target &&
    target.artifactDigest === snapshot.revision.artifactDigest &&
    target.engineRef === snapshot.revision.engineRef &&
    target.key === sourceKey &&
    policy === undefined;

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!target || upToDate || busy || conflict) return;
    setBusy(true);
    setError(null);
    try {
      const path = `${resourcePath("template-profiles", snapshot.resource.data.key)}/updates`;
      const metadata = JSON.stringify({
        artifact_digest: target.artifactDigest,
        engine_ref: target.engineRef,
        source_key: target.key,
      });
      const body =
        policy === undefined
          ? metadata
          : `${metadata.slice(0, -1)},"fleet_input_policy":${policy}}`;
      const { data } = await api<Accepted>(path, {
        method: "POST",
        body,
        headers: attempt.current.headers("POST", path, body, snapshot.resource.etag, false),
      });
      void client.invalidateQueries();
      if (mounted.current)
        onAccepted(
          data.noOp
            ? null
            : { id: data.changeId, resource: snapshot.resource.data.key, type: "profile" },
        );
    } catch (error) {
      if (mounted.current) {
        setConflict(error instanceof ApiError && [409, 412, 422].includes(error.status));
        setError(error);
      }
    } finally {
      if (mounted.current) setBusy(false);
    }
  }

  return (
    <form aria-label="Template update review" className="min-w-0 space-y-6" onSubmit={submit}>
      <fieldset disabled={busy} className="min-w-0 space-y-6">
        <section
          className="space-y-4 rounded-xl border bg-card p-5 sm:p-6"
          aria-label="Update source"
        >
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
          {sourceKey ? (
            target ? (
              <dl className="text-sm">
                <KeyValue label="Default template">{sourceKey}</KeyValue>
              </dl>
            ) : (
              <p role="status" className="text-sm text-muted-foreground">
                Saved default template {sourceKey} is unavailable for this platform. Use New
                revision to choose another source.
              </p>
            )
          ) : !candidates.length ? (
            <p role="status" className="text-sm text-muted-foreground">
              No defaults are available for this platform. Use New revision to publish your own
              template.
            </p>
          ) : (
            <Field label="Default template">
              <NativeSelect
                required
                value={target?.key || ""}
                onChange={(event) => {
                  setTarget(candidates.find((source) => source.key === event.target.value));
                  setPolicy(undefined);
                }}
              >
                <NativeSelectOption value="">Choose a default template</NativeSelectOption>
                {candidates.map((source) => (
                  <NativeSelectOption key={source.key} value={source.key}>
                    {source.key}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </Field>
          )}
        </section>

        <div className="grid min-w-0 gap-4 md:grid-cols-2">
          <section
            className="min-w-0 rounded-xl border bg-card p-5 sm:p-6"
            aria-label="Current revision"
          >
            <h2>Current revision r{snapshot.resource.data.desiredRevision}</h2>
            <dl className="mt-4 space-y-4 text-sm">
              <KeyValue label="Artifact">
                <span className="break-all font-mono text-xs">
                  {snapshot.revision.artifactDigest}
                </span>
              </KeyValue>
              <KeyValue label="Engine">{snapshot.revision.engineRef}</KeyValue>
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
                {sourceKey
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

        <section
          className="space-y-2 rounded-xl border bg-card p-5 sm:p-6"
          aria-label="Retained bindings"
        >
          <h2>Bindings retained</h2>
          <p className="text-sm text-muted-foreground">
            Shaula reuses the bindings from revision r{snapshot.resource.data.desiredRevision} on
            the server. Secret values are never loaded into this form.
          </p>
          <p className="text-sm text-muted-foreground">
            If the target requires different bindings, use New revision to configure them.
          </p>
        </section>

        {target && (
          <>
            {discovery.loading && <Loading />}
            {discovery.error !== null && (
              <ErrorNotice error={discovery.error} retry={() => void discovery.inspect()} />
            )}
            {variables && !variables.available && (
              <p className="text-sm text-muted-foreground">
                {variables.reason || "Variable discovery is unavailable."} The current policy can
                still be retained.
              </p>
            )}
            <TemplateUpdatePolicy variables={variables} policy={policy} onChange={setPolicy} />
          </>
        )}
      </fieldset>

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
            <Button
              type="submit"
              disabled={busy || !target || upToDate || conflict || discovery.loading}
            >
              {busy ? <LoaderCircle className="animate-spin" /> : <ArrowUpFromLine />}
              {busy ? "Updating…" : "Update"}
            </Button>
          </div>
        </div>
      </div>
    </form>
  );
}
