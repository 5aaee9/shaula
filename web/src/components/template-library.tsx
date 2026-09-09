import { useQuery } from "@tanstack/react-query";
import { ArrowRight, Box, Library } from "lucide-react";
import { api } from "@/lib/api";
import type { TemplateSource } from "@/lib/template-variables";
import { Button } from "./ui/button";
import { ErrorNotice, Loading } from "./status";

export function useTemplateSources(enabled: boolean) {
  return useQuery({
    queryKey: ["template-sources"],
    enabled,
    queryFn: ({ signal }) => api<{ sources: TemplateSource[] }>("/template-sources", { signal }),
  });
}

export function TemplateLibrary({
  canPublish,
  onUse,
}: {
  canPublish: boolean;
  onUse: (source: TemplateSource) => void;
}) {
  const query = useTemplateSources(true);
  return (
    <section
      className="mb-8 space-y-5 rounded-xl border bg-muted/30 p-4 sm:p-5"
      aria-label="Default templates"
    >
      <div className="flex items-start gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-lg border bg-background">
          <Library className="size-4 text-muted-foreground" aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <h2 className="text-base font-semibold">Default templates</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Choose a starting point, configure its variables, and publish your own template.
          </p>
        </div>
      </div>
      {query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : query.isPending ? (
        <Loading />
      ) : !query.data.data.sources.length ? (
        <div className="rounded-lg border border-dashed bg-background px-4 py-5">
          <p className="font-medium">No default templates are available.</p>
          <p className="mt-1 text-sm text-muted-foreground">
            You can still publish a template from an archive or an existing artifact.
          </p>
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {query.data.data.sources.map((source) => (
            <div key={source.key} className="flex min-w-0 flex-col rounded-lg border bg-background">
              <div className="flex-1 space-y-4 p-4">
                <div className="flex items-center gap-2.5">
                  <Box className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <h3 className="min-w-0 break-all font-semibold">{source.key}</h3>
                </div>
                <dl className="grid min-w-0 grid-cols-2 gap-3 text-xs">
                  <div className="min-w-0">
                    <dt className="mb-1.5 text-muted-foreground">Platform</dt>
                    <dd className="break-all font-medium">{source.platform}</dd>
                  </div>
                  <div className="min-w-0">
                    <dt className="mb-1.5 text-muted-foreground">Engine</dt>
                    <dd className="break-all font-medium">{source.engineRef}</dd>
                  </div>
                </dl>
              </div>
              <div className="flex items-center justify-end border-t px-4 py-3">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!canPublish}
                  aria-label={`Use template ${source.key}`}
                  onClick={() => onUse(source)}
                >
                  Use template
                  <ArrowRight aria-hidden="true" />
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
