import type { TemplatePoolMemberSpec } from "./types";

const MEMBER_KEY_PATTERN = /^[a-z0-9][a-z0-9-]*$/;

export const initialPoolMember = (index: number): TemplatePoolMemberSpec => ({
  key: `member-${index + 1}`,
  template_profile_ref: "",
  weight: 1,
  template_inputs: {},
});

/** First validation problem for a weighted template pool member list, or null. */
export function poolMembersProblem(
  members: TemplatePoolMemberSpec[],
  options: { requireTemplate?: boolean } = {},
): string | null {
  if (members.length < 1 || members.length > 32)
    return "Template pool must contain between 1 and 32 members.";
  const keys = new Set<string>();
  for (const member of members) {
    if (!MEMBER_KEY_PATTERN.test(member.key)) return `Invalid member key: ${member.key}`;
    if (keys.has(member.key)) return `Duplicate member key: ${member.key}`;
    keys.add(member.key);
    if (!Number.isInteger(member.weight) || member.weight < 1 || member.weight > 10000)
      return `Weight for ${member.key} must be an integer from 1 to 10000.`;
    if (
      member.max_runners !== undefined &&
      (!Number.isInteger(member.max_runners) || member.max_runners < 0)
    )
      return `Maximum runners for ${member.key} must be a non-negative integer.`;
    if (options.requireTemplate && !member.template_profile_ref.trim())
      return `Choose a template for member ${member.key}.`;
  }
  return null;
}
