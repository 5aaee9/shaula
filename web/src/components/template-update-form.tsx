import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowUpFromLine, LoaderCircle } from "lucide-react";
import { api, ApiError, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import { inputEntries } from "@/lib/input-values";
import { useTemplateVariables, type TemplateSource } from "@/lib/template-variables";
import type { Accepted, ChangeRef, TemplateResource, TemplateRevision } from "@/lib/types";
import { isSensitiveMarker } from "@/lib/types";
import { initialUpdateBindings, TemplateUpdateBindings } from "./template-update-bindings";
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
  const [bindings, setBindings] = useState<string>(() =>
    initialUpdateBindings(snapshot.revision.bindings),
  );
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
  const bindingsChanged = useMemo(() => {
    // Compare the desired merged set against the base: a non-secret value that
    // differs, or a sensitive field with a new (non-null) value, is a change.
    let current: Map<string, string>;
    try {
      current = inputEntries(bindings);
    } catch {
      return true; // invalid JSON: allow submit so the server reports it
    }
    const base = snapshot.revision.bindings ?? {};
    const keys = new Set([...Object.keys(base), ...current.keys()]);
    for (const key of keys) {
      const baseValue = base[key];
      if (isSensitiveMarker(baseValue)) {
        if (current.has(key)) return true; // a supplied secret replaces
        continue;
      }
      const next = current.get(key);
      const baseJson = JSON.stringify(baseValue === undefined ? null : baseValue);
      if ((next ?? "null") !== baseJson) return true;
    }
    return false;
  }, [bindings, snapshot.revision.bindings]);
  const upToDate =
    !!target &&
    target.artifactDigest === snapshot.revision.artifactDigest &&
    target.engineRef === snapshot.revision.engineRef &&
    target.key === sourceKey &&
    policy === undefined &&
    !bindingsChanged;

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
      // Omit `bindings` entirely when nothing changed so the request replays
      // the plain reuse-the-base identity (spec 0038: omission = reuse).
      let body = metadata;
      if (bindingsChanged) {
        const current = inputEntries(bindings);
        const base = snapshot.revision.bindings ?? {};
        const out = new Map<string, string>();
        const keys = new Set([...Object.keys(base), ...current.keys()]);
        for (const key of keys) {
          // Sensitive field: new value replaces, otherwise `null` = keep.
          if (isSensitiveMarker(base[key])) {
            out.set(key, current.has(key) ? current.get(key)! : "null");
          } else if (current.has(key)) {
            out.set(key, current.get(key)!);
          } else {
            out.set(key, "null");
          }
        }
        const merged = `{${[...out].map(([k, v]) => `${JSON.stringify(k)}:${v}`).join(",")}}`;
        body = `${body.slice(0, -1)},"bindings":${merged}}`;
      }
      if (policy !== undefined) body = `${body.slice(0, -1)},"fleet_input_policy":${policy}}`;
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

        <TemplateUpdateBindings variables={variables} bindings={bindings} onChange={setBindings} />

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
