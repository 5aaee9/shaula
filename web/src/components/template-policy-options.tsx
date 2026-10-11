import { useId } from "react";
import { optionLabel, parseInputValue, sameInputValue } from "@/lib/input-values";
import {
  approvedValues,
  policyApprovalHint,
  policyOptionValues,
} from "@/lib/template-policy-approvals";
import type { TemplateVariable } from "@/lib/template-variables";
import { RawInputValuePreview } from "./template-input-values";
import { TemplatePolicyValueInput } from "./template-policy-value-input";
import { TemplateVariableDescription } from "./template-variable-description";

function PolicyOptionCard({
  raw,
  index,
  declared,
  checked,
  onToggle,
}: {
  raw: string;
  index: number;
  declared: boolean;
  checked: boolean;
  onToggle: (checked: boolean) => void;
}) {
  const parsed = parseInputValue(raw);
  const label = parsed.children ? `Configuration ${index + 1}` : optionLabel(parsed);
  return (
    <label className="flex min-w-0 cursor-pointer items-start gap-3 rounded-md border p-3 has-checked:border-primary has-checked:bg-muted/50 has-disabled:cursor-not-allowed has-disabled:opacity-50">
      <input
        type="checkbox"
        aria-label={label}
        checked={checked}
        onChange={(event) => onToggle(event.target.checked)}
        className="mt-0.5 size-4 shrink-0 accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      />
      <span className="min-w-0 space-y-1">
        {parsed.children && <span className="block text-sm font-medium">{label}</span>}
        <RawInputValuePreview raw={raw} />
        {!declared && <span className="block text-xs text-muted-foreground">Added value</span>}
      </span>
    </label>
  );
}

function PolicyOptionChoices({
  variable,
  selected,
  editable,
  onToggle,
}: {
  variable: TemplateVariable;
  selected: string[] | null;
  editable: boolean;
  onToggle: (raw: string, checked: boolean) => void;
}) {
  const isApproved = (raw: string) => selected?.some((item) => sameInputValue(item, raw)) ?? false;
  return (
    <>
      {!!variable.options.length && <p className="text-sm font-medium">Declared options</p>}
      <div className="grid min-w-0 gap-2 sm:grid-cols-2">
        {policyOptionValues(variable, selected).map((raw, index) => (
          <PolicyOptionCard
            key={raw}
            raw={raw}
            index={index}
            declared={variable.options.some((option) => sameInputValue(option.valueJson, raw))}
            checked={isApproved(raw)}
            onToggle={(checked) => onToggle(raw, checked)}
          />
        ))}
      </div>
      {!variable.options.length && (
        <TemplatePolicyValueInput
          variable={variable}
          disabled={!editable}
          onAdd={(raw) => {
            if (!isApproved(raw)) onToggle(raw, true);
          }}
        />
      )}
    </>
  );
}

export function TemplatePolicyOptions({
  variable,
  value,
  disabled = false,
  onChange,
}: {
  variable: TemplateVariable;
  value?: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  const id = useId();
  const selected = approvedValues(value);
  const editable = !disabled && selected !== null;
  function toggle(raw: string, checked: boolean) {
    if (!editable || !selected) return;
    const remaining = selected.filter((item) => !sameInputValue(item, raw));
    onChange(`[${(checked ? [...remaining, raw] : remaining).join(",")}]`);
  }
  return (
    <fieldset
      className="min-w-0 space-y-3 rounded-lg border p-4"
      aria-describedby={`${id}-hint`}
      disabled={!editable}
    >
      <legend className="px-1 text-sm font-medium">{variable.label || variable.key}</legend>
      <TemplateVariableDescription variable={variable} />
      {variable.sensitive ? (
        <p className="text-sm text-muted-foreground">
          Configure approved values in Fleet input policy (JSON) under Advanced settings.
        </p>
      ) : (
        <PolicyOptionChoices
          variable={variable}
          selected={selected}
          editable={editable}
          onToggle={toggle}
        />
      )}
      <p id={`${id}-hint`} className="text-xs text-muted-foreground" role="status">
        {policyApprovalHint(selected, variable.sensitive)}
      </p>
    </fieldset>
  );
}
