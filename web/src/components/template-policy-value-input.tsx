import { useId, useState, type KeyboardEvent } from "react";
import type { TemplateVariable } from "@/lib/template-variables";
import {
  policyDraftValidity,
  policyValueKind,
  scalarDefault,
  type PolicyValueKind,
} from "@/lib/template-policy-value-draft";
import { Button } from "./ui/button";
import { Input } from "./ui/input";

function BooleanPolicyChoice({
  id,
  label,
  disabled,
  text,
  onSelect,
  onKeyDown,
}: {
  id: string;
  label: string;
  disabled: boolean;
  text: string;
  onSelect: (value: string) => void;
  onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void;
}) {
  return (
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
              onChange={() => onSelect(value)}
              onKeyDown={onKeyDown}
              className="size-4 shrink-0 accent-primary"
            />
            {title}
          </label>
        ))}
      </div>
    </fieldset>
  );
}

function TextPolicyValueField({
  id,
  label,
  kind,
  integer,
  text,
  placeholder,
  disabled,
  invalid,
  onTextChange,
  onKeyDown,
}: {
  id: string;
  label: string;
  kind: PolicyValueKind;
  integer: boolean;
  text: string;
  placeholder?: string;
  disabled: boolean;
  invalid: boolean;
  onTextChange: (text: string) => void;
  onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void;
}) {
  return (
    <>
      <label htmlFor={id} className="text-sm font-medium">
        {label}
      </label>
      <Input
        id={id}
        type="text"
        inputMode={kind === "number" ? "decimal" : "text"}
        value={text}
        placeholder={placeholder}
        disabled={disabled}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={invalid || undefined}
        aria-describedby={invalid ? `${id}-error` : undefined}
        onChange={(event) => onTextChange(event.target.value)}
        onKeyDown={onKeyDown}
      />
      {invalid && (
        <p id={`${id}-error`} className="text-sm text-destructive" role="status">
          Enter {integer ? "an integer" : "a number"}.
        </p>
      )}
      {kind === "string" && text === "" && (
        <p className="text-xs text-muted-foreground">
          Adding a blank value approves an empty string.
        </p>
      )}
    </>
  );
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
  const kind = policyValueKind(variable.typeName);
  const defaultValue = scalarDefault(variable);

  if (!kind)
    return (
      <p className="text-sm text-muted-foreground">
        Configure approved values in Fleet input policy (JSON) under Advanced settings.
      </p>
    );

  const { canAdd, invalid } = policyDraftValidity(kind, variable.typeName, text);

  function add() {
    if (disabled || !canAdd) return;
    onAdd(kind === "string" ? JSON.stringify(text) : text);
    setText("");
  }

  function handleInputKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter" && !event.nativeEvent.isComposing) {
      event.preventDefault();
      add();
    }
  }

  return (
    <div className="form-stack min-w-0 gap-2">
      {kind === "boolean" ? (
        <BooleanPolicyChoice
          id={id}
          label={label}
          disabled={disabled}
          text={text}
          onSelect={setText}
          onKeyDown={handleInputKeyDown}
        />
      ) : (
        <TextPolicyValueField
          id={id}
          label={label}
          kind={kind}
          integer={variable.typeName === "integer"}
          text={text}
          placeholder={defaultValue?.text}
          disabled={disabled}
          invalid={invalid}
          onTextChange={setText}
          onKeyDown={handleInputKeyDown}
        />
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
