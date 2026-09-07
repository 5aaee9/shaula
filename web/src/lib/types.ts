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
  template_profile_ref: string | { key: string; revision: number };
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
  platform: string;
  state: string;
}
export interface AuthResource extends TemplateSummary {
  kind: string | null;
  identity: string | null;
  credential_present: boolean;
  target_allowlist: string[];
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
