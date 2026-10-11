import { useState } from "react";
import { useAccessTokenIssuance, useAccessTokenList, type Token } from "@/lib/use-access-tokens";
import {
  AccessTokenIssueForm,
  AccessTokenList,
  IssuedTokenPanel,
} from "@/components/access-token-sections";

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
  const readable = scopes.includes("access-token.read");
  const writable = enabled && scopes.includes("access-token.write");
  const canRevoke = scopes.includes("access-token.revoke");
  const list = useAccessTokenList(readable);
  const issuance = useAccessTokenIssuance(() => void list.tokens.refetch());
  const { issued, busy, rotating } = issuance;
  // A pending secret must be verified or revoked before another can be issued.
  const secretPending = !!issued?.token;
  function rotate(token: Token) {
    issuance.startRotation(token);
    setName(`${token.name} replacement`);
    setSelected(token.scopes.filter((s) => scopes.includes(s)));
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
      {issuance.error && (
        <p role="alert" className="text-destructive">
          {issuance.error}
        </p>
      )}
      {issued && (
        <IssuedTokenPanel
          issued={issued}
          saved={issuance.saved}
          revealed={issuance.revealed}
          rotating={rotating}
          busy={busy}
          canRevoke={canRevoke}
          onReveal={issuance.reveal}
          onVerify={() => void issuance.verifySaved()}
          onRevoke={(token) => void issuance.revoke(token)}
        />
      )}
      {!enabled && (
        <p>
          Personal access tokens are disabled by the deployment. Existing token metadata remains
          available for review and revocation.
        </p>
      )}
      {writable && (
        <AccessTokenIssueForm
          scopes={scopes}
          rotating={rotating}
          name={name}
          days={days}
          selected={selected}
          onName={setName}
          onDays={setDays}
          onSelected={setSelected}
          submitDisabled={busy || secretPending}
          busy={busy}
          onSubmit={() => void issuance.create({ name, days, selected })}
        />
      )}
      {readable ? (
        <AccessTokenList
          tokens={list.tokens}
          state={list.state}
          onState={list.filter}
          cursor={list.cursor}
          onCursor={list.setCursor}
          rowActions={{
            busy,
            canRevoke,
            canRotate: writable,
            rotateDisabled: busy || secretPending,
            onRevoke: (token) => void issuance.revoke(token),
            onRotate: rotate,
          }}
        />
      ) : (
        <p className="text-muted-foreground">Listing token metadata requires access-token.read.</p>
      )}
    </div>
  );
}
