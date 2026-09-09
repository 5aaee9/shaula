import { useEffect, useRef, useState, type FormEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, LoaderCircle } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import type { Accepted, AuthResource, ChangeRef } from "@/lib/types";
import { selectorLabel } from "@/lib/types";
import {
  rowsFromSelectors,
  selectorFromRow,
  selectorKey,
  type SelectorRow,
} from "@/lib/auth-policy";
import { AuthPolicyPreview } from "./auth-policy-preview";
import { AuthTargetPolicy } from "./auth-target-policy";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";
import { ErrorNotice, Field, KeyValue } from "./status";

export type AuthFormMode = "create" | "policy" | "rotate";
type AuthFormProps = {
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
} & (
  | { mode: "create"; resource?: never }
  | { mode: "policy" | "rotate"; resource: Resource<AuthResource> }
);

export function AuthForm({ mode, resource, onClose, onAccepted }: AuthFormProps) {
  const client = useQueryClient();
  const [key, setKey] = useState(resource?.data.key || "");
  const [appId, setAppId] = useState(resource?.data.active?.app_id || "");
  const [secret, setSecret] = useState("");
  const previousPolicy = resource?.data.active?.target_policy || [];
  const [selectors, setSelectors] = useState<SelectorRow[]>(
    previousPolicy.length
      ? rowsFromSelectors(previousPolicy)
      : [{ kind: "organization", owner: "", repository: "", account_kind: "user" }],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
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
      const body = JSON.stringify(
        mode === "policy"
          ? {
              base_revision: resource?.data.activeRevision,
              target_policy: nextPolicy,
            }
          : {
              kind: "github_app",
              schema_version: 2,
              app_id: appId,
              private_key: secret,
              target_policy: nextPolicy,
            },
      );
      const path =
        resourcePath("github-auth-profiles", key) + (mode === "policy" ? "/policy-updates" : "");
      const method = mode === "policy" ? "POST" : "PUT";
      const { data } = await api<Accepted>(path, {
        method,
        body,
        headers: attempt.current.headers(method, path, body, resource?.etag || null, !resource),
      });
      void client.invalidateQueries();
      if (mounted.current) {
        setSecret("");
        onAccepted({ id: data.changeId, resource: key, type: "profile" });
      }
    } catch (thrown) {
      if (mounted.current) setError(thrown);
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  const nextPolicy = mode === "rotate" ? previousPolicy : selectors.map(selectorFromRow);
  const impact = useQuery({
    queryKey: ["auth-impact", key],
    enabled: !!resource,
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
  const impactReady = !resource || (impact.isSuccess && !impact.isFetching);
  return (
    <form
      aria-label="Authentication configuration"
      className="form-stack min-w-0"
      onSubmit={submit}
    >
      <fieldset disabled={busy} className="form-stack min-w-0">
        <section className="form-stack min-w-0 rounded-xl border p-6">
          <h2>{mode === "policy" ? "Active credential" : "GitHub App credential"}</h2>
          {mode === "policy" ? (
            <dl className="details-grid">
              <KeyValue label="App ID">{appId}</KeyValue>
              <KeyValue label="Base revision">r{resource.data.activeRevision}</KeyValue>
            </dl>
          ) : (
            <>
              <Field label="Profile key">
                <Input
                  required
                  pattern="[a-z0-9][a-z0-9-]*"
                  maxLength={63}
                  disabled={!!resource}
                  value={key}
                  onChange={(event) => setKey(event.target.value)}
                />
              </Field>
              <Field label="App ID">
                <Input
                  required
                  value={appId}
                  disabled={!!resource}
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
            </>
          )}
        </section>
        <section className="form-stack min-w-0 rounded-xl border p-6">
          {mode === "rotate" ? (
            <>
              <h2>Active target policy</h2>
              <p className="text-sm text-muted-foreground">
                These targets are retained when the credential is rotated.
              </p>
              <ul className="flex flex-wrap gap-2">
                {previousPolicy.map((selector) => (
                  <li className="label-chip" key={selectorKey(selector)}>
                    {selectorLabel(selector)}
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <AuthTargetPolicy rows={selectors} onChange={setSelectors} existing={!!resource} />
          )}
        </section>
        {resource && (
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
      <div className="sticky bottom-0 z-10 flex flex-wrap items-center justify-end gap-2 border-t bg-background py-4">
        <Button variant="outline" type="button" disabled={busy} onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" disabled={busy || !impactReady}>
          {busy ? <LoaderCircle className="animate-spin" /> : <KeyRound />}
          {mode === "create"
            ? "Create profile"
            : mode === "policy"
              ? "Publish policy"
              : "Rotate credential"}
        </Button>
      </div>
    </form>
  );
}
