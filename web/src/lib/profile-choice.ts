import type { Resource } from "./api";
import type { TemplateSummary } from "./types";

export interface ProfileChoices {
  data?: Resource<{ profiles: TemplateSummary[] }>;
  error: unknown;
  isPending: boolean;
  isFetching: boolean;
  isSuccess: boolean;
  refetch: () => unknown;
}

export function profileUnavailableReason(profile: TemplateSummary): string | undefined {
  if (profile.status === "Unsupported") return "Unsupported authentication schema";
  if (profile.status === "Retiring" || profile.status === "Retired")
    return "Retirement prevents new references";
  if (!profile.activeRevision) return "No Active revision";
}

export function canChooseProfile(query: ProfileChoices, key: string, canRead: boolean) {
  if (!canRead || !query.isSuccess) return false;
  const profile = query.data?.data.profiles.find((profile) => profile.key === key);
  return !!profile && !profileUnavailableReason(profile);
}
