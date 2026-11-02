import type { AuthRevisionState } from "@/lib/types";
import { gitlabTargetName } from "@/lib/runner-backend";
import { KeyValue } from "./status";

export function GitlabAuthDetails({ state }: { state: NonNullable<AuthRevisionState["gitlab"]> }) {
  const probe = state.validation;
  return (
    <section className="form-stack mt-6" aria-label="GitLab validation">
      <h3 className="font-medium">Active GitLab target</h3>
      <p className="break-all">{gitlabTargetName(state.target)}</p>
      {probe ? (
        <dl className="details-grid">
          <KeyValue label="Server version">{probe.server_version || "--"}</KeyValue>
          <KeyValue label="Checked at">
            {new Date(probe.checked_at_unix_ms).toLocaleString()}
          </KeyValue>
          <KeyValue label="Valid until">
            {new Date(probe.valid_until_unix_ms).toLocaleString()}
          </KeyValue>
          <KeyValue label="Target ID">{probe.target_id}</KeyValue>
          <KeyValue label="Token expires">
            {probe.token_expires_at || "No expiry reported"}
          </KeyValue>
        </dl>
      ) : (
        <p>No validation observation is available.</p>
      )}
    </section>
  );
}
