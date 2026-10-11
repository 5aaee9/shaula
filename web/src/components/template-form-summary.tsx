import { ArrowRight, CheckCheck, Layers3, LoaderCircle, Upload } from "lucide-react";
import type { TemplateSource } from "@/lib/template-variables";
import { ErrorNotice } from "./status";
import type { TemplateSourceKind } from "./template-source-fields";
import { Button } from "./ui/button";

export function PublicationSummary({
  profileKey,
  source,
  selectedSource,
  file,
  engineRef,
}: {
  profileKey: string;
  source: TemplateSourceKind;
  selectedSource?: TemplateSource;
  file: File | null;
  engineRef: string;
}) {
  return (
    <aside
      className="min-w-0 rounded-xl border bg-muted/25 p-5 xl:sticky xl:top-6"
      aria-label="Publication summary"
    >
      <div className="mb-5 flex items-center gap-2">
        <Layers3 className="size-4" aria-hidden="true" />
        <h2 className="text-sm">Publication summary</h2>
      </div>
      <dl className="space-y-4 text-sm">
        <div>
          <dt className="text-xs text-muted-foreground">Profile key</dt>
          <dd className="mt-1 break-all font-medium">{profileKey || "Not named yet"}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Source</dt>
          <dd className="mt-1 break-all font-medium">
            {source === "default"
              ? selectedSource?.key || "Choose a default template"
              : source === "archive"
                ? file?.name || "Upload archive"
                : "Existing artifact"}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Engine</dt>
          <dd className="mt-1 break-all font-medium">{engineRef || "Not set"}</dd>
        </div>
      </dl>
      <div className="mt-5 space-y-3 border-t pt-5 text-sm">
        <div className="flex items-center gap-2 font-medium">
          <CheckCheck className="size-4 text-muted-foreground" aria-hidden="true" />
          What happens next
        </div>
        <p className="text-xs leading-relaxed text-muted-foreground">
          Shaula validates your template after publishing. Once validation passes, the new revision
          activates automatically.
        </p>
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          Publish <ArrowRight className="size-3" aria-hidden="true" /> Validate{" "}
          <ArrowRight className="size-3" aria-hidden="true" /> Activate
        </p>
      </div>
    </aside>
  );
}

export function PublishBar({
  busy,
  inspecting,
  error,
  onClose,
}: {
  busy: boolean;
  /** Publishing waits for an in-flight variable inspection. */
  inspecting: boolean;
  error: unknown;
  onClose: () => void;
}) {
  return (
    <div className="sticky bottom-0 z-10 mt-6 border-t bg-background/95 py-4 backdrop-blur-sm">
      {error !== null && <ErrorNotice error={error} />}
      <div className="flex items-center justify-between gap-3">
        <p className="hidden text-xs text-muted-foreground sm:block">
          {busy ? "Publishing your template…" : "Changes are saved when you publish."}
        </p>
        <div className="flex w-full justify-end gap-2 sm:w-auto">
          <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy || inspecting}>
            {busy ? <LoaderCircle className="animate-spin" /> : <Upload />}{" "}
            {busy ? "Publishing…" : "Publish"}
          </Button>
        </div>
      </div>
    </div>
  );
}
