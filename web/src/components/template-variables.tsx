import { inputEntries, inputsJson, parseInputValue } from "@/lib/input-values";
import type { TemplateVariable, TemplateVariables as Variables } from "@/lib/template-variables";
import { ApprovedValueSelect, RawInputValuePreview } from "./template-input-values";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";
import { Field } from "./status";

function entries(raw: string): Map<string, string> | null {
  try {
    return inputEntries(raw);
  } catch {
    return null;
  }
}

function VariableDescription({ variable }: { variable: TemplateVariable }) {
  return (
    <div className="min-w-0 space-y-2 text-sm">
      <p className="text-xs text-muted-foreground break-all">
        {variable.key} · {variable.typeName} · {variable.required ? "Required" : "Optional"}
        {variable.sensitive && " · Sensitive"}
      </p>
      {variable.description && (
        <p className="text-muted-foreground whitespace-pre-wrap break-words">
          {variable.description}
        </p>
      )}
      {!variable.sensitive && variable.defaultValueJson !== undefined && (
        <div>
          <p className="mb-1 font-medium">Terraform default</p>
          <RawInputValuePreview raw={variable.defaultValueJson} />
        </div>
      )}
    </div>
  );
}

function BindingEditor({
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
  if (!scalar || incompatible)
    return (
      <div className="form-stack gap-2">
        <h4 className="text-sm font-medium">{label}</h4>
        <VariableDescription variable={variable} />
        <p className="text-sm text-muted-foreground">
          Configure this value in Bindings (JSON) under Advanced settings.
        </p>
        {value !== undefined && !variable.sensitive && <RawInputValuePreview raw={value} />}
      </div>
    );
  const options = variable.sensitive ? [] : variable.options;
  return (
    <div className="form-stack gap-2">
      {options.length ? (
        <ApprovedValueSelect
          label={label}
          required={variable.required}
          options={options}
          value={value}
          onChange={onChange}
        />
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
            type={variable.sensitive ? "password" : expected === "number" ? "number" : "text"}
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
      {value !== undefined && (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="self-start"
          onClick={() => onChange(undefined)}
        >
          Clear {label}
        </Button>
      )}
    </div>
  );
}

export function TemplateVariables({
  variables,
  bindings,
  policy,
  setBindings,
  setPolicy,
}: {
  variables: Variables;
  bindings: string;
  policy: string;
  setBindings: (value: string) => void;
  setPolicy: (value: string) => void;
}) {
  const bindingValues = entries(bindings);
  const policyValues = entries(policy);
  const defaults = variables.bindings.filter(
    // An omitted optional attribute already gets Terraform's null default.
    // Explicit null can be rejected by the binding schema's declared type.
    (variable) =>
      !variable.sensitive &&
      variable.defaultValueJson !== undefined &&
      variable.defaultValueJson !== "null",
  );
  const declared = variables.parameters.filter(
    (variable) => !variable.sensitive && variable.options.length,
  );
  if (!variables.available)
    return (
      <p className="text-sm text-muted-foreground" role="status">
        {variables.reason || "Variable discovery is unavailable for this template."} You can still
        publish with manual bindings and input policy.
      </p>
    );
  return (
    <div className="form-stack min-w-0">
      <section className="form-stack min-w-0" aria-label="Template bindings">
        <div>
          <h3 className="text-sm font-semibold">Template bindings</h3>
          <p className="text-sm text-muted-foreground">
            Configure the environment used by every fleet on this template.
          </p>
        </div>
        {!!defaults.length && (
          <Button
            type="button"
            variant="outline"
            className="self-start"
            disabled={!bindingValues}
            onClick={() => {
              if (!bindingValues) return;
              for (const variable of defaults)
                if (!bindingValues.has(variable.key))
                  bindingValues.set(variable.key, variable.defaultValueJson!);
              setBindings(inputsJson(bindingValues));
            }}
          >
            Use defaults
          </Button>
        )}
        {!bindingValues ? (
          <p className="text-sm" role="status">
            Bindings JSON is invalid. Correct it in Advanced settings to use visual editing.
          </p>
        ) : (
          variables.bindings.map((variable) => (
            <BindingEditor
              key={variable.key}
              variable={variable}
              value={bindingValues.get(variable.key)}
              onChange={(value) => {
                if (value === undefined) bindingValues.delete(variable.key);
                else bindingValues.set(variable.key, value);
                setBindings(inputsJson(bindingValues));
              }}
            />
          ))
        )}
        {!variables.bindings.length && (
          <p className="text-sm text-muted-foreground">No template bindings.</p>
        )}
      </section>
      <section className="form-stack min-w-0" aria-label="Fleet input variables">
        <div>
          <h3 className="text-sm font-semibold">Fleet input variables</h3>
          <p className="text-sm text-muted-foreground">
            Review declared values, then choose which values fleets may use. Defaults do not approve
            values automatically.
          </p>
        </div>
        {!!declared.length && (
          <Button
            type="button"
            variant="outline"
            className="self-start"
            disabled={!policyValues}
            onClick={() => {
              if (!policyValues) return;
              for (const variable of declared)
                if (!policyValues.has(variable.key))
                  policyValues.set(
                    variable.key,
                    `[${variable.options.map((option) => option.valueJson).join(",")}]`,
                  );
              setPolicy(inputsJson(policyValues));
            }}
          >
            Use declared options
          </Button>
        )}
        {!policyValues && (
          <p className="text-sm" role="status">
            Fleet input policy JSON is invalid. Correct it in Advanced settings first.
          </p>
        )}
        {variables.parameters.map((variable) => (
          <div key={variable.key} className="form-stack gap-2 rounded-md border p-3 min-w-0">
            <h4 className="text-sm font-medium">{variable.label || variable.key}</h4>
            <VariableDescription variable={variable} />
            {!variable.sensitive && !!variable.options.length && (
              <div className="space-y-2">
                <p className="text-sm font-medium">Declared options</p>
                {variable.options.map((option, index) => (
                  <div key={index}>
                    <RawInputValuePreview raw={option.valueJson} />
                  </div>
                ))}
              </div>
            )}
            {policyValues?.has(variable.key) ? (
              <p className="text-xs text-muted-foreground">
                Input policy configured. Edit approved values in Advanced settings.
              </p>
            ) : (
              <p className="text-xs text-muted-foreground">No values approved yet.</p>
            )}
          </div>
        ))}
        {!variables.parameters.length && (
          <p className="text-sm text-muted-foreground">No fleet input variables.</p>
        )}
      </section>
    </div>
  );
}
