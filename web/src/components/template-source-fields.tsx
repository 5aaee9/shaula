import { useId } from "react";
import { Check, Fingerprint, Library, Upload } from "lucide-react";
import type { TemplateSource } from "@/lib/template-variables";
import { useTemplateSources } from "./template-library";
import { TemplateArchiveUpload } from "./template-archive-upload";
import { ErrorNotice, Field } from "./status";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

export type TemplateSourceKind = "default" | "archive" | "existing";

const choices = [
  {
    value: "default",
    label: "Default template",
    description: "Start from the library",
    icon: Library,
  },
  {
    value: "archive",
    label: "Upload archive",
    description: "Bring your own template",
    icon: Upload,
  },
  {
    value: "existing",
    label: "Existing artifact",
    description: "Reuse an uploaded artifact",
    icon: Fingerprint,
  },
] as const;

export function TemplateSourceFields({
  source,
  setSource,
  selectedSource,
  onSelect,
  digest,
  setDigest,
  file,
  setFile,
  canRead,
  busy,
}: {
  source: TemplateSourceKind;
  setSource: (source: TemplateSourceKind) => void;
  selectedSource?: TemplateSource;
  onSelect: (source?: TemplateSource) => void;
  digest: string;
  setDigest: (digest: string) => void;
  file: File | null;
  setFile: (file: File | null) => void;
  canRead: boolean;
  busy: boolean;
}) {
  const id = useId();
  const sources = useTemplateSources(canRead);
  return (
    <div className="space-y-4">
      <fieldset>
        <legend className="mb-3 text-sm font-semibold">Template source</legend>
        <div className="grid gap-2 sm:grid-cols-3">
          {choices.map(({ value, label, description, icon: Icon }) => (
            <label key={value} className="relative cursor-pointer">
              <input
                type="radio"
                name={`${id}-source`}
                value={value}
                aria-label={label}
                checked={source === value}
                disabled={value === "default" && !canRead}
                onChange={() => setSource(value)}
                className="peer absolute inset-0 z-10 h-full w-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
              />
              <span className="flex h-full items-center gap-3 rounded-lg border p-3 transition-colors hover:bg-muted/40 peer-checked:border-primary peer-checked:bg-muted/60 peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-focus-visible:ring-offset-2 peer-disabled:cursor-not-allowed peer-disabled:opacity-40 sm:flex-col sm:items-start sm:gap-3">
                <Icon className="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
                <span className="min-w-0 space-y-1 pr-4 sm:pr-0">
                  <span className="block text-sm font-medium">{label}</span>
                  <span className="block text-xs text-muted-foreground">{description}</span>
                </span>
                {source === value && (
                  <Check className="absolute top-3 right-3 size-3.5" aria-hidden="true" />
                )}
              </span>
            </label>
          ))}
        </div>
      </fieldset>
      {source === "default" && (
        <div className="space-y-3 rounded-lg border bg-muted/20 p-4">
          <Field label="Default template">
            <NativeSelect
              required
              value={selectedSource?.key || ""}
              onChange={(event) =>
                onSelect(
                  sources.data?.data.sources.find((entry) => entry.key === event.target.value),
                )
              }
            >
              <NativeSelectOption value="">
                {sources.isPending ? "Loading templates..." : "Choose a template"}
              </NativeSelectOption>
              {sources.data?.data.sources.map((entry) => (
                <NativeSelectOption key={entry.key} value={entry.key}>
                  {entry.key}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
          {sources.error && (
            <ErrorNotice error={sources.error} retry={() => void sources.refetch()} />
          )}
          {sources.data && !sources.data.data.sources.length && (
            <p className="text-sm text-muted-foreground">
              No default templates are available. Upload an archive or use an existing artifact.
            </p>
          )}
          {selectedSource && (
            <div className="space-y-2 text-xs text-muted-foreground">
              <p>
                Platform{" "}
                <span className="font-medium text-foreground">{selectedSource.platform}</span> ·
                Engine{" "}
                <span className="font-medium text-foreground">{selectedSource.engineRef}</span>
              </p>
              <details>
                <summary className="cursor-pointer">Artifact digest</summary>
                <p className="mt-2 break-all font-mono">{selectedSource.artifactDigest}</p>
              </details>
            </div>
          )}
        </div>
      )}
      <TemplateArchiveUpload
        file={file}
        onChange={setFile}
        active={source === "archive"}
        disabled={busy}
      />
      <div hidden={source !== "existing"}>
        <Field label="Existing artifact digest">
          <Input
            required={source === "existing"}
            disabled={source !== "existing"}
            pattern="sha256:[a-f0-9]{64}"
            placeholder="sha256:..."
            className="font-mono text-xs"
            aria-describedby={`${id}-digest-hint`}
            autoComplete="off"
            spellCheck={false}
            value={digest}
            onChange={(event) => setDigest(event.target.value)}
          />
          <p id={`${id}-digest-hint`} className="text-xs text-muted-foreground">
            Paste the SHA-256 digest of an archive already uploaded to Shaula.
          </p>
        </Field>
      </div>
    </div>
  );
}
