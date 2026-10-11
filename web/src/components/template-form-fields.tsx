import type { ReactNode } from "react";
import { AdvancedSettings } from "./advanced-settings";
import { Field } from "./status";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";

export function TemplateDetailsSection({
  id,
  locked,
  profileKey,
  onKeyChange,
  children,
}: {
  id: string;
  /** Existing profiles keep their key for every revision. */
  locked: boolean;
  profileKey: string;
  onKeyChange: (key: string) => void;
  children: ReactNode;
}) {
  return (
    <section
      className="space-y-6 rounded-xl border bg-card p-5 sm:p-6"
      aria-labelledby={`${id}-details`}
    >
      <div>
        <h2 id={`${id}-details`}>Template details</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Give your template a name and choose where its configuration comes from.
        </p>
      </div>
      <Field label="Profile key">
        <Input
          required
          disabled={locked}
          pattern="[a-z0-9][a-z0-9-]*"
          maxLength={63}
          placeholder="e.g. docker-linux"
          autoComplete="off"
          spellCheck={false}
          aria-describedby={`${id}-key-hint`}
          value={profileKey}
          onChange={(event) => onKeyChange(event.target.value)}
        />
        <p id={`${id}-key-hint`} className="text-xs text-muted-foreground">
          {locked
            ? "The profile key stays the same for every revision."
            : "A unique name used by fleets. Lowercase letters, numbers and hyphens, up to 63 characters."}
        </p>
      </Field>
      <div className="border-t pt-6">{children}</div>
    </section>
  );
}

export function ManualPublishSettings({
  open,
  onOpenChange,
  fixedEngine,
  engineRef,
  setEngine,
  bindings,
  setBindings,
  policy,
  setPolicy,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** A default template fixes its engine reference. */
  fixedEngine: boolean;
  engineRef: string;
  setEngine: (value: string) => void;
  bindings: string;
  setBindings: (value: string) => void;
  policy: string;
  setPolicy: (value: string) => void;
}) {
  return (
    <div className="rounded-xl border bg-card px-5 pb-5 pt-3 sm:px-6">
      <AdvancedSettings
        open={open}
        onOpenChange={onOpenChange}
        summary="Engine reference, manual bindings and fleet input policy."
      >
        <Field label="Engine reference">
          <Input
            required
            readOnly={fixedEngine}
            value={engineRef}
            onChange={(event) => setEngine(event.target.value)}
          />
          {fixedEngine && (
            <p className="text-xs text-muted-foreground">
              The default template fixes its engine. Choose Existing artifact to use a custom
              engine.
            </p>
          )}
        </Field>
        <Field label="Bindings (JSON)">
          <Textarea
            autoComplete="off"
            spellCheck={false}
            className="mono min-h-28"
            value={bindings}
            onChange={(event) => setBindings(event.target.value)}
          />
        </Field>
        <Field label="Fleet input policy (JSON)">
          <Textarea
            spellCheck={false}
            className="mono min-h-28"
            value={policy}
            onChange={(event) => setPolicy(event.target.value)}
          />
        </Field>
      </AdvancedSettings>
    </div>
  );
}
