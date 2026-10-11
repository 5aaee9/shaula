import { profileUnavailableReason } from "./profile-choice";
import type { TemplateSummary } from "./types";

export function sortedProfiles(profiles: readonly TemplateSummary[] = []): TemplateSummary[] {
  return [...profiles].sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

export function profileOptionLabel(profile: TemplateSummary): string {
  const reason = profileUnavailableReason(profile);
  const revision = profile.activeRevision ? `r${profile.activeRevision}` : "No Active";
  return `${profile.key} · ${revision} · ${profile.status}${reason ? ` · ${reason}` : ""}`;
}
