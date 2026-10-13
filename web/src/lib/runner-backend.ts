import { targetName, type AuthResource, type FleetSpec } from "./types";
import type { ProfileChoices } from "./profile-choice";

export type RunnerBackend = "github" | "forgejo" | "gitlab";
export type ForgejoScope =
  | { kind: "instance" }
  | { kind: "user" }
  | { kind: "organization"; name: string }
  | { kind: "repository"; owner: string; name: string };
export interface ForgejoTarget {
  instance_url: string;
  scope: ForgejoScope;
}
/** A full group path, or a namespace/project path that always contains '/'. */
export type GitlabScope = { kind: "group"; path: string } | { kind: "project"; path: string };
export interface GitlabTarget {
  instance_url: string;
  scope: GitlabScope;
}

/** Credential kind per token backend; every other kind belongs to GitHub. */
const tokenKinds: Record<Exclude<RunnerBackend, "github">, string> = {
  forgejo: "forgejo_token",
  gitlab: "gitlab_token",
};

export function backendLabel(backend: RunnerBackend | undefined): string {
  return backend === "forgejo" ? "Forgejo" : backend === "gitlab" ? "GitLab" : "GitHub Actions";
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

export function gitlabTargetName(target: GitlabTarget): string {
  return `${target.instance_url} / ${target.scope.path} (${target.scope.kind})`;
}

export function fleetTargetName(spec: FleetSpec): string {
  if (spec.kind === "forgejo") return forgejoTargetName(spec.forgejo);
  if (spec.kind === "gitlab") return gitlabTargetName(spec.gitlab);
  return targetName(spec.github.target);
}

export function authKindLabel(kind: string | null): string {
  return kind === "forgejo_token"
    ? "Forgejo token"
    : kind === "gitlab_token"
      ? "GitLab token"
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
  const tokens: (string | null)[] = Object.values(tokenKinds);
  return {
    ...query,
    data: query.data && {
      ...query.data,
      data: {
        profiles: query.data.data.profiles.filter((p) => {
          const profile = p as AuthResource;
          if (backend === "github") return !tokens.includes(profile.kind);
          return profile.kind === tokenKinds[backend];
        }),
      },
    },
  };
}
