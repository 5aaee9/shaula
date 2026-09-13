import { useMemo } from "react";
import { inputEntries, inputsJson, parseInputValue } from "@/lib/input-values";
import type { TemplateVariable, TemplateVariables } from "@/lib/template-variables";
import { isSensitiveMarker } from "@/lib/types";
import { RawInputValuePreview } from "./template-input-values";
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

/**
 * Initial bindings JSON for an Update review. Non-sensitive fields prefill with
 * their stored value; sensitive fields are omitted so the server keeps them
 * (spec 0038: omitted/`null` sensitive field = retain base value).
 */
export function initialUpdateBindings(
  revisionBindings: Record<string, unknown> | null | undefined,
): string {
  const values = new Map<string, string>();
  for (const [key, value] of Object.entries(revisionBindings ?? {})) {
    if (isSensitiveMarker(value)) continue;
    values.set(key, JSON.stringify(value));
  }
  return inputsJson(values);
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
  const label = variable.label || variable.key;
  const scalar = ["string", "number", "integer", "boolean", "bool"].includes(variable.typeName);
  let parsed;
  try {
    parsed = value === undefined ? undefined : parseInputValue(value);
  } catch {
    /* Manual JSON is preserved. */
  }
  const expected =
    variable.typeName === "integer"
      ? "number"
      : variable.typeName === "bool"
        ? "boolean"
        : variable.typeName;
  const incompatible = parsed !== undefined && parsed.kind !== expected;

  if (variable.sensitive) {
    return (
      <div className="form-stack gap-2">
        <Field label={label}>
          <Input
            type="password"
            autoComplete="new-password"
            spellCheck={false}
            required={false}
            placeholder="Leave blank to keep the stored secret"
            value={parsed?.text ?? ""}
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

  if (!scalar || incompatible)
    return (
      <div className="form-stack gap-2">
        <h4 className="text-sm font-medium">{label}</h4>
        <VariableDescription variable={variable} />
        <p className="text-sm text-muted-foreground">
          Edit this value in Bindings (JSON) under Advanced settings.
        </p>
        {value !== undefined && <RawInputValuePreview raw={value} />}
      </div>
    );

  const options = variable.options;
  return (
    <div className="form-stack gap-2">
      {options.length ? (
        <Field label={label}>
          <NativeSelect
            required={variable.required}
            value={value ?? ""}
            onChange={(event) => onChange(event.target.value || undefined)}
          >
            <NativeSelectOption value="">Not set</NativeSelectOption>
            {options.map((option, index) => (
              <NativeSelectOption key={index} value={option.valueJson}>
                {option.valueJson}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </Field>
      ) : expected === "boolean" ? (
        <Field label={label}>
          <NativeSelect
            required={variable.required}
            value={value ?? ""}
            onChange={(event) => onChange(event.target.value || undefined)}
          >
            <NativeSelectOption value="">Not set</NativeSelectOption>
            <NativeSelectOption value="true">Yes</NativeSelectOption>
            <NativeSelectOption value="false">No</NativeSelectOption>
          </NativeSelect>
        </Field>
      ) : (
        <Field label={label}>
          <Input
            type={expected === "number" ? "number" : "text"}
            autoComplete="off"
            spellCheck={false}
            step={variable.typeName === "integer" ? 1 : "any"}
            required={variable.required}
            value={parsed?.text ?? ""}
            onChange={(event) => {
              const text = event.target.value;
              if (expected === "string") onChange(JSON.stringify(text));
              else if (!text) onChange(undefined);
              else if (/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(text)) onChange(text);
            }}
          />
        </Field>
      )}
      <VariableDescription variable={variable} />
      {value !== undefined && !variable.required && (
        <button
          type="button"
          className="self-start text-xs text-muted-foreground underline"
          onClick={() => onChange(undefined)}
        >
          Clear {label}
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
