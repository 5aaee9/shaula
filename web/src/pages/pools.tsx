import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, RefreshCw } from "lucide-react";
import { api, resourcePath } from "@/lib/api";
import { useTemplatePools } from "@/lib/queries";
import type { TemplatePoolResource } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Empty, ErrorNotice, Loading } from "@/components/status";
import { PoolForm } from "@/components/pool-form";

/**
 * Shared TemplatePool inventory (spec 0037): pools author the weighted
 * member mix once; fleets reference them by key.
 */
export function PoolsPage({ scopes }: { scopes: string[] }) {
  const client = useQueryClient();
  const pools = useTemplatePools(scopes.includes("template.read"));
  const [editing, setEditing] = useState<string | "new" | null>(null);
  const summaries = pools.data?.data.pools || [];
  const detail = useQuery({
    queryKey: ["template-pool", editing],
    enabled: !!editing && editing !== "new" && scopes.includes("template.read"),
    queryFn: ({ signal }) =>
      api<TemplatePoolResource>(resourcePath("template-pools", editing as string), {
        signal,
      }),
  });

  if (!scopes.includes("template.read"))
    return <ErrorNotice error={new Error("Template read permission is required.")} />;
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Template pools</h1>
          <p>Weighted template mixes shared across fleets.</p>
        </div>
        {scopes.includes("template.publish") ? (
          <Button onClick={() => setEditing("new")}>
            <Plus />
            Create pool
          </Button>
        ) : (
          <Button disabled>
            <Plus />
            Create pool
          </Button>
        )}
      </div>
      {pools.error ? (
        <ErrorNotice error={pools.error} retry={() => void pools.refetch()} />
      ) : pools.isPending ? (
        <Loading />
      ) : !summaries.length ? (
        <Empty title="No template pools yet">
          <p className="text-sm text-muted-foreground">
            A pool defines a weighted mix of template members. Fleets reference it by key and draw
            one member per new runner.
          </p>
        </Empty>
      ) : (
        <section className="resource-section" aria-label="Template pool inventory">
          <div className="section-heading">
            <h2>
              All pools <span className="count">{summaries.length}</span>
            </h2>
            <Button
              variant="ghost"
              size="sm"
              disabled={pools.isFetching}
              onClick={() => void pools.refetch()}
            >
              <RefreshCw className={pools.isFetching ? "animate-spin" : undefined} />
              Refresh
            </Button>
          </div>
          <ul className="divide-y">
            {summaries.map((pool) => (
              <li key={pool.key} className="flex flex-wrap items-center justify-between gap-3 py-3">
                <div className="min-w-0">
                  <p className="font-medium break-all">{pool.key}</p>
                  <p className="text-sm text-muted-foreground">Revision {pool.revision}</p>
                </div>
                {scopes.includes("template.publish") && (
                  <Button variant="outline" size="sm" onClick={() => setEditing(pool.key)}>
                    Edit
                  </Button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
      {editing === "new" && (
        <PoolForm
          scopes={scopes}
          onClose={() => setEditing(null)}
          onAccepted={() => void client.invalidateQueries({ queryKey: ["template-pools"] })}
        />
      )}
      {editing && editing !== "new" && detail.data && (
        <PoolForm
          key={editing}
          resource={detail.data}
          scopes={scopes}
          onClose={() => setEditing(null)}
          onAccepted={() => {
            void client.invalidateQueries({ queryKey: ["template-pools"] });
            void client.invalidateQueries({ queryKey: ["template-pool", editing] });
          }}
        />
      )}
    </>
  );
}
