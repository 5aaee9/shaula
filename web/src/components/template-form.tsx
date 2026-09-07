import { useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Upload } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import type { Accepted, ChangeRef, TemplateResource } from "@/lib/types";
import { Modal } from "./modal";
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
  const [engine, setEngine] = useState("terraform");
  const [bindings, setBindings] = useState("{}");
  const [policy, setPolicy] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const parsedBindings: unknown = JSON.parse(bindings);
      const parsedPolicy: unknown = JSON.parse(policy);
      if (
        !parsedBindings ||
        typeof parsedBindings !== "object" ||
        Array.isArray(parsedBindings) ||
        !parsedPolicy ||
        typeof parsedPolicy !== "object" ||
        Array.isArray(parsedPolicy)
      )
        throw new Error("Bindings and input policy must be JSON objects.");
      let artifactDigest = digest;
      if (file) {
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
          <Field label="Template archive (.tar.gz)">
            <Input
              type="file"
              accept=".tar.gz,.tgz,application/gzip"
              onChange={(event) => setFile(event.target.files?.[0] || null)}
            />
          </Field>
          {!file && (
            <Field label="Existing artifact digest">
              <Input
                required
                pattern="sha256:[a-f0-9]{64}"
                placeholder="sha256:..."
                value={digest}
                onChange={(event) => setDigest(event.target.value)}
              />
            </Field>
          )}
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
