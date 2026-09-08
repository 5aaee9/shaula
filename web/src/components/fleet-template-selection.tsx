import type { FleetInputs } from "@/lib/use-fleet-inputs";
import type { TemplateSummary } from "@/lib/types";
import { ErrorNotice, Field } from "./status";
import { Button } from "./ui/button";
import { Input } from "./ui/input";

export function FleetTemplateSelection({
  editor,
  templates,
}: {
  editor: FleetInputs;
  templates: TemplateSummary[];
}) {
  return (
    <div className="form-stack">
      <Field label="Template profile">
        <Input
          required
          list="template-keys"
          value={editor.template}
          disabled={editor.locked || !editor.canRead}
          onChange={(event) => editor.changeTemplate(event.target.value)}
          onBlur={() => {
            if (
              editor.template.trim() &&
              !editor.locked &&
              !editor.pending &&
              editor.template.trim() !== editor.selection?.contract.profileKey
            )
              void editor.load();
          }}
          placeholder="kubernetes-linux"
        />
        <datalist id="template-keys">
          {templates
            .filter((template) => template.activeRevision)
            .map((template) => (
              <option key={template.key} value={template.key} />
            ))}
        </datalist>
      </Field>
      {editor.selection && (
        <p className="text-sm text-muted-foreground" role="status">
          {editor.selection.contract.profileKey} · Revision {editor.selection.contract.revision}
        </p>
      )}
      {!editor.locked && editor.canRead && (
        <Button
          type="button"
          variant="outline"
          disabled={editor.loading || !editor.template.trim()}
          onClick={() => void editor.load(false, true)}
        >
          {editor.selection ? "Load latest Active" : "Load template"}
        </Button>
      )}
      {editor.loading && (
        <p role="status" className="text-sm">
          Loading template inputs…
        </p>
      )}
      {editor.error !== null && (
        <div>
          <ErrorNotice error={editor.error} />
          {editor.locked && (
            <p className="text-sm">
              Template reference and inputs are locked and preserved. You can still save other fleet
              settings.
            </p>
          )}
          {editor.canRead && (
            <Button
              type="button"
              variant="outline"
              disabled={editor.loading}
              onClick={() => void editor.retry()}
            >
              Retry input contract
            </Button>
          )}
        </div>
      )}
      {editor.pending && (
        <div className="form-stack rounded-md border p-3" role="alert">
          <p className="text-sm">
            Switch to {editor.pending.contract.profileKey}, Revision{" "}
            {editor.pending.contract.revision}? Your current template inputs will be discarded.
          </p>
          <Button type="button" variant="outline" onClick={editor.confirmSwitch}>
            Discard inputs and switch
          </Button>
        </div>
      )}
      {editor.needsCancel && (
        <Button type="button" variant="ghost" onClick={editor.cancelSwitch}>
          Cancel switch
        </Button>
      )}
    </div>
  );
}
