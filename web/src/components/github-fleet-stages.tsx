import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { cn } from "cn";
import type { Placement } from "@/lib/fleet-form-body";
import type { ProfileChoices } from "@/lib/profile-choice";
import type { GitHubFleetSpec, GitHubTarget, TemplatePoolSummary } from "@/lib/types";
import type { FleetInputs } from "@/lib/use-fleet-inputs";
import type { FleetPlacementDraft } from "@/lib/use-fleet-placement";
import { AdvancedSettings } from "./advanced-settings";
import { FleetStage, PlacementControls, RunnerCountField } from "./fleet-form-parts";
import { ProfileSelector } from "./profile-selector";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Field } from "./status";

type GitHubSettings = GitHubFleetSpec["github"];

export function GitHubTargetStage({
  editing,
  target,
  onChange,
}: {
  editing: boolean;
  target: GitHubTarget;
  onChange: (target: GitHubTarget) => void;
}) {
  return (
    <FleetStage
      title="GitHub target"
      description={
        editing
          ? "The organization or repository this fleet serves is fixed at creation."
          : "Where GitHub Actions jobs for this fleet run."
      }
    >
      <div className="form-grid">
        <Field label="GitHub target">
          <NativeSelect
            disabled={editing}
            value={target.kind}
            onChange={(event) =>
              onChange(
                event.target.value === "repository"
                  ? { kind: "repository", owner: target.owner, repository: "" }
                  : { kind: "organization", owner: target.owner },
              )
            }
          >
            <NativeSelectOption value="organization">Organization</NativeSelectOption>
            <NativeSelectOption value="repository">Repository</NativeSelectOption>
          </NativeSelect>
        </Field>
        <Field label="Owner">
          <Input
            required
            disabled={editing}
            value={target.owner}
            onChange={(event) => onChange({ ...target, owner: event.target.value })}
            placeholder="acme"
          />
        </Field>
      </div>
      {target.kind === "repository" && (
        <Field label="Repository">
          <Input
            required
            disabled={editing}
            value={target.repository}
            onChange={(event) =>
              onChange({ kind: "repository", owner: target.owner, repository: event.target.value })
            }
          />
        </Field>
      )}
    </FleetStage>
  );
}

export function GitHubAuthStage({
  value,
  original,
  query,
  canRead,
  onChange,
}: {
  value: string;
  /** The profile the fleet was loaded with, offered back after a change. */
  original?: string;
  query: ProfileChoices;
  canRead: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <FleetStage
      title="Authentication"
      description="The GitHub App profile used to manage runners for this fleet."
    >
      <ProfileSelector
        label="GitHub authentication profile"
        value={value}
        query={query}
        canRead={canRead}
        permission="auth.read"
        onChange={onChange}
      />
      {original && value !== original && (
        <Button
          type="button"
          variant="ghost"
          className="self-start"
          onClick={() => onChange(original)}
        >
          Restore original authentication
        </Button>
      )}
    </FleetStage>
  );
}

const placementDescriptions: Record<Placement, string> = {
  single: "All runners in this fleet use one template profile.",
  pool: "Each new runner draws one member by weight. Weights shape the long-run mix across backends.",
  shared:
    "This fleet draws runners from a shared pool resource; multiple fleets can use the same mix.",
};

const placementOptions = [
  ["single", "Single template"],
  ["pool", "Weighted pool"],
  ["shared", "Shared pool"],
] as const;

export function GitHubPlacementStage({
  editing,
  originalPoolRef,
  draft,
  editor,
  templates,
  pools,
}: {
  editing: boolean;
  /** The shared pool the loaded fleet already uses, if any. */
  originalPoolRef?: string;
  draft: FleetPlacementDraft;
  editor: FleetInputs;
  templates: ProfileChoices;
  pools: TemplatePoolSummary[];
}) {
  const { placement, setPlacement, poolRef, setPoolRef } = draft;
  return (
    <FleetStage title="Template placement" description={placementDescriptions[placement]}>
      {!editing && (
        <div
          role="radiogroup"
          aria-label="Template placement"
          className="inline-flex w-fit rounded-md border bg-muted/40 p-0.5"
        >
          {placementOptions.map(([value, label]) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={placement === value}
              onClick={() => setPlacement(value)}
              className={cn(
                "rounded-[5px] px-3 py-1.5 text-sm text-muted-foreground transition-colors",
                placement === value && "bg-background font-medium text-foreground shadow-sm",
              )}
            >
              {label}
            </button>
          ))}
        </div>
      )}
      <PlacementControls
        draft={draft}
        editor={editor}
        templates={templates}
        sharedPool={
          <Field label="Template pool">
            <NativeSelect
              required
              value={poolRef}
              onChange={(event) => setPoolRef(event.target.value)}
            >
              <NativeSelectOption value="">Choose a pool</NativeSelectOption>
              {pools.map((pool) => (
                <NativeSelectOption key={pool.key} value={pool.key}>
                  {pool.key} · revision {pool.revision}
                </NativeSelectOption>
              ))}
            </NativeSelect>
            {originalPoolRef && (
              <p className="text-muted-foreground text-sm">
                Switching pools re-routes every future draw and needs the fleet to be empty.
              </p>
            )}
            <p className="text-muted-foreground text-sm">
              The pool defines members, weights and caps. Each member follows its template's latest
              Active revision; this fleet catches up automatically when it drains.
            </p>
          </Field>
        }
      />
    </FleetStage>
  );
}

export function GitHubCapacityStage({
  editing,
  fleetKey,
  spec,
  setSpec,
  github,
  labels,
  setLabels,
  advancedOpen,
  setAdvancedOpen,
}: {
  editing: boolean;
  fleetKey: string;
  spec: GitHubFleetSpec;
  setSpec: (spec: GitHubFleetSpec) => void;
  github: (patch: Partial<GitHubSettings>) => void;
  labels: string;
  setLabels: (labels: string) => void;
  advancedOpen: boolean;
  setAdvancedOpen: (open: boolean) => void;
}) {
  const customSettings = [
    !!spec.github.scale_set_name && spec.github.scale_set_name !== fleetKey,
    spec.github.runner_group !== "Default",
    spec.capacity.min_runners !== 0,
    !!labels.trim(),
  ].filter(Boolean).length;
  return (
    <FleetStage title="Capacity" description="How many concurrent runners this fleet may run.">
      <div className="form-grid">
        <RunnerCountField
          label="Maximum runners"
          value={spec.capacity.max_runners}
          onChange={(max_runners) =>
            setSpec({ ...spec, capacity: { ...spec.capacity, max_runners } })
          }
        />
      </div>
      <AdvancedSettings
        open={advancedOpen}
        onOpenChange={setAdvancedOpen}
        summary={
          customSettings
            ? `${customSettings} custom settings`
            : "Default runner group, 0 minimum runners"
        }
      >
        <div className="form-grid">
          <Field label="Scale set name">
            <Input
              disabled={editing}
              maxLength={100}
              value={spec.github.scale_set_name}
              placeholder={fleetKey || "Fleet key"}
              onChange={(event) => github({ scale_set_name: event.target.value })}
            />
            {!editing && (
              <p className="text-muted-foreground text-sm">Uses the fleet key when left empty.</p>
            )}
          </Field>
          <Field label="Runner group">
            <Input
              required
              disabled={editing}
              maxLength={100}
              value={spec.github.runner_group}
              onChange={(event) => github({ runner_group: event.target.value })}
            />
          </Field>
        </div>
        <div className="form-grid">
          <RunnerCountField
            label="Minimum runners"
            value={spec.capacity.min_runners}
            onChange={(min_runners) =>
              setSpec({ ...spec, capacity: { ...spec.capacity, min_runners } })
            }
          />
        </div>
        <Field label="Labels">
          <Input
            value={labels}
            onChange={(event) => setLabels(event.target.value)}
            placeholder="linux, x64"
          />
          <p className="text-muted-foreground text-sm">
            Separate labels with commas. Leave empty to use the scale set name as the label.
            {editing &&
              " Changes sync to GitHub after saving. Running jobs continue while labels update."}
          </p>
        </Field>
      </AdvancedSettings>
    </FleetStage>
  );
}
