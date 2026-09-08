import type { GitHubTarget, TargetSelector } from "./types";

export type SelectorRow = {
  kind: TargetSelector["kind"];
  owner: string;
  repository: string;
  account_kind: "user" | "organization";
};

export function parseLegacyTargets(value: string): GitHubTarget[] {
  return value
    .split(/[\n,]+/)
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      const parts = entry
        .replace(/^https:\/\/github\.com\//, "")
        .replace(/\/$/, "")
        .split("/");
      if (parts.length > 2 || parts.some((part) => !part))
        throw new Error("Targets must be an organization or owner/repository.");
      return parts.length === 2
        ? { kind: "repository", owner: parts[0], repository: parts[1] }
        : { kind: "organization", owner: parts[0] };
    });
}

export function parseLegacyIdentity(identity?: string | null) {
  const match = identity?.match(/^app\/([^/]+)\/installation\/([1-9][0-9]*)$/);
  return { appId: match?.[1] || "", installationId: match?.[2] || "" };
}

export function selectorFromRow(row: SelectorRow): TargetSelector {
  if (row.kind === "repository")
    return { kind: row.kind, owner: row.owner, repository: row.repository };
  if (row.kind === "account_repositories")
    return { kind: row.kind, account_kind: row.account_kind, owner: row.owner };
  return { kind: row.kind, owner: row.owner };
}

export function rowsFromSelectors(selectors: TargetSelector[]): SelectorRow[] {
  return selectors.map((selector) => ({
    kind: selector.kind,
    owner: selector.owner,
    repository: selector.kind === "repository" ? selector.repository : "",
    account_kind: selector.kind === "account_repositories" ? selector.account_kind : "user",
  }));
}

/** Same typed, case-insensitive set identity as the policy boundary. */
export function selectorKey(selector: TargetSelector): string {
  return JSON.stringify([
    selector.kind,
    selector.kind === "account_repositories" ? selector.account_kind : "",
    selector.owner.toLowerCase(),
    selector.kind === "repository" ? selector.repository.toLowerCase() : "",
  ]);
}

export function selectorAllows(selector: TargetSelector, target: GitHubTarget): boolean {
  if (selector.owner.toLowerCase() !== target.owner.toLowerCase()) return false;
  if (selector.kind === "organization") return target.kind === "organization";
  return (
    target.kind === "repository" &&
    (selector.kind === "account_repositories" ||
      selector.repository.toLowerCase() === target.repository.toLowerCase())
  );
}

export function policyDifference(previous: TargetSelector[], next: TargetSelector[]) {
  const previousKeys = new Set(previous.map(selectorKey));
  const nextKeys = new Set(next.map(selectorKey));
  return {
    added: next.filter((selector) => !previousKeys.has(selectorKey(selector))),
    removed: previous.filter((selector) => !nextKeys.has(selectorKey(selector))),
  };
}
