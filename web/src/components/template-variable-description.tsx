import type { TemplateVariable } from "@/lib/template-variables";
import { RawInputValuePreview } from "./template-input-values";

export function TemplateVariableDescription({ variable }: { variable: TemplateVariable }) {
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
