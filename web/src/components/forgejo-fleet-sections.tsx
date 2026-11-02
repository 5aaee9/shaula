import type { ProfileChoices } from "@/lib/profile-choice";
import { forgejoTargetName, type ForgejoTarget } from "@/lib/runner-backend";
import { FleetStage } from "./fleet-form-parts";
import { ProfileSelector } from "./profile-selector";
import { Input } from "./ui/input";
import { Field } from "./status";

export function ForgejoAuthSection({
  editing,
  fleetKey,
  choices,
  canRead,
  auth,
  setAuth,
  target,
  prefix,
  setPrefix,
  labels,
  setLabels,
}: {
  editing: boolean;
  fleetKey: string;
  choices: ProfileChoices;
  canRead: boolean;
  auth: string;
  setAuth: (auth: string) => void;
  target?: ForgejoTarget;
  prefix: string;
  setPrefix: (prefix: string) => void;
  labels: string;
  setLabels: (labels: string) => void;
}) {
  return (
    <FleetStage title="Authentication">
      <ProfileSelector
        label="Forgejo authentication profile"
        query={choices}
        value={auth}
        canRead={canRead}
        permission="auth.read"
        disabled={editing}
        onChange={setAuth}
      />
      {target && (
        <p className="break-all text-sm" aria-label="Forgejo target">
          {forgejoTargetName(target)}
        </p>
      )}
      <Field label="Runner name prefix">
        <Input
          disabled={editing}
          maxLength={100}
          value={prefix}
          onChange={(e) => setPrefix(e.target.value)}
          placeholder={fleetKey ? `shaula-${fleetKey}-` : "Uses shaula-<fleet key>-"}
        />
      </Field>
      <Field label="Labels">
        <Input
          required
          disabled={editing}
          value={labels}
          onChange={(e) => setLabels(e.target.value)}
          placeholder="linux:host"
        />
      </Field>
    </FleetStage>
  );
}
