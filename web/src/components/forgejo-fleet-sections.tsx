import type { Placement } from "@/lib/fleet-form-body";
import type { ProfileChoices } from "@/lib/profile-choice";
import { forgejoTargetName, type ForgejoTarget } from "@/lib/runner-backend";
import type { ForgejoFleetSpec, TemplatePoolSummary } from "@/lib/types";
import type { FleetInputs } from "@/lib/use-fleet-inputs";
import type { FleetPlacementDraft } from "@/lib/use-fleet-placement";
import { PlacementControls, RunnerCountField } from "./fleet-form-parts";
import { ProfileSelector } from "./profile-selector";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";
import { ErrorNotice, Field } from "./status";

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
    <section className="form-stack border-t pt-6">
      <h2>Forgejo authentication</h2>
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
      <p className="text-sm text-muted-foreground">
        The instance and scope come from the profile's Active revision. Target, profile, runner
        prefix and labels are fixed at fleet creation. Rotate the profile's token to replace
        credentials.
      </p>
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
      <p className="text-sm text-muted-foreground">
        Use a unique prefix per fleet. Every workflow runs-on label must be declared by the runner.
        Bundled templates support explicit :host targets only. Do not share labels with persistent
        runners: they may consume the same queued jobs.
      </p>
    </section>
  );
}

function ForgejoSharedPoolField({
  draft,
  originalPoolRef,
  pools,
  poolsError,
  canRead,
}: {
  draft: FleetPlacementDraft;
  originalPoolRef?: string;
  pools?: TemplatePoolSummary[];
  poolsError: unknown;
  canRead: boolean;
}) {
  // A pool the fleet already uses stays selectable even when it is no longer listed.
  const retained =
    originalPoolRef && !pools?.some((pool) => pool.key === originalPoolRef)
      ? originalPoolRef
      : undefined;
  return (
    <Field label="Template pool">
      <NativeSelect
        required
        value={draft.poolRef}
        onChange={(e) => draft.setPoolRef(e.target.value)}
      >
        <NativeSelectOption value="">Choose a pool</NativeSelectOption>
        {retained && (
          <NativeSelectOption value={retained}>{retained} · retained</NativeSelectOption>
        )}
        {(pools || []).map((p) => (
          <NativeSelectOption key={p.key} value={p.key}>
            {p.key} · revision {p.revision}
          </NativeSelectOption>
        ))}
      </NativeSelect>
      {!!poolsError && <ErrorNotice error={poolsError} />}
      <p className="text-sm text-muted-foreground">
        {canRead
          ? "Pool members are checked by the server. Switching pools or following an updated revision waits for this fleet to drain."
          : "Template read permission is required to choose a different pool. The existing reference can be retained."}
      </p>
    </Field>
  );
}

export function ForgejoPlacementSection({
  editing,
  originalPoolRef,
  draft,
  editor,
  templates,
  pools,
  poolsError,
}: {
  editing: boolean;
  originalPoolRef?: string;
  draft: FleetPlacementDraft;
  editor: FleetInputs;
  templates: ProfileChoices;
  pools?: TemplatePoolSummary[];
  poolsError: unknown;
}) {
  return (
    <section className="form-stack border-t pt-6">
      <h2>Template placement</h2>
      <Field label="Template placement">
        <NativeSelect
          value={draft.placement}
          disabled={editing}
          onChange={(e) => draft.setPlacement(e.target.value as Placement)}
        >
          <NativeSelectOption value="single">Single template</NativeSelectOption>
          <NativeSelectOption value="pool">Weighted pool</NativeSelectOption>
          <NativeSelectOption value="shared">Shared pool</NativeSelectOption>
        </NativeSelect>
      </Field>
      <p className="text-sm text-muted-foreground">
        Every member must use the Forgejo runner backend and support this fleet's label targets.
        Weights choose infrastructure for new runners, not which job they receive.
      </p>
      <PlacementControls
        draft={draft}
        editor={editor}
        templates={templates}
        backend="forgejo"
        sharedPool={
          <ForgejoSharedPoolField
            draft={draft}
            originalPoolRef={originalPoolRef}
            pools={pools}
            poolsError={poolsError}
            canRead={editor.canRead}
          />
        }
      />
    </section>
  );
}

export function ForgejoCapacitySection({
  capacity,
  setCapacity,
}: {
  capacity: ForgejoFleetSpec["capacity"];
  setCapacity: (capacity: ForgejoFleetSpec["capacity"]) => void;
}) {
  return (
    <section className="form-stack border-t pt-6">
      <h2>Capacity</h2>
      <div className="form-grid">
        <RunnerCountField
          label="Minimum runners"
          value={capacity.min_runners}
          onChange={(min_runners) => setCapacity({ ...capacity, min_runners })}
        />
        <RunnerCountField
          label="Maximum runners"
          value={capacity.max_runners}
          onChange={(max_runners) => setCapacity({ ...capacity, max_runners })}
        />
      </div>
      <p className="text-sm text-muted-foreground">
        Minimum runners stay waiting and consume infrastructure. Each ephemeral runner executes at
        most one job. The server's maximum lifetime can interrupt running jobs; token rotation may
        require runners to drain and be recreated.
      </p>
    </section>
  );
}
