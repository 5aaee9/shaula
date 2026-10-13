import type { AuthFormMode } from "@/components/auth-form";

export function authManageCopy(mode: AuthFormMode, profileKey: string) {
  switch (mode) {
    case "create":
      return {
        title: "Create authentication profile",
        description:
          "Connect a GitHub App or a scoped Forgejo or GitLab token for runner management.",
      };
    case "policy":
      return {
        title: `Edit target policy for ${profileKey}`,
        description:
          "Choose allowed targets using the credential from the current Active revision.",
      };
    default:
      return {
        title: `Rotate credential for ${profileKey}`,
        description: "Replace the credential while retaining the current Active target.",
      };
  }
}
