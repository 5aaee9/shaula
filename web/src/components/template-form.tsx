import { useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Upload } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import type { Accepted, ChangeRef, TemplateResource } from "@/lib/types";
import { Modal } from "./modal";
import { AdvancedSettings } from "./advanced-settings";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";
import { ErrorNotice, Field } from "./status";
import { inputEntries } from "@/lib/input-values";
import { useTemplateVariables, type TemplateSource } from "@/lib/template-variables";
import { TemplateVariables } from "./template-variables";
import { useTemplateSources } from "./template-library";
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
  onAccepted: (change: ChangeRef) => void;
}) {
  const [snapshot] = useState(resource);
  const client = useQueryClient();
  const [key, setKey] = useState(snapshot?.data.key || "");
  const [digest, setDigest] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [source, setSource] = useState(initialSource ? "default" : "archive");
  const [selectedSource, setSelectedSource] = useState(initialSource);
  const [advanced, setAdvanced] = useState(false);
  const [engine, setEngine] = useState(initialSource?.engineRef || "terraform");
  const engineEdited = useRef(false);
  const [bindings, setBindings] = useState("{}");
  const [policy, setPolicy] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  const canRead = scopes.includes("template.read");
  const sources = useTemplateSources(canRead);
  const discovery = useTemplateVariables(
    source === "default" ? selectedSource?.artifactDigest || "" : digest,
    file,
    source,
    canRead,
  );
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
        engine_ref: engine,
      });
      const body = `${metadata.slice(0, -1)},"bindings":${parsedBindings},"fleet_input_policy":${parsedPolicy}}`;
      const { data } = await api<Accepted>(path, {
        method: "PUT",
        body,
        headers: attempt.current.headers("PUT", path, body, snapshot?.etag || null, !snapshot),
      });
      onAccepted({ id: data.changeId, resource: key, type: "profile" });
      void client.invalidateQueries();
      onClose();
    } catch (error) {
      setError(error);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      open
      title={snapshot ? `Publish revision of ${key}` : "Publish template"}
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <form className="form-stack" onSubmit={submit}>
        <fieldset disabled={busy} className="form-stack">
          <Field label="Profile key">
            <Input
              required
              disabled={!!snapshot}
              pattern="[a-z0-9][a-z0-9-]*"
              maxLength={63}
              value={key}
              onChange={(event) => setKey(event.target.value)}
            />
          </Field>
          <Field label="Template source">
            <NativeSelect value={source} onChange={(event) => setSource(event.target.value)}>
              <NativeSelectOption value="archive">Upload archive</NativeSelectOption>
              <NativeSelectOption value="existing">Existing artifact</NativeSelectOption>
              <NativeSelectOption value="default" disabled={!canRead}>
                Default template
              </NativeSelectOption>
            </NativeSelect>
          </Field>
          {source === "default" && (
            <div className="form-stack gap-2">
              <Field label="Default template">
                <NativeSelect
                  required
                  value={selectedSource?.key || ""}
                  onChange={(event) => {
                    const next = sources.data?.data.sources.find(
                      (entry) => entry.key === event.target.value,
                    );
                    setSelectedSource(next);
                    if (next && !engineEdited.current) setEngine(next.engineRef);
                  }}
                >
                  <NativeSelectOption value="">Choose a template</NativeSelectOption>
                  {sources.data?.data.sources.map((entry) => (
                    <NativeSelectOption key={entry.key} value={entry.key}>
                      {entry.key}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              </Field>
              {sources.error && (
                <ErrorNotice error={sources.error} retry={() => void sources.refetch()} />
              )}
              {selectedSource && (
                <p className="text-xs text-muted-foreground break-all">
                  {selectedSource.platform} · {selectedSource.artifactDigest}
                </p>
              )}
            </div>
          )}
          <div hidden={source !== "archive"}>
            <Field label="Template archive (.tar.gz)">
              <Input
                type="file"
                required={source === "archive"}
                disabled={source !== "archive"}
                accept=".tar.gz,.tgz,application/gzip"
                onChange={(event) => setFile(event.target.files?.[0] || null)}
              />
            </Field>
          </div>
          <div hidden={source !== "existing"}>
            <Field label="Existing artifact digest">
              <Input
                required={source === "existing"}
                disabled={source !== "existing"}
                pattern="sha256:[a-f0-9]{64}"
                placeholder="sha256:..."
                value={digest}
                onChange={(event) => setDigest(event.target.value)}
              />
            </Field>
          </div>
          {canRead ? (
            <Button
              type="button"
              variant="outline"
              className="self-start"
              disabled={discovery.loading}
              onClick={() => void discovery.inspect()}
            >
              {discovery.loading ? "Inspecting variables..." : "Inspect variables"}
            </Button>
          ) : (
            <p className="text-sm text-muted-foreground">
              Template read permission is required to inspect variables. Manual publishing is
              available.
            </p>
          )}
          {discovery.error !== null && <ErrorNotice error={discovery.error} />}
          {discovery.variables && (
            <TemplateVariables
              variables={discovery.variables}
              bindings={bindings}
              policy={policy}
              setBindings={setBindings}
              setPolicy={setPolicy}
            />
          )}
          <AdvancedSettings
            open={advanced}
            onOpenChange={setAdvanced}
            summary="Engine, template bindings and input restrictions."
          >
            <Field label="Engine reference">
              <Input
                required
                value={engine}
                onChange={(event) => {
                  engineEdited.current = true;
                  setEngine(event.target.value);
                }}
              />
            </Field>
            <Field label="Bindings (JSON)">
              <Textarea
                autoComplete="off"
                spellCheck={false}
                className="mono"
                value={bindings}
                onChange={(event) => setBindings(event.target.value)}
              />
            </Field>
            <Field label="Fleet input policy (JSON)">
              <Textarea
                spellCheck={false}
                className="mono"
                value={policy}
                onChange={(event) => setPolicy(event.target.value)}
              />
            </Field>
          </AdvancedSettings>
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy || discovery.loading}>
            {busy ? <LoaderCircle className="animate-spin" /> : <Upload />}Publish
          </Button>
        </div>
      </form>
    </Modal>
  );
}
