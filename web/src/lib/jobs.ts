import { useRef, useSyncExternalStore } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api";

export interface Job {
  id: string;
  fleet_key: string;
  protocol_job_id: string;
  owner_name: string | null;
  repository_name: string | null;
  job_display_name: string | null;
  job_workflow_ref: string | null;
  workflow_run_id: number | null;
  workflow_run_attempt: number | null;
  observed_status: string;
  reported_result: string | null;
  github_conclusion: string | null;
  github_run_url: string | null;
  association_status: string;
  freshness: string;
  created_at: number;
  updated_at: number;
}
export interface Generation {
  id: string;
  fleet_key: string;
  runner_name: string;
  generation_name: string;
  github_runner_id: number | null;
  state: string;
  subphase: string | null;
  association_status: string;
  created_at: number;
  updated_at: number;
}
export interface Observation {
  id: string;
  kind: string;
  runner_request_id: number;
  runner_id: number | null;
  runner_name: string | null;
  reported_result: string | null;
  observed_at: number;
  association_status: string;
}
export interface JobDetail extends Job {
  observations: Observation[];
  observations_truncated: boolean;
  generations: Generation[];
}
export interface GenerationDetail extends Generation {
  jobs: Job[];
}
export interface Page<T> {
  items: T[];
  next_cursor: string | null;
}
export interface Invocation {
  id: string;
  generation_id: string;
  operation: string;
  ordinal: number;
  started_at: number;
  ended_at: number | null;
  execution_outcome: string;
  capture_status: string;
  retained_bytes: number;
  lost_bytes: number;
  commands: { phase: string; termination: string; exit_code: number | null }[];
}
export interface LogPage {
  invocation_id: string;
  content_version: string;
  capture_status: string;
  entries: {
    command_ordinal: number;
    phase: string;
    stream: string;
    sequence: number;
    text: string;
    observed_at: number;
  }[];
  next_cursor: string | null;
  has_gap: boolean;
  lost_bytes: number;
}
export interface InvocationsPage {
  items: Invocation[];
  next_cursor: string | null;
  latest_create: Invocation | null;
  latest_destroy: Invocation | null;
}
function subscribeVisibility(listener: () => void) {
  document.addEventListener("visibilitychange", listener);
  return () => document.removeEventListener("visibilitychange", listener);
}
export function useHistory<T>(path: string, enabled: boolean) {
  const backoff = useRef({ path, error: 0, success: 0, count: 0 });
  const visible = useSyncExternalStore(
    subscribeVisibility,
    () => document.visibilityState === "visible",
  );
  return useQuery({
    queryKey: ["history", path],
    queryFn: async ({ signal }) => (await api<T>(path, { signal })).data,
    enabled,
    retry: false,
    gcTime: 0,
    refetchInterval: (query) => {
      const state = query.state;
      if (backoff.current.path !== path) backoff.current = { path, error: 0, success: 0, count: 0 };
      const attempt = backoff.current;
      if (state.dataUpdatedAt > attempt.success) {
        attempt.success = state.dataUpdatedAt;
        attempt.count = 0;
      }
      if (state.errorUpdatedAt > attempt.error) {
        attempt.error = state.errorUpdatedAt;
        attempt.count += 1;
      }
      if (
        !visible ||
        (state.error instanceof ApiError && [401, 403, 404].includes(state.error.status))
      )
        return false;
      return Math.min(60_000, 5_000 * 2 ** Math.min(attempt.count, 4));
    },
    refetchIntervalInBackground: false,
  });
}
export function historyTime(value: number | null) {
  return value === null ? "Unknown" : new Date(value).toLocaleString();
}
export function operationName(value: string) {
  return value.toLowerCase() === "create" ? "Apply" : "Destroy";
}
