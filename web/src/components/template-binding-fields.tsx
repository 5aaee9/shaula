import type { ReactNode } from "react";
import { typedBindingValue, type BindingInput } from "@/lib/template-binding-input";
import type { TemplateVariable } from "@/lib/template-variables";
import { RawInputValuePreview } from "./template-input-values";
import { TemplateVariableDescription as VariableDescription } from "./template-variable-description";
import { Field } from "./status";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

/** A binding that cannot be edited visually; `raw` previews its current JSON. */
export function ManualBindingNotice({
  variable,
  label,
  raw,
  children,
}: {
  variable: TemplateVariable;
  label: string;
  raw?: string;
  children: ReactNode;
}) {
  return (
    <div className="form-stack gap-2">
      <h4 className="text-sm font-medium">{label}</h4>
      <VariableDescription variable={variable} />
      <p className="text-sm text-muted-foreground">{children}</p>
      {raw !== undefined && <RawInputValuePreview raw={raw} />}
    </div>
  );
}

export function BooleanBindingSelect({
  label,
  required,
  value,
  onChange,
}: {
  label: string;
  required: boolean;
  value?: string;
  onChange: (value?: string) => void;
}) {
  return (
    <Field label={label}>
      <NativeSelect
        required={required}
        value={value ?? ""}
        onChange={(event) => onChange(event.target.value || undefined)}
      >
        <NativeSelectOption value="">Not set</NativeSelectOption>
        <NativeSelectOption value="true">Yes</NativeSelectOption>
        <NativeSelectOption value="false">No</NativeSelectOption>
      </NativeSelect>
    </Field>
  );
}

export function TypedBindingInput({
  variable,
  input,
  placeholder,
  onChange,
}: {
  variable: TemplateVariable;
  input: BindingInput;
  placeholder?: string;
  onChange: (value?: string) => void;
}) {
  const { label, expected, parsed } = input;
  return (
    <Field label={label}>
      <Input
        type={variable.sensitive ? "password" : expected === "number" ? "number" : "text"}
        autoComplete="off"
        spellCheck={false}
        step={variable.typeName === "integer" ? 1 : "any"}
        required={variable.required}
        placeholder={placeholder}
        value={parsed?.text ?? ""}
        onChange={(event) => {
          const next = typedBindingValue(expected, event.target.value);
          if (next !== null) onChange(next);
        }}
      />
    </Field>
  );
}
