import { targetName, type AuthResource, type FleetSpec } from "./types";
import type { ProfileChoices } from "./profile-choice";

export type RunnerBackend = "github" | "forgejo";
export type ForgejoScope =
  | { kind: "instance" }
  | { kind: "user" }
  | { kind: "organization"; name: string }
  | { kind: "repository"; owner: string; name: string };
export interface ForgejoTarget {
  instance_url: string;
  scope: ForgejoScope;
}

export function forgejoTargetName(target: ForgejoTarget): string {
  const scope = target.scope;
  const name =
    scope.kind === "repository"
      ? `${scope.owner}/${scope.name}`
      : scope.kind === "organization"
        ? scope.name
        : scope.kind === "user"
          ? "Token owner's runners"
          : "Instance runners";
  return `${target.instance_url} / ${name}`;
}

export function fleetTargetName(spec: FleetSpec): string {
  return spec.kind === "forgejo" ? forgejoTargetName(spec.forgejo) : targetName(spec.github.target);
}

export function authKindLabel(kind: string | null): string {
  return kind === "forgejo_token"
    ? "Forgejo token"
    : kind === "github_app"
      ? "GitHub App"
      : kind === "pat"
        ? "Personal access token"
        : "--";
}

/** Preserve query health and refresh semantics while showing only compatible choices. */
export function templateChoices(query: ProfileChoices, backend: RunnerBackend): ProfileChoices {
  return {
    ...query,
    data: query.data && {
      ...query.data,
      data: {
        profiles: query.data.data.profiles.filter(
          (p) => (p.runnerBackend === undefined ? "github" : p.runnerBackend) === backend,
        ),
      },
    },
  };
}

export function authChoices(query: ProfileChoices, backend: RunnerBackend): ProfileChoices {
  return {
    ...query,
    data: query.data && {
      ...query.data,
      data: {
        profiles: query.data.data.profiles.filter((p) => {
          const profile = p as AuthResource;
          if (backend === "github") return profile.kind !== "forgejo_token";
          return profile.kind === "forgejo_token";
        }),
      },
    },
  };
}
