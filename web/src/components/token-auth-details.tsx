import type { ReactNode } from "react";
import type { AuthRevisionState } from "@/lib/types";
import { forgejoTargetName, gitlabTargetName } from "@/lib/runner-backend";
import { KeyValue } from "./status";

function TokenValidationDetails({
  provider,
  target,
  probe,
  extra,
}: {
  provider: "Forgejo" | "GitLab";
  target: string;
  probe?: {
    server_version: string;
    checked_at_unix_ms: number;
    valid_until_unix_ms: number;
  } | null;
  extra?: ReactNode;
}) {
  return (
    <section className="form-stack mt-6" aria-label={`${provider} validation`}>
      <h3 className="font-medium">Active {provider} target</h3>
      <p className="break-all">{target}</p>
      {probe ? (
        <dl className="details-grid">
          <KeyValue label="Server version">{probe.server_version || "--"}</KeyValue>
          <KeyValue label="Checked at">
            {new Date(probe.checked_at_unix_ms).toLocaleString()}
          </KeyValue>
          <KeyValue label="Valid until">
            {new Date(probe.valid_until_unix_ms).toLocaleString()}
          </KeyValue>
          {extra}
        </dl>
      ) : (
        <p>No validation observation is available.</p>
      )}
    </section>
  );
}

export function ForgejoAuthDetails({
  state,
}: {
  state: NonNullable<AuthRevisionState["forgejo"]>;
}) {
  const probe = state.validation;
  return (
    <TokenValidationDetails
      provider="Forgejo"
      target={forgejoTargetName(state.target)}
      probe={probe}
      extra={
        probe && (
          <>
            <KeyValue label="Principal ID">{probe.principal_id ?? "--"}</KeyValue>
            <KeyValue label="Target ID">{probe.target_id ?? "--"}</KeyValue>
          </>
        )
      }
    />
  );
}

export function GitlabAuthDetails({ state }: { state: NonNullable<AuthRevisionState["gitlab"]> }) {
  const probe = state.validation;
  return (
    <TokenValidationDetails
      provider="GitLab"
      target={gitlabTargetName(state.target)}
      probe={probe}
      extra={
        probe && (
          <>
            <KeyValue label="Target ID">{probe.target_id}</KeyValue>
            <KeyValue label="Token expires">
              {probe.token_expires_at || "No expiry reported"}
            </KeyValue>
          </>
        )
      }
    />
  );
}
