import type { FormEvent, ReactNode } from "react";
import { LoaderCircle, Save } from "lucide-react";
import type { ProfileChoices } from "@/lib/profile-choice";
import type { RunnerBackend } from "@/lib/runner-backend";
import type { FleetInputs } from "@/lib/use-fleet-inputs";
import type { FleetPlacementDraft } from "@/lib/use-fleet-placement";
import { FleetTemplateInputs } from "./fleet-template-inputs";
import { FleetTemplateSelection } from "./fleet-template-selection";
import { Modal } from "./modal";
import { TemplatePoolEditor } from "./template-pool-editor";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { ErrorNotice, Field } from "./status";

/** The dialog, disabled-while-busy fieldset and action bar shared by every fleet form. */
export function FleetFormFrame({
  fleetKey,
  editing,
  page,
  busy,
  error,
  canSubmit,
  onClose,
  onSubmit,
  children,
}: {
  fleetKey: string;
  editing: boolean;
  page: boolean;
  busy: boolean;
  error: unknown;
  canSubmit: boolean;
  onClose: () => void;
  onSubmit: (event: FormEvent) => void;
  children: ReactNode;
}) {
  return (
    <Modal
      open
      inline={page}
      title={editing ? `Edit ${fleetKey}` : "Create fleet"}
      className="sm:max-w-2xl"
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <form className="form-stack" onSubmit={onSubmit}>
        <fieldset disabled={busy} className="form-stack">
          {children}
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy || !canSubmit}>
            {busy ? <LoaderCircle className="animate-spin" /> : <Save />}
            {editing ? "Save changes" : "Create fleet"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

/** One form stage: a quiet left rail that names the facet, beside its controls. */
export function FleetStage({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section className="grid grid-cols-1 gap-4 border-t pt-6 first:border-t-0 first:pt-0 sm:grid-cols-[11rem_1fr] sm:gap-6">
      <div className="min-w-0">
        <h2 className="text-sm font-medium">{title}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
      </div>
      <div className="form-stack min-w-0">{children}</div>
    </section>
  );
}

export function RunnerCountField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <Field label={label}>
      <Input
        required
        type="number"
        min={0}
        step={1}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </Field>
  );
}

/** The controls for the chosen placement; the shared-pool picker differs per provider. */
export function PlacementControls({
  draft,
  editor,
  templates,
  backend,
  sharedPool,
}: {
  draft: FleetPlacementDraft;
  editor: FleetInputs;
  templates: ProfileChoices;
  backend?: RunnerBackend;
  sharedPool: ReactNode;
}) {
  const { placement, pool, setPool } = draft;
  if (placement === "single")
    return (
      <>
        <FleetTemplateSelection editor={editor} templates={templates} />
        <FleetTemplateInputs editor={editor} />
      </>
    );
  if (placement === "pool")
    return (
      <TemplatePoolEditor
        members={pool.members}
        setMembers={(members) => setPool({ ...pool, members })}
        failurePolicy={pool.failure_policy}
        setFailurePolicy={(failure_policy) => setPool({ ...pool, failure_policy })}
        canRead={editor.canRead}
        backend={backend}
      />
    );
  return sharedPool;
}
