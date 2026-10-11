import { inputEntries, inputsJson } from "@/lib/input-values";
import { bindingDefault, bindingInput, type BindingInput } from "@/lib/template-binding-input";
import type { TemplateVariable, TemplateVariables as Variables } from "@/lib/template-variables";
import { ApprovedValueSelect } from "./template-input-values";
import {
  BooleanBindingSelect,
  ManualBindingNotice,
  TypedBindingInput,
} from "./template-binding-fields";
import { TemplatePolicyOptions } from "./template-policy-options";
import { TemplateVariableDescription as VariableDescription } from "./template-variable-description";
import { Button } from "./ui/button";

function entries(raw: string): Map<string, string> | null {
  try {
    return inputEntries(raw);
  } catch {
    return null;
  }
}

function BindingControl({
  variable,
  input,
  placeholder,
  value,
  onChange,
}: {
  variable: TemplateVariable;
  input: BindingInput;
  placeholder?: string;
  value?: string;
  onChange: (value?: string) => void;
}) {
  const options = variable.sensitive ? [] : variable.options;
  if (options.length)
    return (
      <ApprovedValueSelect
        label={input.label}
        required={variable.required}
        options={options}
        value={value}
        onChange={onChange}
      />
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
  return (
    <TypedBindingInput
      variable={variable}
      input={input}
      placeholder={placeholder}
      onChange={onChange}
    />
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
  const input = bindingInput(variable, value);
  const { label } = input;
  if (input.manual)
    return (
      <ManualBindingNotice
        variable={variable}
        label={label}
        raw={variable.sensitive ? undefined : value}
      >
        Configure this value in Bindings (JSON) under Advanced settings.
      </ManualBindingNotice>
    );
  const defaultValue = bindingDefault(variable);
  return (
    <div className="form-stack gap-2">
      <BindingControl
        variable={variable}
        input={input}
        placeholder={defaultValue?.text}
        value={value}
        onChange={onChange}
      />
      <VariableDescription variable={variable} />
      {defaultValue && (
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="self-start"
          onClick={() => onChange(defaultValue.raw)}
        >
          Use default for {label}
        </Button>
      )}
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
          <TemplatePolicyOptions
            key={variable.key}
            variable={variable}
            value={policyValues?.get(variable.key)}
            disabled={!policyValues}
            onChange={(value) => {
              if (!policyValues) return;
              policyValues.set(variable.key, value);
              setPolicy(inputsJson(policyValues));
            }}
          />
        ))}
        {!variables.parameters.length && (
          <p className="text-sm text-muted-foreground">No fleet input variables.</p>
        )}
      </section>
    </div>
  );
}
