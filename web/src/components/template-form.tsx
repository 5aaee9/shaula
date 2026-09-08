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
export function TemplateForm({
  resource,
  onClose,
  onAccepted,
}: {
  resource?: Resource<TemplateResource>;
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
}) {
  const [snapshot] = useState(resource);
  const client = useQueryClient();
  const [key, setKey] = useState(snapshot?.data.key || "");
  const [digest, setDigest] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [source, setSource] = useState("archive");
  const [advanced, setAdvanced] = useState(false);
  const [engine, setEngine] = useState("terraform");
  const [bindings, setBindings] = useState("{}");
  const [policy, setPolicy] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  function parseSettings(value: string, label: string) {
    try {
      const parsed: unknown = JSON.parse(value);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error();
      return parsed;
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
      let artifactDigest = digest;
      if (source === "archive") {
        if (!file) throw new Error("Choose a template archive.");
        if (file.size > 64 * 1024 * 1024) throw new Error("Artifact exceeds 64 MiB.");
        const bytes = await file.arrayBuffer();
        const hash = await crypto.subtle.digest("SHA-256", bytes);
        artifactDigest = `sha256:${Array.from(new Uint8Array(hash), (value) => value.toString(16).padStart(2, "0")).join("")}`;
        await api(resourcePath("template-artifacts", artifactDigest), {
          method: "PUT",
          body: bytes,
          headers: { "content-type": "application/gzip" },
        });
      }
      const path = resourcePath("template-profiles", key);
      const body = JSON.stringify({
        artifact_digest: artifactDigest,
        engine_ref: engine,
        bindings: parsedBindings,
        fleet_input_policy: parsedPolicy,
      });
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
            </NativeSelect>
          </Field>
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
          <AdvancedSettings
            open={advanced}
            onOpenChange={setAdvanced}
            summary="Engine, template bindings and input restrictions."
          >
            <Field label="Engine reference">
              <Input required value={engine} onChange={(event) => setEngine(event.target.value)} />
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
          <Button type="submit" disabled={busy}>
            {busy ? <LoaderCircle className="animate-spin" /> : <Upload />}Publish
          </Button>
        </div>
      </form>
    </Modal>
  );
}
