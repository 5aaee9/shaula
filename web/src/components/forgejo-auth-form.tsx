import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { KeyRound, LoaderCircle } from "lucide-react";
import { api, MutationAttempt, resourcePath } from "@/lib/api";
import type { Accepted } from "@/lib/types";
import { useAuthImpact } from "@/lib/queries";
import type { ForgejoTarget } from "@/lib/runner-backend";
import type { AuthFormProps } from "./auth-form";
import { ForgejoTargetFields } from "./runner-backend-fields";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { ErrorNotice, Field } from "./status";

export function ForgejoAuthForm({
  mode,
  resource,
  onClose,
  onAccepted,
  backendSelector,
}: AuthFormProps & { backendSelector?: ReactNode }) {
  const client = useQueryClient();
  const [key, setKey] = useState(resource?.data.key || "");
  const [target, setTarget] = useState<ForgejoTarget>(
    resource?.data.active?.forgejo?.target || { instance_url: "", scope: { kind: "instance" } },
  );
  const [token, setToken] = useState("");
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
  const impact = useAuthImpact(key, !!resource);
  const ready = mode !== "policy" && impact.ready && (!resource || !!resource.data.active?.forgejo);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      if (!ready)
        throw new Error("Load the Active Forgejo target and Fleet impact before rotating.");
      const url = new URL(target.instance_url);
      if (
        !["https:", "http:"].includes(url.protocol) ||
        url.username ||
        url.password ||
        url.search ||
        url.hash
      )
        throw new Error("Use an HTTP(S) instance URL without credentials, query or fragment.");
      if (!token.trim()) throw new Error("Enter a new Forgejo access token.");
      const body = JSON.stringify({ kind: "forgejo_token", ...target, token });
      const path = resourcePath("github-auth-profiles", key);
      setBusy(true);
      const result = await api<Accepted>(path, {
        method: "PUT",
        body,
        headers: attempt.current.headers("PUT", path, body, resource?.etag || null, !resource),
      });
      void client.invalidateQueries();
      if (mounted.current) {
        setToken("");
        onAccepted({ id: result.data.changeId, resource: key, type: "profile" });
      }
    } catch (error) {
      if (mounted.current) setError(error);
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <form
      aria-label="Authentication configuration"
      className="form-stack min-w-0"
      onSubmit={submit}
    >
      <fieldset disabled={busy} className="form-stack min-w-0">
        {backendSelector}
        <section className="form-stack min-w-0 rounded-xl border p-6">
          <h2>Forgejo token credential</h2>
          <Field label="Profile key">
            <Input
              required
              pattern="[a-z0-9][a-z0-9-]*"
              maxLength={63}
              disabled={!!resource}
              value={key}
              onChange={(e) => setKey(e.target.value)}
            />
          </Field>
          <ForgejoTargetFields value={target} onChange={setTarget} disabled={!!resource} />
          <Field label="Access token">
            <Input
              required
              type="password"
              autoComplete="new-password"
              spellCheck={false}
              value={token}
              onChange={(e) => setToken(e.target.value)}
            />
          </Field>
          <p className="text-sm text-muted-foreground">
            The token is write-only and is never sent to workflow runners. Instance scope requires
            an administrator; other scopes require runner-management access to that exact target.
            Shaula validates access before activation.
          </p>
        </section>
        {resource && (
          <section className="form-stack text-sm" aria-label="Credential rotation impact">
            <h2>Live Fleet impact</h2>
            <p>
              The instance and scope are unchanged. Rotation is not a seamless handoff: existing
              runners may need to finish and be recreated. Keep the previous token valid until their
              cleanup completes.
            </p>
            {impact.error ? (
              <ErrorNotice error={impact.error} retry={() => void impact.refetch()} />
            ) : impact.isPending ? (
              <p>Loading current Fleet references…</p>
            ) : impact.data?.length ? (
              <ul className="ml-4 list-disc">
                {impact.data.map((fleet) => (
                  <li key={fleet.fleetKey}>
                    {fleet.fleetKey} ({fleet.phase})
                  </li>
                ))}
              </ul>
            ) : (
              <p>No live Fleets reference this profile.</p>
            )}
          </section>
        )}
      </fieldset>
      {error !== null && <ErrorNotice error={error} />}
      <div className="sticky bottom-0 z-10 flex flex-wrap items-center justify-end gap-2 border-t bg-background py-4">
        <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" disabled={busy || !ready}>
          {busy ? <LoaderCircle className="animate-spin" /> : <KeyRound />}
          {resource ? "Rotate credential" : "Create profile"}
        </Button>
      </div>
    </form>
  );
}
