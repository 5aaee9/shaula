import { useQuery } from "@tanstack/react-query";
import { ExternalLink } from "lucide-react";
import { Link } from "react-router-dom";
import { api } from "@/lib/api";
import type { Change, ChangeRef } from "@/lib/types";
import { ErrorNotice, StatusBadge } from "./status";

export function useChange(ref: ChangeRef | null) {
  return useQuery({
    queryKey: ["change", ref?.type, ref?.id],
    enabled: !!ref,
    queryFn: async ({ signal }) => {
      const { data } = await api<Change & { resource_key?: string }>(
        `/${ref!.type}-changes/${encodeURIComponent(ref!.id)}`,
        { signal },
      );
      return { ...data, resourceKey: data.resourceKey || data.resource_key || ref!.resource };
    },
    refetchInterval: (query) =>
      /^(Converged|Rejected|Superseded|Failed)$/i.test(query.state.data?.state || "")
        ? false
        : 3_000,
  });
}
export function ChangeNotice({ change }: { change: ChangeRef | null }) {
  const query = useChange(change);
  if (!change) return null;
  return (
    <div className="change-notice" role="status">
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-medium">Change for {change.resource}</span>
        <StatusBadge value={query.data?.state || "Accepted"} />
        <Link
          className="inline-flex items-center gap-1 text-xs underline"
          to={`/changes?type=${change.type}&id=${encodeURIComponent(change.id)}`}
        >
          View change <ExternalLink className="size-3" />
        </Link>
      </div>
      {query.data?.reason && <p className="mt-2 text-sm">{query.data.reason}</p>}
      {query.error && <ErrorNotice error={query.error} />}
    </div>
  );
}
