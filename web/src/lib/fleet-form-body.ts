import type { FleetInputs } from "./use-fleet-inputs";
import type { ProfileChoices } from "./profile-choice";
import { canChooseProfile } from "./profile-choice";
import type { ForgejoTarget } from "./runner-backend";
import type {
  ForgejoFleetSpec,
  GitHubFleetSpec,
  TemplatePoolMemberSpec,
  TemplatePoolSpec,
} from "./types";

/** How a fleet routes new runners to templates (spec 0037 §4). */
export type Placement = "single" | "pool" | "shared";

const memberKeyPattern = /^[a-z0-9][a-z0-9-]*$/;

export function initialPlacement(spec?: {
  template_pool?: unknown;
  template_pool_ref?: string;
}): Placement {
  if (spec?.template_pool) return "pool";
  return spec?.template_pool_ref ? "shared" : "single";
}

function parseLabels(labels: string) {
  return labels
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);
}

/**
 * The submitted spec carries exactly one template source. Template inputs are
 * spliced in as raw JSON so values keep the exact text the editor produced.
 */
function placementBody(
  base: object,
  placement: Placement,
  editor: Pick<FleetInputs, "reference" | "json">,
  pool: TemplatePoolSpec,
  poolRef: string,
) {
  if (placement === "single") {
    const json = JSON.stringify({ ...base, template_profile_ref: editor.reference });
    return `${json.slice(0, -1)},"template_inputs":${editor.json}}`;
  }
  if (placement === "pool") return JSON.stringify({ ...base, template_pool: pool });
  return JSON.stringify({ ...base, template_pool_ref: poolRef });
}

/** First problem with GitHub pool members, reported member by member. */
function githubPoolProblem(members: TemplatePoolMemberSpec[]) {
  if (members.length < 1 || members.length > 32)
    return "Template pool must contain between 1 and 32 members.";
  const keys = new Set<string>();
  for (const member of members) {
    if (!memberKeyPattern.test(member.key)) return `Invalid member key: ${member.key}`;
    if (keys.has(member.key)) return `Duplicate member key: ${member.key}`;
    keys.add(member.key);
    if (!Number.isInteger(member.weight) || member.weight < 1 || member.weight > 10000)
      return `Weight for ${member.key} must be an integer from 1 to 10000.`;
  }
  return null;
}

/**
 * First reason a GitHub fleet draft cannot be submitted. "selection" problems
 * disable the submit button; the others are only reported once submitted, and
 * "capacity" lives in the collapsed advanced settings.
 */
export function githubFleetProblem({
  placement,
  editor,
  authAllowed,
  templateAllowed,
  poolRef,
  capacity,
  members,
}: {
  placement: Placement;
  editor: Pick<FleetInputs, "canSubmit">;
  authAllowed: boolean;
  templateAllowed: boolean;
  poolRef: string;
  capacity: { min_runners: number; max_runners: number };
  members: TemplatePoolMemberSpec[];
}): { kind: "selection" | "capacity" | "pool"; message: string } | null {
  if (placement === "single" && !editor.canSubmit)
    return {
      kind: "selection",
      message: "Load and review the template input contract before submitting.",
    };
  if (!authAllowed || (placement === "single" && !templateAllowed))
    return {
      kind: "selection",
      message: "Choose available profiles from a successfully loaded list before submitting.",
    };
  if (placement === "shared" && !poolRef.trim())
    return { kind: "selection", message: "Choose a template pool for this fleet." };
  if (capacity.min_runners > capacity.max_runners)
    return { kind: "capacity", message: "Minimum runners cannot exceed maximum runners." };
  const poolProblem = placement === "pool" ? githubPoolProblem(members) : null;
  return poolProblem ? { kind: "pool", message: poolProblem } : null;
}

export function githubFleetBody({
  spec,
  fleetKey,
  editing,
  labels,
  draft,
  editor,
}: {
  spec: GitHubFleetSpec;
  fleetKey: string;
  editing: boolean;
  labels: string;
  draft: { placement: Placement; pool: TemplatePoolSpec; poolRef: string };
  editor: Pick<FleetInputs, "reference" | "json">;
}) {
  // Omitted template sources stay absent rather than empty.
  const base = {
    ...spec,
    github: {
      ...spec.github,
      scale_set_name: editing ? spec.github.scale_set_name : spec.github.scale_set_name || fleetKey,
      labels: parseLabels(labels),
    },
    template_profile_ref: undefined,
    template_pool: undefined,
    template_pool_ref: undefined,
    template_inputs: undefined,
  };
  const { members, failure_policy } = draft.pool;
  return placementBody(
    base,
    draft.placement,
    editor,
    { members, failure_policy },
    draft.poolRef.trim(),
  );
}

function invalidForgejoMember(member: TemplatePoolMemberSpec) {
  const cap = member.max_runners;
  return (
    !memberKeyPattern.test(member.key) ||
    !Number.isInteger(member.weight) ||
    member.weight < 1 ||
    member.weight > 10000 ||
    (cap !== undefined && (!Number.isInteger(cap) || cap < 0))
  );
}

function forgejoPoolValid(pool: TemplatePoolSpec) {
  const keys = new Set(pool.members.map((member) => member.key));
  return (
    keys.size === pool.members.length &&
    pool.members.length <= 32 &&
    !pool.members.some(invalidForgejoMember)
  );
}

/** Validates a Forgejo fleet draft and serializes it, throwing the first user-facing problem. */
export function forgejoFleetBody({
  fleetKey,
  target,
  allowed,
  auth,
  prefix,
  labels,
  capacity,
  draft,
  editor,
}: {
  fleetKey: string;
  target?: ForgejoTarget;
  allowed: boolean;
  auth: string;
  prefix: string;
  labels: string;
  capacity: ForgejoFleetSpec["capacity"];
  draft: { placement: Placement; pool: TemplatePoolSpec; poolRef: string };
  editor: Pick<FleetInputs, "reference" | "json">;
}) {
  if (!target || !allowed)
    throw new Error(
      "Choose an available Forgejo authentication profile and compatible template placement before submitting.",
    );
  if (draft.placement === "pool" && !forgejoPoolValid(draft.pool))
    throw new Error(
      "Use 1–32 uniquely named members, integer weights 1–10000 and non-negative runner caps.",
    );
  if (capacity.min_runners > capacity.max_runners)
    throw new Error("Minimum runners cannot exceed maximum runners.");
  const names = parseLabels(labels);
  if (!names.length)
    throw new Error(
      "Declare at least one label with an explicit backend target, for example linux:host.",
    );
  const spec: ForgejoFleetSpec = {
    kind: "forgejo",
    forgejo: {
      instance_url: target.instance_url,
      scope: target.scope,
      auth_profile_ref: auth,
      runner_name_prefix: prefix || `shaula-${fleetKey}-`,
      labels: names,
    },
    capacity,
  };
  return placementBody(spec, draft.placement, editor, draft.pool, draft.poolRef);
}

/** Whether the chosen Forgejo placement can be submitted as-is. */
export function forgejoPlacementAllowed({
  placement,
  editor,
  templateAllowed,
  editing,
  original,
  pool,
  poolRef,
  pools,
  templates,
}: {
  placement: Placement;
  editor: Pick<FleetInputs, "canRead" | "canSubmit">;
  templateAllowed: boolean;
  editing: boolean;
  original?: { template_pool?: TemplatePoolSpec; template_pool_ref?: string };
  pool: TemplatePoolSpec;
  poolRef: string;
  pools: { isSuccess: boolean; data?: { data: { pools: { key: string }[] } } };
  templates: ProfileChoices;
}) {
  if (placement === "single") return templateAllowed && editor.canSubmit;
  if (placement === "shared")
    return (
      (editing && poolRef === original?.template_pool_ref) ||
      (editor.canRead &&
        pools.isSuccess &&
        !!pools.data?.data.pools.some((summary) => summary.key === poolRef))
    );
  return (
    (editing && JSON.stringify(pool) === JSON.stringify(original?.template_pool)) ||
    (pool.members.length > 0 &&
      pool.members.every((member) =>
        canChooseProfile(templates, member.template_profile_ref, editor.canRead),
      ))
  );
}
