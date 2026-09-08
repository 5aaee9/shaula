import type { Resource } from "./api";
import type { FleetSpec, TemplateSummary } from "./types";

export interface ProfileChoices {
  data?: Resource<{ profiles: TemplateSummary[] }>;
  error: unknown;
  isPending: boolean;
  isFetching: boolean;
  isSuccess: boolean;
  refetch: () => unknown;
}

export function profileUnavailableReason(profile: TemplateSummary): string | undefined {
  if (profile.status === "Retiring" || profile.status === "Retired")
    return "Retirement prevents new references";
  if (!profile.activeRevision) return "No Active revision";
}

export function canChooseProfile(query: ProfileChoices, key: string, canRead: boolean) {
  if (!canRead || !query.isSuccess) return false;
  const profile = query.data?.data.profiles.find((profile) => profile.key === key);
  return !!profile && !profileUnavailableReason(profile);
}

export function sameTemplateReference(
  left: FleetSpec["template_profile_ref"],
  right: FleetSpec["template_profile_ref"] | undefined,
) {
  if (typeof left === "string" || typeof right === "string") return left === right;
  return !!right && left.key === right.key && left.revision === right.revision;
}
