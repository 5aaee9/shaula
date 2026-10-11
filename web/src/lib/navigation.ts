import { Boxes, GitPullRequest, KeyRound, Layers3, ListChecks, Network } from "lucide-react";

export const navigation = [
  { url: "/jobs", title: "Jobs", icon: ListChecks },
  { url: "/fleets", title: "Fleets", icon: Boxes },
  { url: "/templates", title: "Templates", icon: Layers3 },
  { url: "/pools", title: "Template pools", icon: Network },
  { url: "/auth", title: "Authentication", icon: KeyRound },
  { url: "/settings/access-tokens", title: "Access tokens", icon: KeyRound },
  { url: "/changes", title: "Changes", icon: GitPullRequest },
];
