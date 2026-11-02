import type { AuthRevisionState } from "@/lib/types";
import { forgejoTargetName } from "@/lib/runner-backend";
import { KeyValue } from "./status";

export function ForgejoAuthDetails({
  state,
}: {
  state: NonNullable<AuthRevisionState["forgejo"]>;
}) {
  const probe = state.validation;
  return (
    <section className="form-stack mt-6" aria-label="Forgejo validation">
      <h3 className="font-medium">Active Forgejo target</h3>
      <p className="break-all">{forgejoTargetName(state.target)}</p>
      {probe ? (
        <dl className="details-grid">
          <KeyValue label="Server version">{probe.server_version || "--"}</KeyValue>
          <KeyValue label="Checked at">
            {new Date(probe.checked_at_unix_ms).toLocaleString()}
          </KeyValue>
          <KeyValue label="Valid until">
            {new Date(probe.valid_until_unix_ms).toLocaleString()}
          </KeyValue>
          <KeyValue label="Principal ID">{probe.principal_id ?? "--"}</KeyValue>
          <KeyValue label="Target ID">{probe.target_id ?? "--"}</KeyValue>
        </dl>
      ) : (
        <p>No validation observation is available.</p>
      )}
    </section>
  );
}
