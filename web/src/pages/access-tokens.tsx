import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { authenticatedFetch } from "@/lib/authentication";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorNotice, Loading } from "@/components/status";

interface Token {
  id: string;
  name: string;
  scopes: string[];
  effective_scopes: string[];
  created_at: string;
  expires_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
  state: string;
  revision: number;
}
interface Issued {
  access_token: Token;
  secret_available: boolean;
  token?: string;
}

export function AccessTokensPage({
  scopes,
  enabled = true,
}: {
  scopes: string[];
  enabled?: boolean;
}) {
  const [name, setName] = useState("");
  const [days, setDays] = useState(90);
  const [selected, setSelected] = useState<string[]>([]);
  const [issued, setIssued] = useState<Issued | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [saved, setSaved] = useState(false);
  const [rotating, setRotating] = useState<Token | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [state, setState] = useState("all");
  const attempt = useRef({ body: "", key: "" });
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const readable = scopes.includes("access-token.read");
  const tokens = useQuery({
    queryKey: ["access-tokens", state, cursor],
    enabled: readable,
    queryFn: ({ signal }) =>
      api<{ items: Token[]; next_cursor: string | null }>(
        `/access-tokens?${new URLSearchParams({ state, ...(cursor ? { cursor } : {}) })}`,
        { signal },
      ),
  });
  async function create() {
    const body = JSON.stringify({
      name,
      scopes: [...selected].sort(),
      expires_in_seconds: days * 86400,
    });
    if (attempt.current.body !== body) attempt.current = { body, key: crypto.randomUUID() };
    setBusy(true);
    setError(null);
    setIssued(null);
    setRevealed(false);
    setSaved(false);
    try {
      const result = await api<Issued>("/access-tokens", {
        method: "POST",
        body,
        headers: { "idempotency-key": attempt.current.key },
      });
      if (!mounted.current) return;
      setIssued(result.data);
      if (result.data.secret_available) attempt.current = { body: "", key: "" };
      void tokens.refetch();
    } catch (e) {
      if (mounted.current)
        setError(`${errorMessage(e)} Retry unchanged input to recover the same request.`);
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  async function revoke(token: Token) {
    if (!window.confirm(`Revoke “${token.name}”? Scripts using this token will lose access.`))
      return;
    setBusy(true);
    setError(null);
    try {
      const reviewed = await api<Token>(`/access-tokens/${encodeURIComponent(token.id)}`);
      if (!reviewed.etag || reviewed.data.revision !== token.revision)
        throw new Error("Token changed. Refresh and review it before revoking.");
      const response = await authenticatedFetch(`/api/v1/access-tokens/${token.id}`, {
        method: "DELETE",
        headers: { "if-match": reviewed.etag },
        credentials: "same-origin",
        cache: "no-store",
        redirect: "error",
      });
      if (!response.ok)
        throw new Error(
          `Revocation failed (${response.status}). Refresh the list before retrying.`,
        );
      if (!mounted.current) return;
      if (issued?.access_token.id === token.id) {
        setIssued(null);
        attempt.current = { body: "", key: "" };
      }
      if (rotating?.id === token.id) setRotating(null);
      void tokens.refetch();
    } catch (e) {
      if (mounted.current) setError(errorMessage(e));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  async function savedToken() {
    if (!issued?.token) return;
    setBusy(true);
    setError(null);
    try {
      const primary = await api<{ principal: { issuer: string; subject: string } }>("/session");
      const response = await fetch("/api/v1/session", {
        headers: { authorization: `Bearer ${issued.token}` },
        credentials: "omit",
        redirect: "error",
        cache: "no-store",
      });
      if (!response.ok)
        throw new Error("The new token could not be verified. The old token remains active.");
      const verified = await response.json();
      if (
        verified.authentication?.token_id !== issued.access_token.id ||
        verified.principal?.issuer !== primary.data.principal.issuer ||
        verified.principal?.subject !== primary.data.principal.subject
      )
        throw new Error("New token identity verification failed. The old token remains active.");
      if (!mounted.current) return;
      setSaved(true);
      setIssued({ ...issued, token: undefined });
      setRevealed(false);
    } catch (e) {
      if (mounted.current) setError(errorMessage(e));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <div className="space-y-8">
      <div>
        <h1 className="text-2xl font-semibold">Access tokens</h1>
        <p className="text-muted-foreground mt-2">
          Use a personal token with the API or CLI. Permissions apply to every resource in a
          selected category.
        </p>
        <p className="text-muted-foreground text-sm mt-1">
          Tokens expire and can be revoked. Signing out of the browser does not revoke them.
        </p>
      </div>
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
      {issued && (
        <section aria-label="Token created" className="border rounded-lg p-5 space-y-3">
          <h2 className="font-semibold">
            {saved
              ? "New token saved and verified"
              : issued.secret_available
                ? "Save your token now"
                : "Request recovered — secret unavailable"}
          </h2>
          <p>
            {issued.access_token.name} · {issued.access_token.id}
          </p>
          {issued.token ? (
            <>
              <p>This is the only time this token is shown. Save it securely before closing.</p>
              {revealed ? (
                <code className="block break-all select-all p-3 bg-muted">{issued.token}</code>
              ) : (
                <Button variant="outline" onClick={() => setRevealed(true)}>
                  Reveal token
                </Button>
              )}
              <Button disabled={busy} onClick={() => void savedToken()}>
                I have saved it — verify
              </Button>
            </>
          ) : saved ? (
            <p>
              The new credential is valid.{" "}
              {rotating
                ? "The old token is still active until you revoke it below."
                : "Keep its saved copy secure."}
            </p>
          ) : (
            <p>
              The server already accepted this request. Revoke this token and create another to
              receive a new secret.
            </p>
          )}
          {saved && rotating && scopes.includes("access-token.revoke") && (
            <Button disabled={busy} onClick={() => void revoke(rotating)}>
              Revoke old token: {rotating.name}
            </Button>
          )}
          {scopes.includes("access-token.revoke") && (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => void revoke(issued.access_token)}
            >
              Revoke this token
            </Button>
          )}
        </section>
      )}
      {!enabled && (
        <p>
          Personal access tokens are disabled by the deployment. Existing token metadata remains
          available for review and revocation.
        </p>
      )}
      {enabled && scopes.includes("access-token.write") && (
        <form
          className="max-w-2xl space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <h2 className="text-lg font-semibold">
            {rotating ? `Replace ${rotating.name}` : "Create a token"}
          </h2>
          <label className="block space-y-2">
            <span>Name</span>
            <Input
              required
              value={name}
              maxLength={128}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label className="block space-y-2">
            <span>Expires in days</span>
            <Input
              type="number"
              required
              min={1}
              max={365}
              value={days}
              onChange={(e) => setDays(Number(e.target.value))}
            />
          </label>
          <fieldset className="space-y-2">
            <legend className="font-medium mb-2">Permissions</legend>
            <p className="text-muted-foreground text-sm">
              Select only what the script needs. Write permissions do not include read access.
            </p>
            <div className="grid sm:grid-cols-2 gap-2">
              {scopes
                .filter((s) => s !== "access-token.write")
                .map((scope) => (
                  <label key={scope} className="flex gap-2 items-center">
                    <input
                      type="checkbox"
                      checked={selected.includes(scope)}
                      onChange={(e) =>
                        setSelected((old) =>
                          e.target.checked ? [...old, scope] : old.filter((s) => s !== scope),
                        )
                      }
                    />
                    {scope}
                  </label>
                ))}
            </div>
            <p className="text-muted-foreground text-sm">
              auth.write can change provider credentials; template.publish can change runner
              provisioning; logs.read exposes sanitized diagnostics. These are high-trust
              permissions.
            </p>
          </fieldset>
          <Button type="submit" disabled={busy || !!issued?.token}>
            {busy ? "Saving…" : "Create token"}
          </Button>
        </form>
      )}
      {readable ? (
        <section className="space-y-4">
          <div className="flex gap-4 items-center">
            <h2 className="text-lg font-semibold">Your tokens</h2>
            <label>
              State{" "}
              <select
                aria-label="Token state"
                value={state}
                onChange={(e) => {
                  setState(e.target.value);
                  setCursor(null);
                }}
              >
                {["all", "active", "expired", "revoked", "disabled"].map((s) => (
                  <option key={s}>{s}</option>
                ))}
              </select>
            </label>
          </div>
          {tokens.isPending ? (
            <Loading />
          ) : tokens.error ? (
            <ErrorNotice error={tokens.error} retry={() => void tokens.refetch()} />
          ) : (
            <>
              {!tokens.data?.data.items.length && (
                <p className="text-muted-foreground">No tokens match this state.</p>
              )}
              <div className="divide-y">
                {tokens.data?.data.items.map((token) => (
                  <article
                    key={token.id}
                    className="py-4 flex flex-wrap items-start justify-between gap-4"
                  >
                    <div className="min-w-0 space-y-1">
                      <h3 className="font-medium">
                        {token.name} <span className="text-muted-foreground">({token.state})</span>
                      </h3>
                      <p className="text-sm break-all">{token.id}</p>
                      <p className="text-sm">Expires {token.expires_at}</p>
                      <p className="text-sm">
                        Permissions: {token.scopes.join(", ") || "Identity only"}
                      </p>
                      <p className="text-sm">
                        Effective now: {token.effective_scopes.join(", ") || "Identity only"}
                      </p>
                      <p className="text-muted-foreground text-sm">
                        Last used: {token.last_used_at || "Not recorded"} (approximate)
                      </p>
                    </div>
                    {scopes.includes("access-token.revoke") && token.state !== "revoked" && (
                      <Button variant="outline" disabled={busy} onClick={() => void revoke(token)}>
                        Revoke
                      </Button>
                    )}
                    {enabled &&
                      scopes.includes("access-token.write") &&
                      token.state === "active" && (
                        <Button
                          variant="outline"
                          disabled={busy || !!issued?.token}
                          onClick={() => {
                            setRotating(token);
                            setName(`${token.name} replacement`);
                            setSelected(token.scopes.filter((s) => scopes.includes(s)));
                            setIssued(null);
                            setSaved(false);
                          }}
                        >
                          Rotate
                        </Button>
                      )}
                  </article>
                ))}
              </div>
              <div className="flex gap-3">
                {cursor && (
                  <Button variant="outline" onClick={() => setCursor(null)}>
                    First page
                  </Button>
                )}
                {tokens.data?.data.next_cursor && (
                  <Button
                    variant="outline"
                    onClick={() => setCursor(tokens.data!.data.next_cursor)}
                  >
                    Next page
                  </Button>
                )}
              </div>
            </>
          )}
        </section>
      ) : (
        <p className="text-muted-foreground">Listing token metadata requires access-token.read.</p>
      )}
    </div>
  );
}
