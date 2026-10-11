import { parseInputValue, type InputValue } from "./input-values";
import type { TemplateVariable } from "./template-variables";

const INTEGER = /^-?(?:0|[1-9]\d*)$/;
const NUMBER = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/;

/** Scalar kinds that can be added visually; anything else needs the JSON editor. */
export type PolicyValueKind = "string" | "boolean" | "number";

export function policyValueKind(typeName: string): PolicyValueKind | undefined {
  if (typeName === "string") return "string";
  if (typeName === "boolean" || typeName === "bool") return "boolean";
  if (typeName === "number" || typeName === "integer") return "number";
}

export function scalarDefault(variable: TemplateVariable): InputValue | undefined {
  if (variable.sensitive || variable.defaultValueJson === undefined) return undefined;
  try {
    const value = parseInputValue(variable.defaultValueJson);
    if (variable.typeName === "integer")
      return value.kind === "number" && INTEGER.test(value.raw) ? value : undefined;
    const expected = variable.typeName === "bool" ? "boolean" : variable.typeName;
    return ["string", "number", "boolean"].includes(expected) && value.kind === expected
      ? value
      : undefined;
  } catch {
    return undefined;
  }
}

export function policyDraftValidity(kind: PolicyValueKind, typeName: string, text: string) {
  if (kind === "string") return { canAdd: true, invalid: false };
  if (kind === "boolean") return { canAdd: text === "true" || text === "false", invalid: false };
  const valid = (typeName === "integer" ? INTEGER : NUMBER).test(text);
  return { canAdd: valid, invalid: text !== "" && !valid };
}
