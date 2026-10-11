import { FileSearch, LoaderCircle } from "lucide-react";
import type { useTemplateVariables } from "@/lib/template-variables";
import { ErrorNotice } from "./status";
import { TemplateVariables } from "./template-variables";
import type { TemplateSourceKind } from "./template-source-fields";
import { Button } from "./ui/button";

type Discovery = ReturnType<typeof useTemplateVariables>;

/** Explains why no variables are shown yet; discovery errors render separately. */
function DiscoveryPlaceholder({
  canRead,
  discovery,
  ready,
  source,
}: {
  canRead: boolean;
  discovery: Discovery;
  ready: boolean;
  source: TemplateSourceKind;
}) {
  if (!canRead)
    return (
      <p className="rounded-lg bg-muted/50 p-4 text-sm text-muted-foreground">
        Template read permission is required to inspect variables. Manual publishing is available in
        Advanced settings.
      </p>
    );
  if (discovery.loading)
    return (
      <div
        role="status"
        className="flex items-center gap-3 rounded-lg bg-muted/40 p-5 text-sm text-muted-foreground"
      >
        <LoaderCircle className="size-4 shrink-0 animate-spin" />
        Reading template variables…
      </div>
    );
  if (discovery.variables || discovery.error !== null) return null;
  return (
    <div className="flex items-start gap-3 rounded-lg border border-dashed p-5">
      <FileSearch className="mt-0.5 size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <div className="space-y-1 text-sm">
        <p className="font-medium">
          {ready ? "Your template is ready to inspect" : "Choose a source to get started"}
        </p>
        <p className="text-muted-foreground">
          {source === "archive"
            ? "Inspect variables uploads your archive and loads its configurable fields."
            : source === "default"
              ? "Variables load automatically when you choose a default template."
              : "Enter an artifact digest, then inspect its configurable fields."}
        </p>
      </div>
    </div>
  );
}

export function TemplateVariablesSection({
  id,
  canRead,
  discovery,
  ready,
  source,
  bindings,
  policy,
  setBindings,
  setPolicy,
}: {
  id: string;
  canRead: boolean;
  discovery: Discovery;
  /** The chosen source identifies an artifact that can be inspected. */
  ready: boolean;
  source: TemplateSourceKind;
  bindings: string;
  policy: string;
  setBindings: (value: string) => void;
  setPolicy: (value: string) => void;
}) {
  return (
    <section
      className="space-y-5 rounded-xl border bg-card p-5 sm:p-6"
      aria-labelledby={`${id}-variables`}
      aria-busy={discovery.loading}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1 basis-56">
          <h2 id={`${id}-variables`}>Configure variables</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Set shared bindings and the values fleets are allowed to use.
          </p>
        </div>
        {canRead && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={discovery.loading || !ready}
            onClick={() => void discovery.inspect()}
          >
            {discovery.loading ? <LoaderCircle className="animate-spin" /> : <FileSearch />}
            {discovery.loading ? "Inspecting variables..." : "Inspect variables"}
          </Button>
        )}
      </div>
      <DiscoveryPlaceholder canRead={canRead} discovery={discovery} ready={ready} source={source} />
      {discovery.error !== null && (
        <>
          <ErrorNotice error={discovery.error} />
          <p className="text-sm text-muted-foreground">
            Try inspecting again, or configure bindings and input policy in Advanced settings.
          </p>
        </>
      )}
      {discovery.variables && (
        <TemplateVariables
          variables={discovery.variables}
          bindings={bindings}
          policy={policy}
          setBindings={setBindings}
          setPolicy={setPolicy}
        />
      )}
    </section>
  );
}
