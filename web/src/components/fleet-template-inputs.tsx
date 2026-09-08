import type { FleetInputs } from "@/lib/use-fleet-inputs";
import { ApprovedValueSelect, RawInputValuePreview } from "./template-input-values";
import { Button } from "./ui/button";

export function FleetTemplateInputs({ editor }: { editor: FleetInputs }) {
  const contract = editor.selection?.contract;
  if (!contract)
    return editor.values.size ? (
      <section className="form-stack" aria-label="Existing template inputs">
        <p className="text-sm">
          Existing template inputs are preserved and cannot currently be edited.
        </p>
        <RawInputValuePreview raw={editor.json} />
      </section>
    ) : null;
  if (contract.mode === "presets")
    return (
      <fieldset disabled={editor.loading || !!editor.pending}>
        <ApprovedValueSelect
          label="Input configuration"
          description="Choose one complete approved configuration. Its individual values cannot be edited."
          required
          options={contract.presets}
          value={editor.presetChosen ? editor.json : undefined}
          onChange={editor.replaceValues}
        />
      </fieldset>
    );
  const orphans = [...editor.values].filter(
    ([key]) => !contract.fields.some((field) => field.key === key),
  );
  return (
    <fieldset disabled={editor.loading || !!editor.pending} className="form-stack">
      {!contract.fields.length && !editor.values.size && (
        <p className="text-sm text-muted-foreground">This template needs no input configuration.</p>
      )}
      {contract.fields.map((field) => (
        <ApprovedValueSelect
          key={field.key}
          label={field.label}
          inputKey={field.key}
          description={field.description}
          required={field.required}
          options={field.options}
          value={editor.values.get(field.key)}
          onChange={(value) => editor.updateValue(field.key, value)}
        />
      ))}
      {orphans.map(([key, value]) => (
        <div key={key} className="form-stack gap-2 rounded-md border p-3">
          <p className="font-medium break-all">{key}</p>
          <p className="text-sm">
            Existing value is no longer selectable. It is preserved until explicitly removed.
          </p>
          <RawInputValuePreview raw={value} />
          <Button type="button" variant="outline" onClick={() => editor.updateValue(key)}>
            Remove {key}
          </Button>
        </div>
      ))}
    </fieldset>
  );
}
