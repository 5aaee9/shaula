import type { Dispatch, SetStateAction } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import type { Resource } from "@/lib/api";
import type { Issued, Token } from "@/lib/use-access-tokens";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorNotice, Loading } from "@/components/status";

export function IssuedTokenPanel({
  issued,
  saved,
  revealed,
  rotating,
  busy,
  canRevoke,
  onReveal,
  onVerify,
  onRevoke,
}: {
  issued: Issued;
  saved: boolean;
  revealed: boolean;
  rotating: Token | null;
  busy: boolean;
  canRevoke: boolean;
  onReveal: () => void;
  onVerify: () => void;
  onRevoke: (token: Token) => void;
}) {
  return (
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
            <Button variant="outline" onClick={onReveal}>
              Reveal token
            </Button>
          )}
          <Button disabled={busy} onClick={onVerify}>
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
          The server already accepted this request. Revoke this token and create another to receive
          a new secret.
        </p>
      )}
      {saved && rotating && canRevoke && (
        <Button disabled={busy} onClick={() => onRevoke(rotating)}>
          Revoke old token: {rotating.name}
        </Button>
      )}
      {canRevoke && (
        <Button variant="outline" disabled={busy} onClick={() => onRevoke(issued.access_token)}>
          Revoke this token
        </Button>
      )}
    </section>
  );
}

export function AccessTokenIssueForm({
  scopes,
  rotating,
  name,
  days,
  selected,
  onName,
  onDays,
  onSelected,
  submitDisabled,
  busy,
  onSubmit,
}: {
  scopes: string[];
  rotating: Token | null;
  name: string;
  days: number;
  selected: string[];
  onName: (name: string) => void;
  onDays: (days: number) => void;
  onSelected: Dispatch<SetStateAction<string[]>>;
  submitDisabled: boolean;
  busy: boolean;
  onSubmit: () => void;
}) {
  return (
    <form
      className="max-w-2xl space-y-4"
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit();
      }}
    >
      <h2 className="text-lg font-semibold">
        {rotating ? `Replace ${rotating.name}` : "Create a token"}
      </h2>
      <label className="block space-y-2">
        <span>Name</span>
        <Input required value={name} maxLength={128} onChange={(e) => onName(e.target.value)} />
      </label>
      <label className="block space-y-2">
        <span>Expires in days</span>
        <Input
          type="number"
          required
          min={1}
          max={365}
          value={days}
          onChange={(e) => onDays(Number(e.target.value))}
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
                    onSelected((old) =>
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
          provisioning; logs.read exposes sanitized diagnostics. These are high-trust permissions.
        </p>
      </fieldset>
      <Button type="submit" disabled={submitDisabled}>
        {busy ? "Saving…" : "Create token"}
      </Button>
    </form>
  );
}

const TOKEN_STATES = ["all", "active", "expired", "revoked", "disabled"];

export function AccessTokenList({
  tokens,
  state,
  onState,
  cursor,
  onCursor,
  rowActions,
}: {
  tokens: UseQueryResult<Resource<{ items: Token[]; next_cursor: string | null }>>;
  state: string;
  onState: (state: string) => void;
  cursor: string | null;
  onCursor: (cursor: string | null) => void;
  rowActions: TokenRowActions;
}) {
  return (
    <section className="space-y-4">
      <div className="flex gap-4 items-center">
        <h2 className="text-lg font-semibold">Your tokens</h2>
        <label>
          State{" "}
          <select aria-label="Token state" value={state} onChange={(e) => onState(e.target.value)}>
            {TOKEN_STATES.map((s) => (
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
          {!tokens.data.data.items.length && (
            <p className="text-muted-foreground">No tokens match this state.</p>
          )}
          <div className="divide-y">
            {tokens.data.data.items.map((token) => (
              <TokenRow key={token.id} token={token} actions={rowActions} />
            ))}
          </div>
          <div className="flex gap-3">
            {cursor && (
              <Button variant="outline" onClick={() => onCursor(null)}>
                First page
              </Button>
            )}
            {tokens.data.data.next_cursor && (
              <Button variant="outline" onClick={() => onCursor(tokens.data.data.next_cursor)}>
                Next page
              </Button>
            )}
          </div>
        </>
      )}
    </section>
  );
}

interface TokenRowActions {
  busy: boolean;
  canRevoke: boolean;
  canRotate: boolean;
  rotateDisabled: boolean;
  onRevoke: (token: Token) => void;
  onRotate: (token: Token) => void;
}

function TokenRow({ token, actions }: { token: Token; actions: TokenRowActions }) {
  return (
    <article className="py-4 flex flex-wrap items-start justify-between gap-4">
      <div className="min-w-0 space-y-1">
        <h3 className="font-medium">
          {token.name} <span className="text-muted-foreground">({token.state})</span>
        </h3>
        <p className="text-sm break-all">{token.id}</p>
        <p className="text-sm">Expires {token.expires_at}</p>
        <p className="text-sm">Permissions: {token.scopes.join(", ") || "Identity only"}</p>
        <p className="text-sm">
          Effective now: {token.effective_scopes.join(", ") || "Identity only"}
        </p>
        <p className="text-muted-foreground text-sm">
          Last used: {token.last_used_at || "Not recorded"} (approximate)
        </p>
      </div>
      {actions.canRevoke && token.state !== "revoked" && (
        <Button variant="outline" disabled={actions.busy} onClick={() => actions.onRevoke(token)}>
          Revoke
        </Button>
      )}
      {actions.canRotate && token.state === "active" && (
        <Button
          variant="outline"
          disabled={actions.rotateDisabled}
          onClick={() => actions.onRotate(token)}
        >
          Rotate
        </Button>
      )}
    </article>
  );
}
