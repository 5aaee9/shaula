import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowRight, CheckCheck, FileSearch, Layers3, LoaderCircle, Upload } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import type { Accepted, ChangeRef, TemplateResource } from "@/lib/types";
import { inputEntries } from "@/lib/input-values";
import { useTemplateVariables, type TemplateSource } from "@/lib/template-variables";
import { AdvancedSettings } from "./advanced-settings";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";
import { ErrorNotice, Field } from "./status";
import { TemplateVariables } from "./template-variables";
import { TemplateSourceFields, type TemplateSourceKind } from "./template-source-fields";

export function TemplateForm({
  resource,
  initialSource,
  scopes,
  onClose,
  onAccepted,
}: {
  resource?: Resource<TemplateResource>;
  initialSource?: TemplateSource;
  scopes: string[];
  onClose: () => void;
  onAccepted: (change: ChangeRef | null) => void;
}) {
  const id = useId();
  const [snapshot] = useState(resource);
  const client = useQueryClient();
  const [key, setKey] = useState(snapshot?.data.key || "");
  const [digest, setDigest] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [source, setSource] = useState<TemplateSourceKind>(initialSource ? "default" : "archive");
  const [selectedSource, setSelectedSource] = useState(initialSource);
  const [advanced, setAdvanced] = useState(false);
  const [engine, setEngine] = useState(initialSource?.engineRef || "terraform");
  const [bindings, setBindings] = useState("{}");
  const [policy, setPolicy] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const canRead = scopes.includes("template.read");
  const artifactDigest = source === "default" ? selectedSource?.artifactDigest || "" : digest;
  const engineRef = source === "default" ? selectedSource?.engineRef || "" : engine;
  const sourceReady = source === "archive" ? !!file : /^sha256:[a-f0-9]{64}$/.test(artifactDigest);
  const discovery = useTemplateVariables(artifactDigest, file, source, canRead);

  function parseSettings(value: string, label: string) {
    try {
      inputEntries(value);
      return value;
    } catch {
      setAdvanced(true);
      throw new Error(`${label} must be a JSON object.`);
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const parsedBindings = parseSettings(bindings, "Bindings");
      const parsedPolicy = parseSettings(policy, "Fleet input policy");
      const artifactDigest = await discovery.artifact();
      const path = resourcePath("template-profiles", key);
      const metadata = JSON.stringify({
        artifact_digest: artifactDigest,
        engine_ref: engineRef,
        ...(source === "default" ? { source_key: selectedSource?.key } : {}),
      });
      const body = `${metadata.slice(0, -1)},"bindings":${parsedBindings},"fleet_input_policy":${parsedPolicy}}`;
      const { data } = await api<Accepted>(path, {
        method: "PUT",
        body,
        headers: attempt.current.headers("PUT", path, body, snapshot?.etag || null, !snapshot),
      });
      void client.invalidateQueries();
      if (mounted.current)
        onAccepted(data.noOp ? null : { id: data.changeId, resource: key, type: "profile" });
    } catch (error) {
      if (mounted.current) setError(error);
    } finally {
      if (mounted.current) setBusy(false);
    }
  }

  return (
    <form aria-label="Template configuration" className="min-w-0" onSubmit={submit}>
      <div className="grid min-w-0 items-start gap-6 xl:grid-cols-[minmax(0,1fr)_17rem]">
        <fieldset disabled={busy} className="min-w-0 space-y-6">
          <section
            className="space-y-6 rounded-xl border bg-card p-5 sm:p-6"
            aria-labelledby={`${id}-details`}
          >
            <div>
              <h2 id={`${id}-details`}>Template details</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Give your template a name and choose where its configuration comes from.
              </p>
            </div>
            <Field label="Profile key">
              <Input
                required
                disabled={!!snapshot}
                pattern="[a-z0-9][a-z0-9-]*"
                maxLength={63}
                placeholder="e.g. docker-linux"
                autoComplete="off"
                spellCheck={false}
                aria-describedby={`${id}-key-hint`}
                value={key}
                onChange={(event) => setKey(event.target.value)}
              />
              <p id={`${id}-key-hint`} className="text-xs text-muted-foreground">
                {snapshot
                  ? "The profile key stays the same for every revision."
                  : "A unique name used by fleets. Lowercase letters, numbers and hyphens, up to 63 characters."}
              </p>
            </Field>
            <div className="border-t pt-6">
              <TemplateSourceFields
                source={source}
                setSource={setSource}
                selectedSource={selectedSource}
                onSelect={setSelectedSource}
                digest={digest}
                setDigest={setDigest}
                file={file}
                setFile={setFile}
                canRead={canRead}
                busy={busy}
              />
            </div>
          </section>

          <section
            className="space-y-5 rounded-xl border bg-card p-5 sm:p-6"
            aria-labelledby={`${id}-variables`}
            aria-busy={discovery.loading}
          >
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0 flex-1 basis-56">
                <h2 id={`${id}-variables`}>Configure variables</h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  Set shared bindings and the values fleets are allowed to use.
                </p>
              </div>
              {canRead && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={discovery.loading || !sourceReady}
                  onClick={() => void discovery.inspect()}
                >
                  {discovery.loading ? <LoaderCircle className="animate-spin" /> : <FileSearch />}
                  {discovery.loading ? "Inspecting variables..." : "Inspect variables"}
                </Button>
              )}
            </div>
            {!canRead ? (
              <p className="rounded-lg bg-muted/50 p-4 text-sm text-muted-foreground">
                Template read permission is required to inspect variables. Manual publishing is
                available in Advanced settings.
              </p>
            ) : discovery.loading ? (
              <div
                role="status"
                className="flex items-center gap-3 rounded-lg bg-muted/40 p-5 text-sm text-muted-foreground"
              >
                <LoaderCircle className="size-4 shrink-0 animate-spin" />
                Reading template variables…
              </div>
            ) : !discovery.variables && discovery.error === null ? (
              <div className="flex items-start gap-3 rounded-lg border border-dashed p-5">
                <FileSearch
                  className="mt-0.5 size-5 shrink-0 text-muted-foreground"
                  aria-hidden="true"
                />
                <div className="space-y-1 text-sm">
                  <p className="font-medium">
                    {sourceReady
                      ? "Your template is ready to inspect"
                      : "Choose a source to get started"}
                  </p>
                  <p className="text-muted-foreground">
                    {source === "archive"
                      ? "Inspect variables uploads your archive and loads its configurable fields."
                      : source === "default"
                        ? "Variables load automatically when you choose a default template."
                        : "Enter an artifact digest, then inspect its configurable fields."}
                  </p>
                </div>
              </div>
            ) : null}
            {discovery.error !== null && (
              <>
                <ErrorNotice error={discovery.error} />
                <p className="text-sm text-muted-foreground">
                  Try inspecting again, or configure bindings and input policy in Advanced settings.
                </p>
              </>
            )}
            {discovery.variables && (
              <TemplateVariables
                variables={discovery.variables}
                bindings={bindings}
                policy={policy}
                setBindings={setBindings}
                setPolicy={setPolicy}
              />
            )}
          </section>

          <div className="rounded-xl border bg-card px-5 pb-5 pt-3 sm:px-6">
            <AdvancedSettings
              open={advanced}
              onOpenChange={setAdvanced}
              summary="Engine reference, manual bindings and fleet input policy."
            >
              <Field label="Engine reference">
                <Input
                  required
                  readOnly={source === "default"}
                  value={engineRef}
                  onChange={(event) => setEngine(event.target.value)}
                />
                {source === "default" && (
                  <p className="text-xs text-muted-foreground">
                    The default template fixes its engine. Choose Existing artifact to use a custom
                    engine.
                  </p>
                )}
              </Field>
              <Field label="Bindings (JSON)">
                <Textarea
                  autoComplete="off"
                  spellCheck={false}
                  className="mono min-h-28"
                  value={bindings}
                  onChange={(event) => setBindings(event.target.value)}
                />
              </Field>
              <Field label="Fleet input policy (JSON)">
                <Textarea
                  spellCheck={false}
                  className="mono min-h-28"
                  value={policy}
                  onChange={(event) => setPolicy(event.target.value)}
                />
              </Field>
            </AdvancedSettings>
          </div>
        </fieldset>

        <aside
          className="min-w-0 rounded-xl border bg-muted/25 p-5 xl:sticky xl:top-6"
          aria-label="Publication summary"
        >
          <div className="mb-5 flex items-center gap-2">
            <Layers3 className="size-4" aria-hidden="true" />
            <h2 className="text-sm">Publication summary</h2>
          </div>
          <dl className="space-y-4 text-sm">
            <div>
              <dt className="text-xs text-muted-foreground">Profile key</dt>
              <dd className="mt-1 break-all font-medium">{key || "Not named yet"}</dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">Source</dt>
              <dd className="mt-1 break-all font-medium">
                {source === "default"
                  ? selectedSource?.key || "Choose a default template"
                  : source === "archive"
                    ? file?.name || "Upload archive"
                    : "Existing artifact"}
              </dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">Engine</dt>
              <dd className="mt-1 break-all font-medium">{engineRef || "Not set"}</dd>
            </div>
          </dl>
          <div className="mt-5 space-y-3 border-t pt-5 text-sm">
            <div className="flex items-center gap-2 font-medium">
              <CheckCheck className="size-4 text-muted-foreground" aria-hidden="true" />
              What happens next
            </div>
            <p className="text-xs leading-relaxed text-muted-foreground">
              Shaula validates your template after publishing. Once validation passes, the new
              revision activates automatically.
            </p>
            <p className="flex items-center gap-2 text-xs text-muted-foreground">
              Publish <ArrowRight className="size-3" aria-hidden="true" /> Validate{" "}
              <ArrowRight className="size-3" aria-hidden="true" /> Activate
            </p>
          </div>
        </aside>
      </div>

      <div className="sticky bottom-0 z-10 mt-6 border-t bg-background/95 py-4 backdrop-blur-sm">
        {error !== null && <ErrorNotice error={error} />}
        <div className="flex items-center justify-between gap-3">
          <p className="hidden text-xs text-muted-foreground sm:block">
            {busy ? "Publishing your template…" : "Changes are saved when you publish."}
          </p>
          <div className="flex w-full justify-end gap-2 sm:w-auto">
            <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || discovery.loading}>
              {busy ? <LoaderCircle className="animate-spin" /> : <Upload />}{" "}
              {busy ? "Publishing…" : "Publish"}
            </Button>
          </div>
        </div>
      </div>
    </form>
  );
}
