import type { AuthLiveFleet, TargetSelector } from "@/lib/types";
import { selectorLabel, targetName } from "@/lib/types";
import { policyDifference, selectorAllows, selectorKey } from "@/lib/auth-policy";

export function AuthPolicyPreview({
  previous,
  next,
  fleets,
  loading,
  error,
}: {
  previous: TargetSelector[];
  next: TargetSelector[];
  fleets?: AuthLiveFleet[];
  loading: boolean;
  error: boolean;
}) {
  const difference = policyDifference(previous, next);
  const accounts = [...new Set(next.map((selector) => selector.owner.toLowerCase()))];
  return (
    <section aria-label="Policy change preview" className="form-stack text-sm">
      <h3 className="font-medium">Policy change preview</h3>
      <p>Target accounts: {accounts.join(", ") || "None"}</p>
      {(["added", "removed"] as const).map((kind) => (
        <div key={kind}>
          <p>{kind === "added" ? "Added selectors" : "Removed selectors"}</p>
          {difference[kind].length ? (
            <ul className="ml-4 list-disc">
              {difference[kind].map((selector) => (
                <li key={selectorKey(selector)}>{selectorLabel(selector)}</li>
              ))}
            </ul>
          ) : (
            <p className="text-muted-foreground">None</p>
          )}
        </div>
      ))}
      {!difference.added.length && !difference.removed.length && (
        <p>Target policy unchanged; this publishes a credential rotation.</p>
      )}
      <h4 className="font-medium">Live Fleet impact</h4>
      {loading ? (
        <p>Loading current Fleet references…</p>
      ) : error || !fleets ? (
        <p role="alert">Fleet impact is unavailable. Retry after the connection recovers.</p>
      ) : fleets.length === 0 ? (
        <p>No live Fleets reference this profile.</p>
      ) : (
        <ul className="ml-4 list-disc">
          {fleets.map((fleet) => {
            const covered =
              fleet.target && next.some((selector) => selectorAllows(selector, fleet.target!));
            return (
              <li key={`${fleet.fleetKey}:${JSON.stringify(fleet.target)}`}>
                {fleet.fleetKey} ({fleet.phase}) —{" "}
                {fleet.target ? targetName(fleet.target) : "Target unavailable"}:{" "}
                {fleet.target
                  ? covered
                    ? "Remains covered"
                    : "Blocks activation: target removed"
                  : "Coverage unknown; activation must be verified"}
              </li>
            );
          })}
        </ul>
      )}
      <p className="text-muted-foreground">
        Coverage is structural. GitHub access is verified before activation, and current Fleet
        references are checked again at publication.
      </p>
    </section>
  );
}
