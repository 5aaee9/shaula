import type { FleetInputs } from "@/lib/use-fleet-inputs";
import type { ProfileChoices } from "@/lib/profile-choice";
import { ErrorNotice } from "./status";
import { Button } from "./ui/button";
import { ProfileSelector } from "./profile-selector";

export function FleetTemplateSelection({
  editor,
  templates,
}: {
  editor: FleetInputs;
  templates: ProfileChoices;
}) {
  return (
    <div className="form-stack">
      <ProfileSelector
        label="Template profile"
        value={editor.template}
        query={templates}
        canRead={editor.canRead}
        permission="template.read"
        disabled={editor.locked}
        onChange={editor.changeTemplate}
      />
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
          onClick={() => void editor.load({ latest: true })}
        >
          Load latest Active
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
      {editor.canRestoreOriginal && (
        <Button type="button" variant="ghost" onClick={editor.restoreOriginal}>
          Restore original template and inputs
        </Button>
      )}
    </div>
  );
}
