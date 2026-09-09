import { inputsJson } from "@/lib/input-values";
import type { TemplateVariables } from "@/lib/template-variables";
import { RawInputValuePreview } from "./template-input-values";
import { Button } from "./ui/button";

export function TemplateUpdatePolicy({
  variables,
  policy,
  onChange,
}: {
  variables: TemplateVariables | null;
  policy: string | undefined;
  onChange: (policy: string | undefined) => void;
}) {
  const declared = variables?.available
    ? variables.parameters.filter((variable) => !variable.sensitive && variable.options.length)
    : [];
  return (
    <section
      className="space-y-4 rounded-xl border bg-card p-5 sm:p-6"
      aria-label="Fleet input policy"
    >
      <div>
        <h2>Fleet input policy</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {policy === undefined
            ? "Retain the policy from the current revision. Reading variables does not change approved values."
            : "Replace the entire policy with the declared options shown below. Other approved values will be removed."}
        </p>
      </div>
      {!!declared.length && policy === undefined && (
        <>
          <div className="space-y-3">
            {declared.map((variable) => (
              <div key={variable.key} className="min-w-0 space-y-2">
                <h3 className="text-sm font-medium">{variable.label || variable.key}</h3>
                {variable.options.map((option, index) => (
                  <RawInputValuePreview key={index} raw={option.valueJson} />
                ))}
              </div>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">
            Using these options replaces the entire policy, including approvals for other variables.
          </p>
          <Button
            type="button"
            variant="outline"
            onClick={() =>
              onChange(
                inputsJson(
                  new Map(
                    declared.map((variable) => [
                      variable.key,
                      `[${variable.options.map((option) => option.valueJson).join(",")}]`,
                    ]),
                  ),
                ),
              )
            }
          >
            Use declared options
          </Button>
        </>
      )}
      {policy !== undefined && (
        <>
          <div aria-label="Replacement policy">
            <RawInputValuePreview raw={policy} />
          </div>
          <Button type="button" variant="outline" onClick={() => onChange(undefined)}>
            Retain current policy
          </Button>
        </>
      )}
    </section>
  );
}
