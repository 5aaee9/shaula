import { useId, useState, type KeyboardEvent } from "react";
import { parseInputValue, type InputValue } from "@/lib/input-values";
import type { TemplateVariable } from "@/lib/template-variables";
import { Button } from "./ui/button";
import { Input } from "./ui/input";

function scalarDefault(variable: TemplateVariable): InputValue | undefined {
  if (variable.sensitive || variable.defaultValueJson === undefined) return undefined;
  try {
    const value = parseInputValue(variable.defaultValueJson);
    if (variable.typeName === "integer")
      return value.kind === "number" && /^-?(?:0|[1-9]\d*)$/.test(value.raw) ? value : undefined;
    const expected = variable.typeName === "bool" ? "boolean" : variable.typeName;
    return ["string", "number", "boolean"].includes(expected) && value.kind === expected
      ? value
      : undefined;
  } catch {
    return undefined;
  }
}

export function TemplatePolicyValueInput({
  variable,
  disabled = false,
  onAdd,
}: {
  variable: TemplateVariable;
  disabled?: boolean;
  onAdd: (raw: string) => void;
}) {
  const id = useId();
  const [text, setText] = useState("");
  const label = `Add approved value for ${variable.label || variable.key}`;
  const string = variable.typeName === "string";
  const boolean = variable.typeName === "boolean" || variable.typeName === "bool";
  const number = variable.typeName === "number" || variable.typeName === "integer";
  const defaultValue = scalarDefault(variable);
  const validNumber =
    variable.typeName === "integer"
      ? /^-?(?:0|[1-9]\d*)$/.test(text)
      : /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(text);
  const canAdd = string || (boolean ? text === "true" || text === "false" : number && validNumber);
  const invalid = number && text !== "" && !validNumber;

  function add() {
    if (disabled || !canAdd) return;
    onAdd(string ? JSON.stringify(text) : text);
    setText("");
  }

  function handleInputKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter" && !event.nativeEvent.isComposing) {
      event.preventDefault();
      add();
    }
  }

  if (!string && !boolean && !number)
    return (
      <p className="text-sm text-muted-foreground">
        Configure approved values in Fleet input policy (JSON) under Advanced settings.
      </p>
    );

  return (
    <div className="form-stack min-w-0 gap-2">
      {boolean ? (
        <fieldset disabled={disabled} className="min-w-0 space-y-2">
          <legend className="text-sm font-medium">{label}</legend>
          <div className="flex flex-wrap gap-4">
            {[
              ["true", "Yes"],
              ["false", "No"],
            ].map(([value, title]) => (
              <label key={value} className="flex items-center gap-2 text-sm">
                <input
                  type="radio"
                  name={id}
                  value={value}
                  checked={text === value}
                  onChange={() => setText(value)}
                  onKeyDown={handleInputKeyDown}
                  className="size-4 shrink-0 accent-primary"
                />
                {title}
              </label>
            ))}
          </div>
        </fieldset>
      ) : (
        <>
          <label htmlFor={id} className="text-sm font-medium">
            {label}
          </label>
          <Input
            id={id}
            type="text"
            inputMode={number ? "decimal" : "text"}
            value={text}
            placeholder={defaultValue?.text}
            disabled={disabled}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={invalid || undefined}
            aria-describedby={invalid ? `${id}-error` : undefined}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={handleInputKeyDown}
          />
          {invalid && (
            <p id={`${id}-error`} className="text-sm text-destructive" role="status">
              Enter {variable.typeName === "integer" ? "an integer" : "a number"}.
            </p>
          )}
          {string && text === "" && (
            <p className="text-xs text-muted-foreground">
              Adding a blank value approves an empty string.
            </p>
          )}
        </>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled || !canAdd}
          onClick={add}
        >
          Add value
        </Button>
        {defaultValue && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => {
              onAdd(defaultValue.raw);
              setText("");
            }}
          >
            Use default value
          </Button>
        )}
      </div>
    </div>
  );
}
