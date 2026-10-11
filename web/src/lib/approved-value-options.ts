import type { ApprovedInputOption } from "./input-contract";
import { optionLabel, parseInputValue, sameInputValue, type InputValue } from "./input-values";

export interface ApprovedValueSelection {
  /** Select value: "" when unset, "unavailable" for a stale value, otherwise the option index. */
  selected: string;
  unavailable: boolean;
  /** Options mix value kinds, so titles must spell out the kind. */
  mixed: boolean;
  showPreview: boolean;
}

export function approvedValueSelection(
  options: ApprovedInputOption[],
  value: string | undefined,
): ApprovedValueSelection {
  const mixed = new Set(options.map((option) => parseInputValue(option.valueJson).kind)).size > 1;
  if (value === undefined) return { selected: "", unavailable: false, mixed, showPreview: false };
  const index = options.findIndex((option) => sameInputValue(option.valueJson, value));
  if (index < 0) return { selected: "unavailable", unavailable: true, mixed, showPreview: true };
  const option = parseInputValue(options[index].valueJson);
  // Values a short single-line option title cannot represent faithfully get a full preview.
  const showPreview =
    mixed ||
    !!option.children ||
    option.kind === "null" ||
    option.text === "" ||
    option.text.length > 120;
  return { selected: String(index), unavailable: false, mixed, showPreview };
}

function compoundSummary(value: InputValue): string {
  if (!value.children?.length) return value.kind === "array" ? "Empty array" : "Empty object";
  return (
    value.children
      ?.slice(0, 3)
      .map(
        ([key, child]) =>
          `${value.kind === "object" ? `${key}: ` : ""}${child.children ? child.text : child.text || '""'}`,
      )
      .join(", ")
      .slice(0, 100) ?? value.text
  );
}

function scalarTitle(value: InputValue, mixed: boolean): string {
  if (mixed || value.kind === "null" || value.text === "") return optionLabel(value);
  if (value.kind === "boolean") return value.text === "true" ? "Yes" : "No";
  return value.text;
}

export function approvedOptionTitle(raw: string, index: number, mixed: boolean): string {
  const value = parseInputValue(raw);
  if (value.children) return `Configuration ${index + 1}: ${compoundSummary(value)}`;
  const title = scalarTitle(value, mixed);
  return title.length > 120 ? `${title.slice(0, 117)}…` : title;
}
