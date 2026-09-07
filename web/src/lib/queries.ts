import { useQuery } from "@tanstack/react-query";
import { api, resourcePath } from "./api";
import type { FleetResource, FleetStatus, FleetSummary, Session, TemplateSummary } from "./types";

export function useSession() {
  return useQuery({
    queryKey: ["session"],
    queryFn: ({ signal }) => api<Session>("/session", { signal }),
    retry: false,
    refetchInterval: 30_000,
  });
}
export function useFleets(enabled: boolean) {
  return useQuery({
    queryKey: ["fleets"],
    queryFn: ({ signal }) => api<{ fleets: FleetSummary[] }>("/fleets", { signal }),
    enabled,
    refetchInterval: 10_000,
  });
}
export function useFleet(key: string) {
  return useQuery({
    queryKey: ["fleet", key],
    queryFn: ({ signal }) => api<FleetResource>(resourcePath("fleets", key), { signal }),
    refetchInterval: 10_000,
  });
}
export function useFleetStatus(key: string) {
  return useQuery({
    queryKey: ["fleet-status", key],
    queryFn: ({ signal }) => api<FleetStatus>(`${resourcePath("fleets", key)}/status`, { signal }),
    refetchInterval: 5_000,
  });
}
export function useTemplates(enabled = true) {
  return useQuery({
    queryKey: ["templates"],
    queryFn: ({ signal }) => api<{ profiles: TemplateSummary[] }>("/template-profiles", { signal }),
    enabled,
    refetchInterval: 10_000,
  });
}
