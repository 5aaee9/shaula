import { useRef, useState, type FormEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, LoaderCircle } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import type { Accepted, AuthResource, ChangeRef } from "@/lib/types";
import {
  rowsFromSelectors,
  selectorFromRow,
  policyDifference,
  type SelectorRow,
} from "@/lib/auth-policy";
import { AuthPolicyPreview } from "./auth-policy-preview";
import { AuthTargetPolicy } from "./auth-target-policy";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";
import { ErrorNotice, Field } from "./status";

export function AuthForm({
  resource,
  onClose,
  onAccepted,
}: {
  resource?: Resource<AuthResource>;
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
}) {
  const [snapshot] = useState(resource);
  const client = useQueryClient();
  const [key, setKey] = useState(snapshot?.data.key || "");
  const [appId, setAppId] = useState(snapshot?.data.active?.app_id || snapshot?.data.app_id || "");
  const [secret, setSecret] = useState("");
  const existingSelectors =
    snapshot?.data.desired?.target_policy || snapshot?.data.active?.target_policy || [];
  const [selectors, setSelectors] = useState<SelectorRow[]>(
    existingSelectors.length
      ? rowsFromSelectors(existingSelectors)
      : [{ kind: "organization", owner: "", repository: "", account_kind: "user" }],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  function selectorError(rows: SelectorRow[]): string | null {
    if (!rows.length) return "At least one target selector is required.";
    for (const row of rows) {
      if (!row.owner.trim()) return "Every selector needs an owner.";
      if (row.kind === "repository" && !row.repository.trim())
        return "A repository selector needs a repository name.";
    }
    return null;
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (!impactReady) throw new Error("Load the current Fleet impact before publishing.");
      if (!/^[1-9][0-9]*$/.test(appId))
        throw new Error("Enter the numeric App ID of the same GitHub App before publishing.");
      const problem = selectorError(selectors);
      if (problem) throw new Error(problem);
      const body = JSON.stringify({
        kind: "github_app",
        schema_version: 2,
        app_id: appId,
        private_key: secret,
        target_policy: selectors.map(selectorFromRow),
      });
      const path = resourcePath("github-auth-profiles", key);
      const { data } = await api<Accepted>(path, {
        method: "PUT",
        body,
        headers: attempt.current.headers("PUT", path, body, snapshot?.etag || null, !snapshot),
      });
      setSecret("");
      onAccepted({ id: data.changeId, resource: key, type: "profile" });
      void client.invalidateQueries();
      onClose();
    } catch (thrown) {
      setError(thrown);
    } finally {
      setBusy(false);
    }
  }
  const nextPolicy = selectors.map(selectorFromRow);
  const previousPolicy = snapshot?.data.active?.target_policy || [];
  const difference = policyDifference(previousPolicy, nextPolicy);
  const policyChanged = !!difference.added.length || !!difference.removed.length;
  const impact = useQuery({
    queryKey: ["auth-impact", key],
    enabled: !!snapshot,
    queryFn: async ({ signal }) => {
      const response = await api<{ liveFleets: import("@/lib/types").AuthLiveFleet[] }>(
        resourcePath("github-auth-profiles", key) + "/impact",
        { signal },
      );
      if (!Array.isArray(response.data.liveFleets))
        throw new Error("Fleet impact response is unavailable");
      return response.data.liveFleets;
    },
    refetchInterval: 5_000,
  });
  const impactReady = !snapshot || (impact.isSuccess && !impact.isFetching);
  return (
    <Modal
      open
      title={snapshot ? `Publish ${key}` : "Create authentication profile"}
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <form className="form-stack" onSubmit={submit}>
        <fieldset disabled={busy} className="form-stack">
          <Field label="Profile key">
            <Input
              required
              pattern="[a-z0-9][a-z0-9-]*"
              maxLength={63}
              disabled={!!snapshot}
              value={key}
              onChange={(event) => setKey(event.target.value)}
            />
          </Field>
          <Field label="App ID">
            <Input
              required
              value={appId}
              disabled={!!snapshot?.data.active}
              pattern="[1-9][0-9]*"
              title="Enter the positive numeric App ID, not a client ID"
              onChange={(event) => setAppId(event.target.value)}
            />
          </Field>
          <Field label="Private key (PEM)">
            <Textarea
              required
              rows={3}
              className="h-24 field-sizing-fixed"
              autoComplete="off"
              spellCheck={false}
              value={secret}
              onChange={(event) => setSecret(event.target.value)}
            />
          </Field>
          <AuthTargetPolicy rows={selectors} onChange={setSelectors} existing={!!snapshot} />
          {snapshot && (
            <AuthPolicyPreview
              previous={previousPolicy}
              next={nextPolicy}
              fleets={impact.data}
              loading={impact.isPending}
              error={impact.isError}
            />
          )}
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button variant="outline" type="button" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy || !impactReady}>
            {busy ? <LoaderCircle className="animate-spin" /> : <KeyRound />}
            {snapshot ? (policyChanged ? "Publish policy" : "Rotate credential") : "Create profile"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
