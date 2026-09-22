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
          <KeyValue label="Validated server version">{probe.server_version || "--"}</KeyValue>
          <KeyValue label="Checked at">
            {new Date(probe.checked_at_unix_ms).toLocaleString()}
          </KeyValue>
          <KeyValue label="Validation valid until">
            {new Date(probe.valid_until_unix_ms).toLocaleString()}
          </KeyValue>
          <KeyValue label="Principal ID">{probe.principal_id ?? "--"}</KeyValue>
          <KeyValue label="Target ID">{probe.target_id ?? "--"}</KeyValue>
        </dl>
      ) : (
        <p>No validation observation is available.</p>
      )}
      <p className="text-sm text-muted-foreground">
        This is historical validation, not a live access guarantee. Rotation preserves the target
        and may require existing runners to drain and be recreated.
      </p>
    </section>
  );
}
