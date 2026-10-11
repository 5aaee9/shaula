import { useMemo } from "react";
import { inputEntries, inputsJson } from "@/lib/input-values";
import { bindingInput, type BindingInput } from "@/lib/template-binding-input";
import type { TemplateVariable, TemplateVariables } from "@/lib/template-variables";
import {
  BooleanBindingSelect,
  ManualBindingNotice,
  TypedBindingInput,
} from "./template-binding-fields";
import { TemplateVariableDescription as VariableDescription } from "./template-variable-description";
import { Field } from "./status";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

function entries(raw: string): Map<string, string> | null {
  try {
    return inputEntries(raw);
  } catch {
    return null;
  }
}

function SensitiveBindingEditor({
  variable,
  input,
  onChange,
}: {
  variable: TemplateVariable;
  input: BindingInput;
  onChange: (value?: string) => void;
}) {
  return (
    <div className="form-stack gap-2">
      <Field label={input.label}>
        <Input
          type="password"
          autoComplete="new-password"
          spellCheck={false}
          required={false}
          placeholder="Leave blank to keep the stored secret"
          value={input.parsed?.text ?? ""}
          onChange={(event) => onChange(event.target.value || undefined)}
        />
      </Field>
      <VariableDescription variable={variable} />
      <p className="text-sm text-muted-foreground">
        Stored secret is never shown. Enter a new value to replace it, or leave blank to keep the
        current one.
      </p>
    </div>
  );
}

function UpdateBindingControl({
  variable,
  input,
  value,
  onChange,
}: {
  variable: TemplateVariable;
  input: BindingInput;
  value?: string;
  onChange: (value?: string) => void;
}) {
  const options = variable.options;
  if (options.length)
    return (
      <Field label={input.label}>
        <NativeSelect
          required={variable.required}
          value={value ?? ""}
          onChange={(event) => onChange(event.target.value || undefined)}
        >
          <NativeSelectOption value="">Not set</NativeSelectOption>
          {options.map((option) => (
            <NativeSelectOption key={option.valueJson} value={option.valueJson}>
              {option.valueJson}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Field>
    );
  if (input.expected === "boolean")
    return (
      <BooleanBindingSelect
        label={input.label}
        required={variable.required}
        value={value}
        onChange={onChange}
      />
    );
  return <TypedBindingInput variable={variable} input={input} onChange={onChange} />;
}

function UpdateBindingEditor({
  variable,
  value,
  onChange,
}: {
  variable: TemplateVariable;
  value?: string;
  onChange: (value?: string) => void;
}) {
  const input = bindingInput(variable, value);
  if (variable.sensitive)
    return <SensitiveBindingEditor variable={variable} input={input} onChange={onChange} />;
  if (input.manual)
    return (
      <ManualBindingNotice variable={variable} label={input.label} raw={value}>
        Edit this value in Bindings (JSON) under Advanced settings.
      </ManualBindingNotice>
    );
  return (
    <div className="form-stack gap-2">
      <UpdateBindingControl variable={variable} input={input} value={value} onChange={onChange} />
      <VariableDescription variable={variable} />
      {value !== undefined && !variable.required && (
        <button
          type="button"
          className="self-start text-xs text-muted-foreground underline"
          onClick={() => onChange(undefined)}
        >
          Clear {input.label}
        </button>
      )}
    </div>
  );
}

export function TemplateUpdateBindings({
  variables,
  bindings,
  onChange,
}: {
  variables: TemplateVariables | null;
  bindings: string;
  onChange: (bindings: string) => void;
}) {
  const fields = variables?.available ? variables.bindings : [];
  const bindingValues = useMemo(() => entries(bindings), [bindings]);
  if (!variables?.available || !fields.length) return null;
  return (
    <section
      className="space-y-4 rounded-xl border bg-card p-5 sm:p-6"
      aria-label="Template bindings"
    >
      <div>
        <h2>Template bindings</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Non-secret values are editable. Secrets stay on the server — enter a new value to replace
          one, or leave it blank to keep it.
        </p>
      </div>
      {!bindingValues ? (
        <p className="text-sm" role="status">
          Bindings JSON is invalid. Correct it below to use visual editing.
        </p>
      ) : (
        <div className="form-stack min-w-0">
          {fields.map((variable) => (
            <UpdateBindingEditor
              key={variable.key}
              variable={variable}
              value={bindingValues.get(variable.key)}
              onChange={(value) => {
                if (value === undefined) bindingValues.delete(variable.key);
                else bindingValues.set(variable.key, value);
                onChange(inputsJson(bindingValues));
              }}
            />
          ))}
        </div>
      )}
    </section>
  );
}
