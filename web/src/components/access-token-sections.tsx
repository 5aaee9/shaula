import type { Dispatch, SetStateAction } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import type { Resource } from "@/lib/api";
import type { Issued, Token } from "@/lib/use-access-tokens";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { ErrorNotice, Field, KeyValue, Loading } from "@/components/status";

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
    <section aria-label="Token created" className="form-stack rounded-xl border p-6">
      <h2 className="font-semibold">
        {saved
          ? "New token saved and verified"
          : issued.secret_available
            ? "Save your token now"
            : "Request recovered — secret unavailable"}
      </h2>
      <p className="text-sm text-muted-foreground break-all">
        {issued.access_token.name} · {issued.access_token.id}
      </p>
      {issued.token && (
        <div className="flex flex-wrap items-center gap-2">
          {revealed ? (
            <code className="block w-full break-all select-all rounded-md bg-muted p-3">
              {issued.token}
            </code>
          ) : (
            <Button variant="outline" onClick={onReveal}>
              Reveal token
            </Button>
          )}
          <Button disabled={busy} onClick={onVerify}>
            I have saved it — verify
          </Button>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
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
      </div>
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
      className="form-stack max-w-2xl"
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit();
      }}
    >
      <h2>{rotating ? `Replace ${rotating.name}` : "Create a token"}</h2>
      <Field label="Name">
        <Input required value={name} maxLength={128} onChange={(e) => onName(e.target.value)} />
      </Field>
      <Field label="Expires in days">
        <Input
          type="number"
          required
          min={1}
          max={365}
          value={days}
          onChange={(e) => onDays(Number(e.target.value))}
        />
      </Field>
      <fieldset className="space-y-2">
        <legend className="mb-2 text-sm font-medium">Permissions</legend>
        <div className="grid gap-2 text-sm sm:grid-cols-2">
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
      </fieldset>
      <Button type="submit" className="self-start" disabled={submitDisabled}>
        {busy ? "Saving…" : "Create token"}
      </Button>
    </form>
  );
}

const TOKEN_STATES = ["all", "active", "expired", "revoked", "disabled"];

function scopeList(scopes: string[]) {
  return scopes.join(", ") || "Identity only";
}

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
      <div className="section-heading">
        <h2>Your tokens</h2>
        <Field label="Token state" hideLabel>
          <NativeSelect value={state} onChange={(e) => onState(e.target.value)}>
            {TOKEN_STATES.map((s) => (
              <NativeSelectOption key={s} value={s}>
                {s === "all" ? "All states" : s[0].toUpperCase() + s.slice(1)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </Field>
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
          {token.name} <span className="font-normal text-muted-foreground">({token.state})</span>
        </h3>
        <p className="text-xs text-muted-foreground break-all">{token.id}</p>
        <dl className="details-grid pt-2 text-sm">
          <KeyValue label="Permissions">{scopeList(token.scopes)}</KeyValue>
          {scopeList(token.effective_scopes) !== scopeList(token.scopes) && (
            <KeyValue label="Effective now">{scopeList(token.effective_scopes)}</KeyValue>
          )}
          <KeyValue label="Expires">{token.expires_at}</KeyValue>
          <KeyValue label="Last used">{token.last_used_at || "Never"}</KeyValue>
        </dl>
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
