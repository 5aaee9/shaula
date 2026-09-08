import { useQuery } from "@tanstack/react-query";
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
    <section className="mb-8 space-y-4" aria-label="Default templates">
      <div>
        <h2 className="text-base font-semibold">Default templates</h2>
        <p className="text-sm text-muted-foreground">
          Start from a template in the library and configure its variables before publishing.
        </p>
      </div>
      {query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : query.isPending ? (
        <Loading />
      ) : !query.data.data.sources.length ? (
        <p className="text-sm text-muted-foreground">No default templates are available.</p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {query.data.data.sources.map((source) => (
            <div
              key={source.key}
              className="min-w-0 rounded-lg border p-4 flex items-center justify-between gap-3"
            >
              <div className="min-w-0">
                <h3 className="font-medium break-all">{source.key}</h3>
                <p className="text-xs text-muted-foreground break-all">
                  {source.platform} · {source.engineRef}
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                disabled={!canPublish}
                aria-label={`Use template ${source.key}`}
                onClick={() => onUse(source)}
              >
                Use template
              </Button>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
