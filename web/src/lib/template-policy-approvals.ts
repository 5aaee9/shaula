import { parseInputValue, sameInputValue } from "./input-values";
import type { TemplateVariable } from "./template-variables";

/** Approved values from a policy entry, or null when it is not a JSON array. */
export function approvedValues(raw: string | undefined): string[] | null {
  if (raw === undefined) return [];
  try {
    const value = parseInputValue(raw);
    return value.kind === "array" ? (value.children ?? []).map(([, child]) => child.raw) : null;
  } catch {
    return null;
  }
}

/** Declared options followed by any approved values not already declared, deduplicated. */
export function policyOptionValues(
  variable: TemplateVariable,
  selected: string[] | null,
): string[] {
  const options: string[] = [];
  if (variable.sensitive) return options;
  for (const raw of [...variable.options.map((option) => option.valueJson), ...(selected ?? [])])
    if (!options.some((option) => sameInputValue(option, raw))) options.push(raw);
  return options;
}

export function policyApprovalHint(selected: string[] | null, sensitive: boolean): string {
  if (selected === null)
    return "Approved values must be a JSON array. Correct this variable in Advanced settings to use visual editing.";
  if (sensitive) return "Sensitive values are not displayed.";
  if (!selected.length) return "No values approved yet.";
  return `${selected.length} ${selected.length === 1 ? "value" : "values"} approved. Uncheck to remove an approval.`;
}
