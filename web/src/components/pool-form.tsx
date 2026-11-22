import { useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Save } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import type { Accepted, TemplatePoolMemberSpec, TemplatePoolResource } from "@/lib/types";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { initialPoolMember, poolMembersProblem } from "@/lib/template-pool";
import { TemplatePoolEditor } from "./template-pool-editor";
import { ErrorNotice, Field } from "./status";

/**
 * Create/replace form for one shared TemplatePool resource (spec 0037).
 * A pool PUT always resolves members to their templates' current Active
 * revisions; identical re-assertions are replayable no-ops.
 */
export function PoolForm({
  resource,
  scopes,
  onClose,
  onAccepted,
}: {
  resource?: Resource<TemplatePoolResource>;
  scopes: string[];
  onClose: () => void;
  onAccepted: () => void;
}) {
  const client = useQueryClient();
  const [key, setKey] = useState(resource?.data.key || "");
  const [members, setMembers] = useState<TemplatePoolMemberSpec[]>(
    resource?.data.spec.members || [initialPoolMember(0)],
  );
  const [failurePolicy, setFailurePolicy] = useState<"backpressure" | "redistribute">(
    resource?.data.spec.failure_policy || "backpressure",
  );
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [attempt] = useState(() => new MutationAttempt());
  const canReadTemplates = scopes.includes("template.read");

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      const problem = poolMembersProblem(members, { requireTemplate: true });
      if (problem) throw new Error(problem);
      const body = JSON.stringify({ members, failure_policy: failurePolicy });
      const path = resourcePath("template-pools", key);
      const headers = attempt.headers("PUT", path, body, resource?.etag || null, !resource);
      setBusy(true);
      await api<Accepted>(path, { method: "PUT", headers, body });
      void client.invalidateQueries({ queryKey: ["template-pools"] });
      onAccepted();
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
      title={resource ? `Edit pool ${key}` : "Create pool"}
      className="sm:max-w-2xl"
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <form onSubmit={submit} className="form-stack">
        <fieldset disabled={busy} className="form-stack">
          <Field label="Pool key">
            <Input
              required
              pattern="[a-z0-9][a-z0-9-]*"
              maxLength={128}
              value={key}
              disabled={!!resource}
              onChange={(event) => setKey(event.target.value)}
              placeholder="hkg-builders"
            />
          </Field>
          <TemplatePoolEditor
            members={members}
            setMembers={setMembers}
            failurePolicy={failurePolicy}
            setFailurePolicy={setFailurePolicy}
            canRead={canReadTemplates}
          />
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy}>
            {busy ? <LoaderCircle className="animate-spin" /> : <Save />}
            {resource ? "Save pool" : "Create pool"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
