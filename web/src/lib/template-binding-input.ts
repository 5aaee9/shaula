import { parseInputValue, type InputValue } from "./input-values";
import type { TemplateVariable } from "./template-variables";

const SCALAR_TYPES = ["string", "number", "integer", "boolean", "bool"];

export interface BindingInput {
  label: string;
  /** JSON kind a visual editor produces for this variable. */
  expected: string;
  parsed?: InputValue;
  /** Visual editing is unsafe; the value must be edited as manual JSON. */
  manual: boolean;
}

export function bindingInput(variable: TemplateVariable, value?: string): BindingInput {
  let parsed: InputValue | undefined;
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
  return {
    label: variable.label || variable.key,
    expected,
    parsed,
    manual: !SCALAR_TYPES.includes(variable.typeName) || incompatible,
  };
}

/**
 * Binding JSON for typed text, or `null` when the text is not yet a valid
 * number and the stored value should stay unchanged.
 */
export function typedBindingValue(expected: string, text: string): string | undefined | null {
  if (expected === "string") return JSON.stringify(text);
  if (!text) return undefined;
  if (/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(text)) return text;
  return null;
}

/** Terraform default offered as a one-click binding; secrets and null never are. */
export function bindingDefault(variable: TemplateVariable): InputValue | undefined {
  return !variable.sensitive &&
    variable.defaultValueJson !== undefined &&
    variable.defaultValueJson !== "null"
    ? parseInputValue(variable.defaultValueJson)
    : undefined;
}
