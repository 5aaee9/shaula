import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { authenticatedFetch } from "@/lib/authentication";

export interface Token {
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
export interface Issued {
  access_token: Token;
  secret_available: boolean;
  token?: string;
}
export interface TokenDraft {
  name: string;
  days: number;
  selected: string[];
}

export function useAccessTokenList(readable: boolean) {
  const [cursor, setCursor] = useState<string | null>(null);
  const [state, setState] = useState("all");
  const tokens = useQuery({
    queryKey: ["access-tokens", state, cursor],
    enabled: readable,
    queryFn: ({ signal }) =>
      api<{ items: Token[]; next_cursor: string | null }>(
        `/access-tokens?${new URLSearchParams({ state, ...(cursor ? { cursor } : {}) })}`,
        { signal },
      ),
  });
  function filter(next: string) {
    setState(next);
    setCursor(null);
  }
  return { tokens, state, filter, cursor, setCursor };
}

/**
 * Issue, verify, rotate, and revoke tokens. The issued secret lives only in
 * this state and is dropped once the holder proves they saved it.
 */
export function useAccessTokenIssuance(refetchTokens: () => void) {
  const [issued, setIssued] = useState<Issued | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [saved, setSaved] = useState(false);
  const [rotating, setRotating] = useState<Token | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const attempt = useRef({ body: "", key: "" });
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  async function create({ name, days, selected }: TokenDraft) {
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
      refetchTokens();
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
      refetchTokens();
    } catch (e) {
      if (mounted.current) setError(errorMessage(e));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  async function verifySaved() {
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
  function startRotation(token: Token) {
    setRotating(token);
    setIssued(null);
    setSaved(false);
  }
  return {
    issued,
    revealed,
    reveal: () => setRevealed(true),
    saved,
    rotating,
    busy,
    error,
    create,
    revoke,
    verifySaved,
    startRotation,
  };
}
