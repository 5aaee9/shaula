import { useQuery } from "@tanstack/react-query";
import { api, resourcePath } from "./api";
import { authenticationExpired } from "./authentication";
import type {
  AuthLiveFleet,
  AuthResource,
  FleetResource,
  FleetStatus,
  FleetSummary,
  Session,
  TemplatePoolSummary,
  TemplateSummary,
} from "./types";

export function useSession() {
  return useQuery({
    queryKey: ["session"],
    enabled: !authenticationExpired(),
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
    refetchOnMount: "always",
    refetchInterval: 10_000,
  });
}

export function useTemplatePools(enabled = true) {
  return useQuery({
    queryKey: ["template-pools"],
    queryFn: ({ signal }) => api<{ pools: TemplatePoolSummary[] }>("/template-pools", { signal }),
    enabled,
    refetchOnMount: "always",
    refetchInterval: 10_000,
  });
}

export function useAuthImpact(key: string, enabled: boolean) {
  const query = useQuery({
    queryKey: ["auth-impact", key],
    enabled,
    queryFn: async ({ signal }) => {
      const response = await api<{ liveFleets: AuthLiveFleet[] }>(
        resourcePath("github-auth-profiles", key) + "/impact",
        { signal },
      );
      if (!Array.isArray(response.data.liveFleets))
        throw new Error("Fleet impact response is unavailable");
      return response.data.liveFleets;
    },
    refetchInterval: 5_000,
  });
  return { ...query, ready: !enabled || (query.isSuccess && !query.isFetching) };
}

export function useAuthProfiles(enabled = true) {
  return useQuery({
    queryKey: ["auth-profiles"],
    queryFn: ({ signal }) => api<{ profiles: AuthResource[] }>("/github-auth-profiles", { signal }),
    enabled,
    refetchOnMount: "always",
    refetchInterval: 10_000,
  });
}
