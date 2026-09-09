import { useId, useRef, useState } from "react";
import { FileArchive, Upload, X } from "lucide-react";
import { Button } from "./ui/button";

export function TemplateArchiveUpload({
  file,
  onChange,
  disabled,
  active,
}: {
  file: File | null;
  onChange: (file: File | null) => void;
  disabled: boolean;
  active: boolean;
}) {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState("");

  function choose(files: FileList | null) {
    const next = files?.[0] || null;
    const message =
      files && files.length > 1
        ? "Choose one archive at a time."
        : next && !/\.(tar\.gz|tgz)$/i.test(next.name)
          ? "Choose a .tar.gz or .tgz archive."
          : next && next.size > 64 * 1024 * 1024
            ? "Archive exceeds 64 MiB. Choose a smaller file."
            : "";
    setError(message);
    onChange(message ? null : next);
    if (input.current && (message || !next)) input.current.value = "";
    else if (input.current && files) input.current.files = files;
  }

  return (
    <div className="space-y-3" hidden={!active}>
      <div
        role="group"
        aria-label="Archive upload"
        className={`relative rounded-xl border border-dashed transition-colors focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/30 ${dragging ? "border-primary bg-muted" : "border-input bg-muted/30 hover:bg-muted/60"}`}
        onDragOver={(event) => {
          event.preventDefault();
          if (!disabled) setDragging(true);
        }}
        onDragLeave={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false);
        }}
        onDrop={(event) => {
          event.preventDefault();
          setDragging(false);
          if (!disabled) choose(event.dataTransfer.files);
        }}
      >
        <input
          ref={input}
          id={id}
          type="file"
          aria-label="Template archive (.tar.gz)"
          aria-describedby={`${id}-hint${error ? ` ${id}-error` : ""}`}
          aria-invalid={!!error}
          required={active}
          disabled={disabled || !active}
          accept=".tar.gz,.tgz,application/gzip"
          className="absolute inset-0 h-full w-full cursor-pointer opacity-0 disabled:cursor-default"
          onChange={(event) => choose(event.target.files)}
        />
        <label
          htmlFor={id}
          className="pointer-events-none flex flex-col items-center gap-3 px-4 py-7 text-center"
        >
          <span className="grid size-10 place-items-center rounded-lg border bg-background shadow-xs">
            <Upload className="size-5 text-muted-foreground" aria-hidden="true" />
          </span>
          <span className="space-y-1">
            <span className="block text-sm font-medium">
              {file ? "Drop a file to replace your archive" : "Drop your template archive here"}
            </span>
            <span className="block text-xs text-muted-foreground">
              or{" "}
              <span className="font-medium text-foreground underline underline-offset-2">
                browse files
              </span>
            </span>
          </span>
          <span id={`${id}-hint`} className="text-xs text-muted-foreground">
            .tar.gz or .tgz · Up to 64 MiB
          </span>
        </label>
      </div>
      {error && (
        <p id={`${id}-error`} role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {file && (
        <div className="flex min-w-0 items-center gap-3 rounded-lg border px-3 py-2.5">
          <FileArchive className="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <div className="min-w-0 flex-1" role="status">
            <p className="truncate text-sm font-medium" title={file.name}>
              {file.name}
            </p>
            <p className="text-xs text-muted-foreground">
              {file.size < 1024 * 1024
                ? `${Math.max(1, Math.ceil(file.size / 1024))} KB`
                : `${(file.size / (1024 * 1024)).toFixed(1)} MiB`}{" "}
              · Selected archive
            </p>
          </div>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label="Remove archive"
            disabled={disabled}
            onClick={() => choose(null)}
          >
            <X aria-hidden="true" />
          </Button>
        </div>
      )}
    </div>
  );
}
