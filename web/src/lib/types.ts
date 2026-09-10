export interface Session {
  name: string;
  scopes: string[];
}
export interface FleetSummary {
  key: string;
  revision: number;
  incarnation: string;
}
export type GitHubTarget =
  | { kind: "organization"; owner: string }
  | { kind: "repository"; owner: string; repository: string };
export interface FleetSpec {
  github: {
    target: GitHubTarget;
    auth_profile_ref: string;
    scale_set_name: string;
    runner_group: string;
    labels: string[];
  };
  capacity: { min_runners: number; max_runners: number };
  template_profile_ref: string;
  template_inputs: Record<string, unknown>;
}
export interface FleetResource {
  key: string;
  spec: FleetSpec;
  metadata: { incarnation: string; revision: number };
  resolved: {
    template: {
      key: string;
      revision: number;
      artifactDigest: string;
      attestationId: string;
    } | null;
    authDesired: { profileKey: string; revision: number };
  };
}
export interface FleetStatus {
  fleetKey: string;
  desiredRevision: number;
  observedRevision: number;
  phase: string;
  conditions: { type: string; status: boolean; reason: string | null }[];
  capacity: { assignedDemand: number; target: number; effective: number; occupancy: number };
  lastError: string | null;
  githubAuth?: {
    desired: { profileKey: string; revision: number };
    observed: { profileKey: string; revision: number } | null;
    handoffState: string;
    /** Exact Resolved Auth Context rollout (spec 0011 §6). */
    context?: {
      desired: { profileKey: string; revision: number } | null;
      observed: { profileKey: string; revision: number } | null;
      state: string;
      reason: string | null;
      desiredRoute?: AuthRoute | null;
      observedRoute?: AuthRoute | null;
    };
  };
}
export interface AuthRoute {
  profileKey: string;
  revision: number;
  githubHost: string;
  appId: string;
  account: { login: string; id: number; kind: "user" | "organization" };
  installationId: number;
  target: GitHubTarget;
  organizationId: number | null;
  repositoryId: number | null;
  repositoryOwnerId: number | null;
}
export interface TemplateSummary {
  key: string;
  incarnation: string;
  desiredRevision: number;
  activeRevision: number | null;
  status: string;
}
export interface TemplateResource extends TemplateSummary {
  platform: string | null;
  bindingsContract: string | null;
  bindings_present: boolean;
}
export interface TemplateRevision {
  revision: number;
  artifactDigest: string;
  engineRef: string;
  sourceKey?: string | null;
  platform: string;
  state: string;
  reason: string | null;
}
export interface AuthResource extends TemplateSummary {
  kind: string | null;
  credential_present: boolean;
  /** Policy and bindings are attributed to their owning revision. */
  schema_version?: number;
  app_id?: string;
  active?: AuthRevisionState;
  desired?: AuthRevisionState;
  /** Live fleet targets desiring this profile (spec 0011 §6). */
  liveFleets?: AuthLiveFleet[];
}
/** One live fleet target desiring the profile: the impact surface. */
export interface AuthLiveFleet {
  fleetKey: string;
  phase: string;
  target: GitHubTarget | null;
}
export type TargetSelector =
  | { kind: "organization"; owner: string }
  | { kind: "repository"; owner: string; repository: string }
  | { kind: "account_repositories"; account_kind: "user" | "organization"; owner: string };
export interface AccountBinding {
  account_id: number;
  account_kind: "user" | "organization";
  login: string;
  installation_id: number;
  repository_selection: "all" | "selected";
  validated_at_ms: number;
  /** Bounded per-account health (spec 0011 §6). */
  health: "Validated" | "Unknown" | "Blocked" | "Degraded" | "Healthy";
  checked_at_ms: number | null;
  valid_until_ms: number | null;
  affected_fleets: string[];
  reason: string | null;
}
export interface AuthRevisionState {
  revision: number | null;
  state: string;
  reason: string | null;
  schema_version: number;
  app_id?: string | null;
  target_policy?: TargetSelector[];
  bindings?: AccountBinding[];
}
export function selectorLabel(selector: TargetSelector): string {
  if (selector.kind === "repository")
    return `${selector.owner}/${selector.repository} (repo runners)`;
  if (selector.kind === "account_repositories")
    return `${selector.owner} (${selector.account_kind} repositories)`;
  return `${selector.owner} (org runners)`;
}
export interface Accepted {
  changeId: string;
  state: string;
  revision: number;
  noOp?: boolean;
}
export interface Change {
  id: string;
  resourceKey: string;
  kind: string;
  state: string;
  revision: number;
  reason: string | null;
}
export type ChangeRef = { id: string; resource: string; type: "fleet" | "profile" };
export function targetName(target: GitHubTarget) {
  return target.kind === "repository" ? `${target.owner}/${target.repository}` : target.owner;
}
