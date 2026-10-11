import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "@/lib/api";
import type { TemplateRevision } from "@/lib/types";

/** Tracks the inspected revision, defaulting to the profile's desired revision. */
export function useTemplateRevision({
  path,
  profileKey,
  desired,
  enabled,
}: {
  path: string;
  profileKey: string;
  desired: number | undefined;
  enabled: boolean;
}) {
  const [selected, setSelected] = useState("");
  const revision = selected || (desired ? String(desired) : "");
  const query = useQuery({
    queryKey: ["template-revision", profileKey, revision],
    enabled: !!revision && enabled,
    queryFn: ({ signal }) =>
      api<TemplateRevision>(`${path}/revisions/${encodeURIComponent(revision)}`, { signal }),
    refetchInterval: 10000,
  });
  return { revision, setSelected, query };
}
