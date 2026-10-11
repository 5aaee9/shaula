import { useQueries } from "@tanstack/react-query";
import { api, resourcePath } from "@/lib/api";
import type { FleetResource, FleetStatus, FleetSummary } from "@/lib/types";
import { fleetTargetName } from "@/lib/runner-backend";

export type FleetRow = FleetSummary & {
  resource?: FleetResource;
  status?: FleetStatus;
  error: Error | null;
};

/** Joins each fleet summary with its polled resource and live status. */
export function useFleetRows(summaries: FleetSummary[]) {
  const resources = useQueries({
    queries: summaries.map(({ key }) => ({
      queryKey: ["fleet", key],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        api<FleetResource>(resourcePath("fleets", key), { signal }),
      refetchInterval: 10000,
    })),
  });
  const statuses = useQueries({
    queries: summaries.map(({ key }) => ({
      queryKey: ["fleet-status", key],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        api<FleetStatus>(`${resourcePath("fleets", key)}/status`, { signal }),
      refetchInterval: 5000,
    })),
  });
  const rows: FleetRow[] = summaries.map((summary, i) => ({
    ...summary,
    resource: resources[i].data?.data,
    status: statuses[i].data?.data,
    error: resources[i].error || statuses[i].error,
  }));
  const complete = statuses.every((query) => query.isSuccess);
  // Totals are withheld until every status has loaded so partial sums never show.
  const total = (key: keyof FleetStatus["capacity"]) =>
    complete ? rows.reduce((sum, row) => sum + (row.status?.capacity[key] || 0), 0) : null;
  return { rows, total };
}

export function fleetRowMatches(row: FleetRow, search: string, filter: string) {
  const text = `${row.key} ${row.resource ? fleetTargetName(row.resource.spec) : ""}`;
  if (!text.toLowerCase().includes(search.toLowerCase())) return false;
  if (filter === "all") return true;
  if (filter === "attention")
    return /block|error|fail|reject/i.test(row.status?.phase || "") || !!row.error;
  return row.status?.phase === filter;
}
