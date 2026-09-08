import { useId } from "react";
import type { ApprovedInputOption } from "@/lib/input-contract";
import { optionLabel, parseInputValue, sameInputValue, type InputValue } from "@/lib/input-values";
import { Button } from "./ui/button";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

export function InputValuePreview({ value }: { value: InputValue }) {
  if (value.kind === "object")
    return (
      <dl className="space-y-2 border-l pl-3 text-sm">
        {!value.children?.length && <div>Empty object</div>}
        {value.children?.map(([key, child]) => (
          <div key={key} className="min-w-0">
            <dt className="font-medium break-all">{key}</dt>
            <dd>
              <InputValuePreview value={child} />
            </dd>
          </div>
        ))}
      </dl>
    );
  if (value.kind === "array")
    return (
      <ol className="list-inside list-decimal space-y-2 border-l pl-3 text-sm">
        {!value.children?.length && <li>Empty array</li>}
        {value.children?.map(([key, child]) => (
          <li key={key}>
            <InputValuePreview value={child} />
          </li>
        ))}
      </ol>
    );
  return <span className="whitespace-pre-wrap break-all text-sm">{optionLabel(value)}</span>;
}

export function RawInputValuePreview({ raw }: { raw: string }) {
  try {
    return <InputValuePreview value={parseInputValue(raw)} />;
  } catch {
    return (
      <div className="space-y-2 min-w-0">
        <p className="text-sm">
          This existing value is too deeply nested for a structured preview. Its complete value is
          preserved below.
        </p>
        <pre
          className="max-h-64 overflow-auto whitespace-pre-wrap break-all text-xs"
          aria-label="Preserved input value"
        >
          {raw}
        </pre>
      </div>
    );
  }
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

export function ApprovedValueSelect({
  label,
  inputKey,
  description,
  required,
  options,
  value,
  onChange,
}: {
  label: string;
  inputKey?: string;
  description?: string;
  required: boolean;
  options: ApprovedInputOption[];
  value?: string;
  onChange: (value?: string) => void;
}) {
  const id = useId();
  const index =
    value === undefined
      ? -1
      : options.findIndex((option) => sameInputValue(option.valueJson, value));
  const unavailable = value !== undefined && index < 0;
  const selected = value === undefined ? "" : unavailable ? "unavailable" : String(index);
  const mixed = new Set(options.map((option) => parseInputValue(option.valueJson).kind)).size > 1;
  const selectedOption = index < 0 ? undefined : parseInputValue(options[index].valueJson);
  const showPreview =
    unavailable ||
    mixed ||
    selectedOption?.children ||
    selectedOption?.kind === "null" ||
    selectedOption?.text === "" ||
    (selectedOption?.text.length ?? 0) > 120;
  return (
    <div className="form-stack gap-2 min-w-0 [&_[data-slot=native-select-wrapper]]:w-full [&_[data-slot=native-select-wrapper]]:max-w-full">
      <label htmlFor={id} className="text-sm font-medium">
        {label}
        {required && <span aria-hidden="true"> *</span>}
      </label>
      {inputKey && <p className="text-xs text-muted-foreground break-all">{inputKey}</p>}
      <NativeSelect
        id={id}
        className="w-full max-w-full"
        aria-describedby={`${id}-details`}
        required={required}
        value={selected}
        onChange={(event) =>
          onChange(
            event.target.value === "" ? undefined : options[Number(event.target.value)].valueJson,
          )
        }
      >
        <NativeSelectOption value="">{required ? "Choose a value" : "Not set"}</NativeSelectOption>
        {unavailable && (
          <NativeSelectOption value="unavailable" disabled>
            Existing value is no longer selectable
          </NativeSelectOption>
        )}
        {options.map((option, optionIndex) => {
          const parsed = parseInputValue(option.valueJson);
          const title =
            mixed || parsed.kind === "null" || parsed.children || parsed.text === ""
              ? optionLabel(parsed)
              : parsed.kind === "boolean"
                ? parsed.text === "true"
                  ? "Yes"
                  : "No"
                : parsed.text;
          return (
            <NativeSelectOption key={optionIndex} value={optionIndex}>
              {parsed.children
                ? `Configuration ${optionIndex + 1}: ${compoundSummary(parsed)}`
                : title.length > 120
                  ? `${title.slice(0, 117)}…`
                  : title}
            </NativeSelectOption>
          );
        })}
      </NativeSelect>
      <div id={`${id}-details`} className="min-w-0 space-y-2">
        {description && (
          <p className="whitespace-pre-wrap break-words text-sm text-muted-foreground">
            {description}
          </p>
        )}
        {unavailable && (
          <p className="text-sm" role="status">
            Existing value is no longer selectable. It is preserved until you replace or remove it.
          </p>
        )}
        {value !== undefined && showPreview && <RawInputValuePreview raw={value} />}
        {unavailable && (
          <Button type="button" variant="outline" size="sm" onClick={() => onChange(undefined)}>
            Remove {inputKey ?? label}
          </Button>
        )}
      </div>
    </div>
  );
}
