import type { TemplatePoolMemberSpec } from "./types";

export const initialPoolMember = (index: number): TemplatePoolMemberSpec => ({
  key: `member-${index + 1}`,
  template_profile_ref: "",
  weight: 1,
  template_inputs: {},
});
