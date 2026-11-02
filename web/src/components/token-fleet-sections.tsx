import type { ReactNode } from "react";
import type { Placement } from "@/lib/fleet-form-body";
import type { ProfileChoices } from "@/lib/profile-choice";
import type { RunnerBackend } from "@/lib/runner-backend";
import type { FleetSpec, TemplatePoolSummary } from "@/lib/types";
import type { FleetInputs } from "@/lib/use-fleet-inputs";
import type { FleetPlacementDraft } from "@/lib/use-fleet-placement";
import type { useTokenFleet } from "@/lib/use-token-fleet";
import {
  FleetFormFrame,
  FleetStage,
  PlacementControls,
  RunnerCountField,
} from "./fleet-form-parts";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";
import { ErrorNotice, Field } from "./status";

/** Placement and capacity sections shared by the token backends (Forgejo and GitLab). */
function TokenSharedPoolField({
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
      {!canRead && (
        <p className="text-sm text-muted-foreground">
          Template read permission is required to choose a different pool.
        </p>
      )}
    </Field>
  );
}

function TokenPlacementSection({
  backend,
  editing,
  originalPoolRef,
  draft,
  editor,
  templates,
  pools,
  poolsError,
}: {
  backend: Exclude<RunnerBackend, "github">;
  editing: boolean;
  originalPoolRef?: string;
  draft: FleetPlacementDraft;
  editor: FleetInputs;
  templates: ProfileChoices;
  pools?: TemplatePoolSummary[];
  poolsError: unknown;
}) {
  return (
    <FleetStage title="Template placement">
      <Field label="Template placement" hideLabel>
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
      <PlacementControls
        draft={draft}
        editor={editor}
        templates={templates}
        backend={backend}
        sharedPool={
          <TokenSharedPoolField
            draft={draft}
            originalPoolRef={originalPoolRef}
            pools={pools}
            poolsError={poolsError}
            canRead={editor.canRead}
          />
        }
      />
    </FleetStage>
  );
}

function TokenCapacitySection({
  capacity,
  setCapacity,
}: {
  capacity: FleetSpec["capacity"];
  setCapacity: (capacity: FleetSpec["capacity"]) => void;
}) {
  return (
    <FleetStage title="Capacity">
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
    </FleetStage>
  );
}

/**
 * The token-backend fleet form: key, the provider's authentication section
 * (children), placement and capacity. `buildBody` validates and serializes.
 */
export function TokenFleetFrame({
  backend,
  fleet,
  backendSelector,
  page,
  onClose,
  placeholder,
  buildBody,
  children,
}: {
  backend: Exclude<RunnerBackend, "github">;
  fleet: ReturnType<typeof useTokenFleet>;
  backendSelector?: ReactNode;
  page: boolean;
  onClose: () => void;
  placeholder: string;
  buildBody: () => string;
  children: ReactNode;
}) {
  const { editing, key, pools } = fleet;
  return (
    <FleetFormFrame
      fleetKey={key}
      editing={editing}
      page={page}
      busy={fleet.busy}
      error={fleet.error}
      canSubmit={fleet.allowed}
      onClose={onClose}
      onSubmit={(event) => void fleet.submit(event, buildBody)}
    >
      {backendSelector}
      <FleetStage title="Fleet">
        <Field label="Fleet key">
          <Input
            required
            disabled={editing}
            pattern="[a-z0-9][a-z0-9-]*"
            maxLength={63}
            value={key}
            onChange={(e) => fleet.setKey(e.target.value)}
            placeholder={placeholder}
          />
        </Field>
      </FleetStage>
      {children}
      <TokenPlacementSection
        backend={backend}
        editing={editing}
        originalPoolRef={fleet.original?.template_pool_ref}
        draft={fleet.draft}
        editor={fleet.editor}
        templates={fleet.templates}
        pools={pools.data?.data.pools}
        poolsError={pools.error}
      />
      <TokenCapacitySection capacity={fleet.capacity} setCapacity={fleet.setCapacity} />
    </FleetFormFrame>
  );
}
